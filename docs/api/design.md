# Per-space PDF design: upload, draft, activation, versions (4f)

Completes #10. The design model of [pdf.md](pdf.md) stays: the embedded default (`design/` in the repo) overridden
file by file by the space's stored **live** files under `spaces/{id}/design/`. 4f adds editing from the UI with a
**draft** that is validated and previewed before it becomes live, and a **history of activations**. Archived PDFs
are never re-rendered (1d). All routes are on a space host, **admin+** (Settings); every key below is relative to
`spaces/{id}/`.

## Storage layout
- `design/…` — live files (unchanged; also written by `invoice design push --space`).
- `design-draft/…` — the draft: uploaded files that will override live ones on activation.
- `design-versions/{versionId}/…` — a full copy of the live set right after an activation / revert.
- Table `design_drafts` (space_id PK, deleted paths `text[]`, updated_at, updated_by) — draft deletions (a live file
  to remove on activation). Table `design_versions` (id uuid, space_id, number per space 1.., kind `activation` |
  `revert` | `default`, created_at, created_by, files `jsonb` [{ path, size, sha256 }]). The newest row is the live
  state.

## Files and limits
- Allowed extensions: `.typ`, `.toml`, `.ttf`, `.otf`, `.woff2`, `.svg`, `.png`, `.jpg`, `.jpeg`. Path: lowercase
  `[a-z0-9._-]` segments, at most 2 levels (`logo.svg`, `fonts/inter.ttf`), no `..`, not `qr.svg` (reserved, 1d).
- File ≤ 10 MB (as today). **Quota per space: 50 MB** over live + draft + kept versions (counted by stored object
  size). A file over 10 MB → 422 `{"code":"validation","fields":{"file":"too_large"}}`; over the quota → 422
  `{"code":"quota_exceeded","detail":"<used> / 50 MB"}`. Bad path / extension → 422 `path: invalid`.
- `invoice.typ` must be UTF-8 (1d rule).

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
  **embedded ∪ live ∪ draft (minus deletions)**; render errors as 1d (`502 pdf_render_failed` with `detail`).
- `POST /api/pdf/design/activate` → `200 { version }`: under a per-space lock, (1) renders the sample with the
  draft set in **both** locales — any failure → 422 `{"code":"design_invalid","detail":"<locale>: <mdcast
  message>"}` and nothing changes; (2) writes the new live set: copies draft files to `design/`, deletes the draft
  deletions; (3) copies the resulting live set to `design-versions/{id}/` and inserts the version row; (4) clears the
  draft. Empty draft → 422 `{"code":"draft_empty"}`.
- `GET /api/pdf/design/versions` → `[ { id, number, kind, createdAt, createdBy: { email, displayName }, files,
  current: boolean } ]`, newest first.
- `POST /api/pdf/design/versions/{id}/restore` → `200 { version }`: validates that set like activation (both
  locales), then replaces live with it (live files not in the version are deleted) and records a `revert` version.
  The draft is untouched.
- `POST /api/pdf/design/reset` → `200 { version }`: deletes every live custom file (the embedded default applies),
  records a `default` version (no files). No validation needed.
- Keeping: the **10 newest versions**; older rows and their `design-versions/{id}/` objects are deleted after each
  new version.
- Failure handling: a storage failure in step (2)/(3) → 503 `storage_unavailable`; the version row is only written
  after the files, so a failed activation leaves no version claiming a set that is not live; the draft is kept so
  the user can retry. Restoring a version is the recovery path for a half-applied live set.
- Rate limit: activation / restore 10 per hour per space (each costs two mdcast renders); 429 `rate_limited`.

## Interaction with existing behaviour
- Renders (issue, preview) keep reading **live** only. The design cache (1d, per space since 4a) is invalidated by
  the activation / restore / reset of that space.
- `invoice design push|pull|ls|rm --space` keep working on live directly (operator tool, no versions); the next
  activation snapshots whatever is live.
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
(filled during the 4f implementation)
