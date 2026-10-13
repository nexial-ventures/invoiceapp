# Per-space PDF design: upload, draft, activation, versions (4f)

Completes #10. The design model of [pdf.md](pdf.md) stays: the embedded default (`design/` in the repo) overridden
file by file by the space's stored **live** files under `spaces/{id}/design/`. 4f adds editing from the UI with a
**draft** that is validated and previewed before it becomes live, and a **history of activations**. Archived PDFs
are never re-rendered (1d). All routes are on a space host, **admin+** (Settings); every key below is relative to
`spaces/{id}/`.

## Storage layout
- `design/…` — live files (unchanged; also written by `invoiceapp design push --space`).
- `design-draft/…` — the draft: uploaded files that will override live ones on activation.
- `design-versions/{versionId}/…` — a full copy of the live set right after an activation / revert.
- Table `design_drafts` (space_id PK, deleted paths `text[]`, revision, updated_at, updated_by) — draft deletions (a
  live file to remove on activation); `revision` grows with every draft change; the row is also the space's design
  lock. Table `design_versions` (id uuid, space_id, number per space 1.., kind `activation` |
  `revert` | `default`, created_at, created_by, files `jsonb` [{ path, size, sha256 }]). The newest row is the live
  state.

## Files and limits
- Allowed extensions: `.typ`, `.toml`, `.ttf`, `.otf`, `.woff2`, `.svg`, `.png`, `.jpg`, `.jpeg`. Path: lowercase
  `[a-z0-9._-]` segments, at most 2 levels (`logo.svg`, `fonts/inter.ttf`), no `..`, not `qr.svg` (reserved, 1d).
  A path equal to an **existing** default, live or draft file may be uploaded whatever its name (replacing
  `fonts/Inter-Regular.ttf`, `fonts/OFL.txt`, CLI-pushed files). A path that clashes with an existing one as file vs
  directory (`logo.svg/x.png` while `logo.svg` exists, `img.png` while `img.png/a.png` exists) → `path: invalid`.
- File ≤ 10 MB (as today). **Quota per space: 50 MB** over live + draft + kept versions (counted by stored object
  size). A file over 10 MB → 422 `{"code":"validation","fields":{"file":"too_large"}}`; over the quota → 422
  `{"code":"quota_exceeded","detail":"<used> / 50 MB"}`. Bad path / extension → 422 `path: invalid`.
- **Space quota** ([spaces.md](spaces.md#limits)): the same design bytes also count toward the space's storage quota
  (100 MB by default, shared with originals and the PDF archive). A draft upload must fit **both**: the 50 MB design
  quota is checked first (`detail` `<used> / 50 MB`), then the space quota (`detail` `<space used> / <space limit>
  MB`, the space usage = documents from the database + the design `used` just listed). Every design listing and
  change records the design bytes for the space usage (`space_storage_usage`; an upload, activation and restore in
  their own transaction, under the space's quota lock — [spaces.md](spaces.md#limits)).
- `invoice.typ` must be UTF-8 (1d rule).
- **Lock**: every draft change, activation, restore and reset takes the space's lock without waiting; held by another
  change → 409 `{"code":"design_busy"}`, nothing changed (the UI retries or tells the user to try again).

## Routes
- `GET /api/pdf/design` (extends 1d) → `{ storage, files: [ { path, source: "custom"|"default", size } ],
  draft: { files: [ { path, size, change: "added"|"replaced" } ], deleted: [path], updatedAt, updatedBy } | null,
  quota: { used, limit } }`.
- `GET /api/pdf/design/file?path=…&source=live|draft|default` → the file (`Content-Disposition: attachment`) —
  download a current, draft or default file as a starting point. Unknown → 404.
- `PUT /api/pdf/design/draft/file?path=…` (raw body, `Content-Type` any) → 200 the draft listing. Validates path,
  extension, size, quota; overwrites a draft file of the same path; removes the path from draft deletions.
- `DELETE /api/pdf/design/draft/file?path=…` → 200: removes a draft file if one exists; otherwise, when the path is a
  live custom file, records a **draft deletion** (the default file shows through after activation). A default-only
  path → 404.
- `DELETE /api/pdf/design/draft` → 204 discard the whole draft.
- `GET /api/pdf/design/draft/preview?locale=cs|en` → `200 application/pdf`: the 1d sample invoice rendered with
  **embedded ∪ live ∪ draft (minus deletions)**; render errors as 1d (`502 pdf_render_failed` with `detail`). A
  preview under the per-space preview limits ([pdf.md, Render limits](pdf.md#render-limits)): 429 `rate_limited`.
- `POST /api/pdf/design/activate` → `200 { version }`: (1) renders the sample with the draft set in **both**
  locales, **without** holding the lock — a template failure → 422 `{"code":"design_invalid","detail":"<locale>:
  <mdcast message>"}` (the message is mdcast's sanitised `render failed (incident <id>)`; the typst diagnostics are in
  mdcast's logs) and nothing changes; then under the lock: (2) the draft must still be the one rendered (same
  revision, deletions and stored draft files), else 409 `{"code":"draft_changed"}` and nothing changes; (3) writes
  the new live set: deletes the draft deletions, copies draft files to `design/` (server-side copies); (4) copies the
  resulting live set to `design-versions/{id}/` and inserts the version row; (5) clears the deletions and commits;
  the draft files are deleted after the commit. Empty draft → 422 `{"code":"draft_empty"}`.
- `GET /api/pdf/design/versions` → `[ { id, number, kind, createdAt, createdBy: { email, displayName }, files,
  current: boolean } ]`, newest first.
- `POST /api/pdf/design/versions/{id}/restore` → `200 { version }`: a version whose stored files are missing or
  differ in size from its row → 409 `{"code":"version_incomplete"}` (checked before the renders and again under the
  lock); validates that set like activation (both locales, outside the lock), then replaces live with it (live files not in the version are deleted) and records a `revert` version.
  The draft is untouched.
- `POST /api/pdf/design/reset` → `200 { version }`: deletes every live custom file (the embedded default applies),
  records a `default` version (no files). No validation needed.
- Keeping: the **10 newest versions**; older rows are deleted with each new version, their
  `design-versions/{id}/` objects **after the commit** (a rolled-back transaction never leaves rows without files).
- Failure handling: a storage failure in step (2)/(3) → 503 `storage_unavailable`; the version row is only written
  after the files, so a failed activation leaves no version claiming a set that is not live; the draft is kept so
  the user can retry. Restoring a version is the recovery path for a half-applied live set.
- Rate limit: activation / restore 10 per hour per space (each costs two mdcast renders); 429 `rate_limited`.

## Interaction with existing behaviour
- Renders (issue, preview) keep reading **live** only. The design cache (1d, per space since 4a) — the downloaded
  files and the live listing (reused for 30 s) — is invalidated by the activation / restore / reset (also when its
  transaction fails) or the deletion of that space, and bounded by `INVOICEAPP__DESIGN_CACHE_MB` (LRU).
- `invoiceapp design push|pull|ls|rm --space` keep working on live directly (operator tool, no versions); the next
  activation snapshots whatever is live. A running server picks a push up once its cached listing expires (≤ 30 s); an `rm`
  of a listed file makes the next render list again. Cache invalidation is in-process — the app assumes a single
  instance; any other instance would see an activation / restore / reset only after ≤ 30 s.
- Space delete removes the whole `spaces/{id}/` prefix incl. drafts and versions (4a).

## UI (Settings → Design, admin+)
- Live files table (as today) with download; "Výchozí soubory" downloadable as a starting point.
- Draft panel: upload (file picker; the target path editable, default = the file name; drag & drop optional),
  replace, delete (marks a live file for deletion), list of pending changes, "Náhled konceptu" (cs / en), "Aktivovat"
  (shows `design_invalid` detail on failure), "Zahodit koncept".
- History: versions with number, kind, author, time, file count, "Obnovit" (confirm) and "Vrátit na výchozí design"
  (confirm). Quota bar (used / 50 MB).
- Members below admin don't see the Design tab (as today).

## Tests
- Integration (mdcast mock that can be told to fail on a marker in `invoice.typ`): upload validation (path, type,
  size, quota incl. versions), draft add / replace / delete live / discard, draft preview uses the draft, activation
  success (live replaced, version recorded, draft cleared, cache invalidated → the next preview uses it) and failure
  (nothing changes, `design_invalid` detail), restore and reset, keeping 10 versions (older objects deleted),
  admin-only, space isolation of design keys, rate limit.
- Frontend: draft panel actions, error display, history restore / reset confirms, quota bar.

## Clarifications (as implemented)
- **Roles**: every new route is admin+ (`Manage`). `GET /api/pdf/design` and `GET /api/pdf/preview` stay readable by
  every role as in 1d / spaces.md (the listing now also carries `draft` and `quota`); the UI shows the tab to admins
  only.
- **Lock**: the `design_drafts` row of the space, created by its own statement on first use, then
  `SELECT … FOR UPDATE NOWAIT` in the transaction of the change (Postgres `55P03` → 409 `design_busy`). The locked
  part does only storage copies / deletes and DB writes — never a render — and the transaction is rolled back
  explicitly on any error. `draft` is `null` whenever there are no draft files and no deletions.
- **Shapes**: `draft.updatedBy` and a version's `createdBy` are both `{ email, displayName }`, `null` when the account
  is gone (or nothing was recorded yet); `updatedAt` likewise nullable. `PUT` / `DELETE …/draft/file` answer 200 with
  the **whole** `GET /api/pdf/design` listing; activate / restore / reset answer `{ version }` = the new version as
  listed by `GET …/versions` (`current: true`). The frontend ignores mutation bodies and re-GETs the listing.
- **Draft changes**: `change` is `replaced` when the path exists in the current effective set (custom or default),
  else `added`. A draft deletion is undone only by discarding the draft or uploading that path again (the upload
  removes it from `deleted`); a second `DELETE` of an already deleted live path is a 200 no-op. Deleting a path that
  is neither a draft file nor a live custom file (incl. default-only paths) → 404.
- **Paths**: the strict rule applies to new paths; an existing default / live / draft path is accepted as is (it
  must still be a safe relative path — no empty, `.`/`..`, hidden or control-character segment). The clash rule
  compares against default, live and draft paths. Downloads and draft deletions accept any safe path (otherwise
  404). A non-UTF-8 `invoice.typ` upload → 422 `file: invalid`.
- **Upload order**: the path is checked (against a fresh listing) before the body is read; a declared
  `Content-Length` over 10 MB is refused without reading, otherwise the body is read with a 10 MB cap (anything above
  → `file: too_large`; exactly 10 MB is accepted). Path and quota are checked again under the lock.
- **Quota**: `used` = the stored size of every object under `design/`, `design-draft/` and `design-versions/`
  (hidden ones included; the three listings run concurrently and serve the whole request). An upload is refused
  when `used − (draft file it replaces) + new size > 50 MB`; `detail` = the current `used` in MiB, one decimal
  (`"46.0 / 50 MB"`). Activation, restore and reset are not quota-checked (recovery must stay possible; keeping 10
  versions bounds the growth) — neither by the design quota nor by the space quota. On `fs` a copy is a hard link,
  so the disk use can be lower than `used`.
- **Download**: `source` defaults to `live`; `live` is the effective file (custom, else default); an unknown
  `source` → 422 `source: invalid`. `Content-Type` by extension, `Cache-Control: no-store`, filename = last segment.
- **Preview / checks**: the draft preview and the checks render `embedded ∪ (live − deleted) ∪ draft` (a restore
  check: `embedded ∪ version`); company and bank are read once per request; `locale` defaults to the company's.
  Only live files go through the shared design cache — draft and version files are read directly for each check.
  Only a template failure (mdcast `render_failed`, a design file over 10 MB, a non-UTF-8 template) becomes
  `design_invalid` (cs is checked first, so its error wins); mdcast unreachable stays 503 `pdf_unavailable`, other
  upstream errors 502 `pdf_render_failed` — nothing changes in either case.
- **Render limits**: the activation / restore checks are not previews (no per-space preview limits) — they take
  the preview share of the instance-wide render slots, never the one kept for issuing (503 `pdf_unavailable` `busy`, which gives the `design-space` attempt back) and keep the 60 s timeout
  ([pdf.md](pdf.md#render-limits)).
- **Rate limit**: bucket `design-space` (10 / hour per space, in memory per instance) shared by activate and
  restore; reserved after the `draft_empty` / 404 / `version_incomplete` checks and before the renders, never
  refunded (also not on `design_invalid`, `design_busy`, `draft_changed`). Reset is not limited.
- **Versions**: snapshots, applying a draft and restoring are server-side copies (S3 `CopyObject`, fs hard link
  renamed into place); a snapshot file's sha256 is computed by streaming the copy. A failed snapshot removes its
  partial directory. Under the lock, before a snapshot, objects of version directories no row references
  (leftovers of a failed commit) are deleted; pruned versions' objects are deleted after the commit, best effort
  (logged). The completeness check of a restore compares presence and size against the row (not the sha256).
- **After an activation's commit** the applied draft files are deleted unless one was re-uploaded meanwhile (its
  ETag — or size + modification time without one — differs); the design cache of the space is invalidated after
  every activation / restore / reset (commit or failure).
- **Restore**: live files not in the version are deleted (hidden files are left alone); restoring a `default`
  version empties the live set like reset but records `revert`. Unknown version (or another space's, or pruned in
  the meantime) → 404.
- **Reset** deletes the whole `design/` prefix (hidden files too) and records `default` with `files: []`, even when
  nothing was custom.
