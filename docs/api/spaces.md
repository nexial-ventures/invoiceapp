# Spaces, roles, data scoping and API tokens (4a)

The instance moves from one static Bearer token to **spaces** (one space = one own company = one accounting unit),
**users** with memberships and **personal API tokens**. `INVOICE__API_TOKEN` is removed. Login, registration and
sessions are in [auth.md](auth.md).

## Hosts
- `INVOICE__PUBLIC_URL` (required, e.g. `https://invoiceapp.cz`; dev `http://localhost:3000`) gives the scheme, the
  **base host** and an optional port. A space lives at `{scheme}://{slug}.{base host}{:port}`
  (`https://firma.invoiceapp.cz`; dev `http://firma.localhost:3000`). DNS / wildcard TLS / the reverse proxy are
  deployment concerns; the proxy must pass the original `Host`.
- Every request is classified by its `Host` (lowercased, port ignored):
  - **base host** → the *base context*: registration, login, the "my spaces" hub, account routes ([auth.md](auth.md));
    business routes → 404 `not_found`.
  - **`{slug}.{base host}`** of an existing space → the *space context*: every business route of phases 1–3, the
    space's login, account and token routes.
  - anything else (unknown slug, deeper subdomain, foreign host) → 404 `{"code":"not_found"}` for every `/api` route.
  - `/api/health` and `/api/openapi.json` answer on any host, without auth.
- The SPA is served on every known host; it asks `GET /api/context` to know which app to show.

`GET /api/context` (no auth) → `200 { kind: "base" | "space", space: { slug, name } | null,
registration: boolean, baseUrl: string }` — `registration` = the instance switch ([auth.md](auth.md)); `baseUrl` =
`INVOICE__PUBLIC_URL`. Unknown host → 404.

## Spaces
Table `spaces` (id uuid, slug unique, name, created_at). Slug: `[a-z0-9-]`, 3–30 chars, no leading / trailing `-`,
not reserved (`www`, `api`, `app`, `admin`, `mail`, `smtp`, `static`, `assets`, `cdn`, `status`, `docs`, `help`,
`support`, `blog`), **immutable**. `name` 1–200 chars.

Base context (session of a verified user, [auth.md](auth.md)):
- `GET /api/spaces` → `[ { slug, name, role, url } ]` — the user's memberships, by name.
- `POST /api/spaces { slug, name }` → `201 { slug, name, role: "owner", url }`. 422: `slug: required | invalid |
  reserved | taken`, `name: required | too_long`; 403 `space_creation_disabled` while `INVOICE__REGISTRATION` is off
  (only invited people enter such an instance, [members.md](members.md)). No limit on the number of spaces per user.
  Creation also creates
  everything a fresh instance used to get from migrations (company profile row, default VAT rates, number series,
  e-mail templates, accounting settings, … — every former global seed, now per space).

Space context:
- `GET /api/space` (any role) → `{ slug, name, url, role }` (the caller's role).
- `PUT /api/space { name }` (admin+) → the same shape.
- `DELETE /api/space { slug, password }` (owner, **session only** — a token gets 403 `forbidden`) → `204`.
  `slug` must equal the space slug (422 `slug: mismatch`), `password` must be the caller's password (422
  `password: invalid`). Deletes in one transaction every row of the space (all tables, memberships, tokens, its
  sessions), then deletes the storage prefix `spaces/{id}/`. A storage failure after the commit is logged at
  `error` and does not fail the request (orphaned files only). The slug becomes free again.

## Memberships and roles
Table `space_members` (space_id, user_id, role, created_at; unique pair). The creator is `owner`. Invitations and
member management: [members.md](members.md) (4b).

| role | may |
|---|---|
| `accountant` | read everything, PDFs, ISDOC / CSV / Pohoda / Money exports; own tokens (role `accountant`) |
| `member` | + create / change documents, contacts, catalog, payments, imports, e-mails, MCP writes |
| `admin` | + everything under Settings (company, accounts, VAT, series, categories, custom fields, design, e-mail, accounting), `PUT /api/space`, list / revoke everyone's tokens, members and invitations up to `admin` ([members.md](members.md)) |
| `owner` | + `DELETE /api/space`, grant / change / remove `owner` ([members.md](members.md)) |

- Enforcement is per route by an extractor (minimum role); GET / HEAD routes need `accountant`, every mutation needs
  at least `member`, settings mutations `admin`. POST routes that only read (`/compute`, previews) need `member`.
  Insufficient role → **403 `{"code":"forbidden"}`**.
- MCP: read tools need `accountant`, write tools `member`; a forbidden tool call returns the MCP tool error
  `{"code":"forbidden"}`.
- The effective role of a request = the membership role (session) or `min(token role, membership role)` (token).
  No membership (removed, or the user disabled) → 401.

## Data scoping
- Every **root** table gets `space_id uuid NOT NULL REFERENCES spaces ON DELETE CASCADE`: company (one row per
  space), bank_accounts, vat_rates, number_series, contacts, documents, catalog_items, catalog_groups, categories,
  custom_fields, e-mail templates / settings / log, accounting_settings (one row per space), import batches if
  stored, and any other table not owned by a parent row. **Child** tables (document lines, VAT recap, payments,
  number series counters, catalog group members, …) are scoped through their parent and get no `space_id`.
- Global (no space): `exchange_rates` (ČNB), ARES lookups, SMTP / mdcast configuration, `users`, `sessions`.
- Unique indexes become per space (document number per series, contact IČO, VAT rate, series code, category name,
  custom field key, …).
- **Repo API shape enforces it:** every repo function on space data takes a `SpaceId` (from the request extractor,
  never from client input); root queries filter `space_id`, child queries join or check through the parent. A
  referenced id from the client (contactId, bankAccountId, categoryId, catalog item, original document, …) that is
  not in the current space behaves exactly like a non-existent id (same 404 / 422 as today). Another space's id in
  a URL → 404.
- Migration: append-only; adds the new tables and the NOT NULL columns. It **aborts with a clear error if any
  space-owned table has rows** (there is no default space; reset the dev database). Former global seed rows are
  removed by the migration and created per space instead.

## Storage
All keys move under `spaces/{space_id}/`: `…/documents/{year}/…` (issued-PDF archive, received originals) and
`…/design/…`. **The mdcast design overlay is per space:** a render reads `spaces/{id}/design/` over the embedded
default, file by file, exactly as 1d / #17 did instance-wide (Settings → Design works per space unchanged).
`invoice design push|pull|ls` gain a required `--space <slug>`. 4f adds `…/design-draft/…` and
`…/design-versions/{id}/…` ([design.md](design.md)). No file migration (no existing data).

## Personal API tokens
For MCP and scripts. A token belongs to **one user in one space** and works only on that space's host.

- Format `inv_{prefix}_{secret}`: `prefix` 8 chars, secret 32 random bytes base64url; stored as sha256 of the whole
  token plus the prefix (shown in lists). Shown **once** at creation.
- `Authorization: Bearer <token>` on the space host → user + space + effective role. Wrong host, unknown, revoked or
  expired token → 401. Bearer requests are exempt from the CSRF Origin check ([auth.md](auth.md)).
- Routes (space context, session or token):
  - `GET /api/tokens` → own tokens `[ { id, name, prefix, role, createdAt, expiresAt, lastUsedAt } ]`; admin+ get
    every token in the space with `user: { email, displayName }`.
  - `POST /api/tokens { name, role, expiresAt? }` → `201 { …, token }`. `role` ≤ the caller's role (422
    `role: too_high`), `name` 1–100, `expiresAt` a future date or null.
  - `DELETE /api/tokens/{id}` → 204; own token, or any token in the space for admin+; else 404.
- `lastUsedAt` is updated at most once a minute per token.

## UI (space host)
- Header: space name, user menu (Account, API tokens, "Moje spaces" → the base host hub, Log out).
- Account page: change password, log out other sessions ([auth.md](auth.md)).
- API tokens page: list, create (name, role, expiry; the token shown once with a copy button), revoke.
- Settings → Space (admin+): name; owner: "Smazat space" with the archiving warning (tax documents must be kept),
  a link to the accountant export, confirmation by typing the slug + password.
- Routes the role may not use are hidden; a 403 still shows a readable message.

## Tests
- Isolation: two spaces with identical data never see each other's records (list, get, update, delete, number
  allocation, exports, MCP, PDFs, storage keys); a foreign id in a URL or a reference → as non-existent.
- Hosts: base vs space vs unknown; token on a foreign space host → 401.
- Roles: the matrix above per route group (incl. MCP tools); token role ceiling.
- Space create (slug rules, seeds present), delete (mismatch, wrong password, token → 403, data and storage gone).
- Every existing integration test runs inside a fresh space through a token or session (`tests/common`).

## Clarifications (as implemented)
- **Host** = the URI authority (HTTP/2) or the `Host` header; lowercased, port and a trailing dot ignored. A name that
  is not `{label}.{base host}` with a label that could be a slug (slug rules, not reserved) is unknown without a DB
  lookup; an IPv6 literal is unknown. Unknown host → plain 404 `{"code":"not_found"}` for every `/api` route incl.
  `/api/context` (the SPA derives the base URL itself). The SPA is served on every host.
- Space-host-only routes on the base host → 404 **before** authentication (also without credentials);
  `/api/spaces` on a space host → 404. An unknown `/api/*` path on a known host → 404 with or without credentials
  (before 4a it was 401 without the token).
- `GET /api/spaces` sorts by name (case-insensitive), then slug. `POST /api/spaces`: registration off → 403
  `space_creation_disabled` first for any signed-in caller with a well-formed JSON body (auth and the JSON extractor
  run before the handler), then an unverified caller → 403 `email_unverified`, both before validation; the hub hides
  the "new space" form while `context.registration` is false; slug trimmed, not lowercased (`Firma` → `invalid`);
  name trimmed, 1–200.
  `taken` comes from the unique index (also a concurrent create).
- **Minimum roles** as implemented (extractors `Read` / `Write` / `Manage` / `Own` in `src/auth/ctx.rs`): every GET
  `accountant` (incl. PDFs, exports, ISDOC, `GET /api/pdf/preview`, `/api/pdf/design`, ARES, exchange rates, e-mail
  status / templates / history / prefill); every business mutation and read-only POST (`/compute`, imports
  preview, e-mail template preview) `member`; Settings mutations `admin`: company, bank accounts, VAT rates, number
  series, categories, custom fields, accounting, e-mail templates PUT / DELETE and the test e-mail, every 4f design
  editor route ([design.md](design.md)); `PUT /api/space`
  `admin`; `DELETE /api/space` `owner`. `/api/tokens` (all three) `accountant`: everyone manages own tokens.
- MCP `tools/list` lists every tool whatever the role; a call below the tool's role is the tool error
  `{"code":"forbidden"}` (read tools `accountant`, write tools `member`).
- A token of an unverified or disabled user, or without a membership → 401. The effective role is recomputed per
  request (a lowered membership caps existing tokens at once).
- `DELETE /api/space` order: role (owner) → 403, then session (a token, even the owner's) → 403, then `slug:
  mismatch` and `password: invalid` reported together (422); wrong passwords count in the user's login bucket
  (429 after 5 / 15 min, [auth.md](auth.md)). The row delete cascades (memberships, tokens, the space's sessions,
  every space-owned row) in one statement; the intra-document FKs without an action
  (`document_lines.advance_document_id`, `documents.category_id`) need no change, because Postgres checks
  `NO ACTION` at the end of that statement, when the referencing rows are gone too (tested with proforma →
  payment → DDPP → final invoice deducting it → credit note → delete). The storage prefix is then removed in a
  detached task (survives a client disconnect) with the backend's bulk delete; a failure is logged at `error`.
- **Tables**: `company` and `accounting_settings` are keyed by `space_id` (one row each, created with the space).
  `number_series` got a surrogate `id`; `number_series_counters` are keyed `(series_id, year)` — a child without
  `space_id`. `document_emails` (the send log) is a child of `documents`. E-mail templates are storage objects of
  the space (`email/templates/{locale}.json`). The per-space unique indexes keep their old names, so the 409 / 422
  mappings are unchanged.
- **Migration** `m20261016_000001_spaces` aborts when documents, contacts, bank accounts, catalog items / groups,
  categories, custom fields or counters exist, or the company row was filled in (name or IČO); the untouched seed
  rows are deleted. Its `down` refuses (restore a backup instead).
- **Storage**: `PdfRoot::space(id)` gives a storage scoped to `spaces/{id}/`; the DB keeps the space-relative key
  (`documents/2026/….pdf`). The shared design cache is namespaced by the space prefix. `invoice storage migrate`
  is unchanged (copies keys as they are; no single-tenant archive is mapped into a space). `invoice design …
  --space <slug>` resolves the slug in the database, so it also needs `INVOICE__DATABASE_URL`.
- **Tokens**: `inv_{prefix}_{secret}`, prefix 8 lowercase hex chars, secret 32 bytes base64url (43 chars); anything
  else in `Authorization: Bearer` → 401 without a lookup. The DB keeps sha256 (hex) of the whole token. `expiresAt`
  on `POST` is a date `YYYY-MM-DD` (or null = no expiry): the token is valid through the end of that day UTC
  (expired from the next UTC midnight); a date before today (UTC) → 422 `expiresAt: invalid`. `role` missing →
  `required`, unknown → `invalid`, above the caller's effective role → `too_high`. List / create responses:
  `expiresAt` the date or null, `createdAt` / `lastUsedAt` RFC 3339; `user` only in admin+ listings; `POST`
  returns the item plus `token`. Lists are oldest first. A token created with a token belongs to the token's user.
- **A token cannot outlive the token that minted it**: when the caller authenticates with a token that has an
  `expiresAt`, the new token's `expiresAt` must be ≤ it — later or null (no expiry) → 422 `expiresAt: too_late`
  (`invalid` wins for a past date). A caller token without expiry, or a session, sets no cap.
- A user's API tokens (all spaces) are deleted by a password change, a password reset and
  `sessions/revoke-others` ([auth.md](auth.md)).
- A Bearer request **with** a foreign `Origin` → 403 `csrf` (the stricter reading of auth.md's CSRF rule); without
  `Origin` it is unaffected.
- Members are added through invitations since 4b ([members.md](members.md)); the role tests still write
  `space_members` rows directly (`tests/common/auth.rs::member`), which is simpler.
- **4c** ([mfa.md](mfa.md)): `SpaceInfo` gains `requireMfa`; `PUT /api/space` takes `{ name?, requireMfa? }`
  (`requireMfa` owner only, session only, needs the owner's TOTP and a step-up `code`, on and off); `DELETE
  /api/space` and `POST /api/tokens` take a step-up `code` for users with TOTP; `POST /api/tokens` in a space with
  the policy → 403 `mfa_required` for a caller without TOTP.
