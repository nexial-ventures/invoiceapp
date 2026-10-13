# API contract

All routes under `/api`; since 4a authenticated by a session cookie or a personal Bearer token on a space host
([spaces.md](spaces.md), [auth.md](auth.md)). JSON camelCase on the wire
(`#[serde(rename_all = "camelCase")]`). Money/rates are decimals serialized as STRINGS
(e.g. `"21"`, `"12.5"`) to avoid float loss. Decimal *inputs* of document lines, `exchangeRate`, compute and payments also accept a JSON
number (taken by its shortest text, then validated like the string) — see `src/num_text.rs`. IDs are UUID strings. Dates `YYYY-MM-DD`.

## Errors
- 404 `{"code":"not_found"}`
- 422 `{"code":"validation","fields":{"<field>":"<reason_code>"}}` — reason codes:
  `required`, `invalid`, `too_long`, `duplicate`, `invalid_ico`, `invalid_pattern`.
  Field names are the camelCase wire names; for list items use `items.0.rate` style if ever needed.
- 409 `{"code":"conflict"}` only for unique violations and concurrent-change races not attributable to one field; a
  refusal with a known cause has its own code (e.g. `mfa_disabled`, `email_taken`, `last_owner`).
- ARES: 404 `{"code":"ares_not_found"}`, 502 `{"code":"ares_unavailable"}`, 422 `{"code":"validation","fields":{"ico":"invalid_ico"}}`.


Later phases add more codes (`document_locked`, `invalid_state`, `cnb_unavailable`, `advance_settled`, `related_in_use`, `smtp_failed`, `template_invalid`, …) — see each part.
422 field keys for list items use the 0-based array index: `lines.0.unitPrice`, `members.2.itemId`.

## Conventions
### PUT semantics
- `PUT` **replaces** the resource: every field of the body is written, an omitted optional field is stored as its
  default / `null`, a missing required field is 422 `required` (never a serde 400 — required fields are `Option` in
  the input struct and checked by `FieldErrors`).
- **"Absent = keep"** applies only to these named settings singletons and fields, each documented in its part:
  - `PUT /api/settings/company` — the document defaults (`defaultRounding*`, `defaultPaymentMethod`,
    `defaultCurrency`, `defaultHeaderNote`, `defaultFooterNote`); absent or `null` = keep ([defaults.md](defaults.md)).
    The profile fields are replaced.
  - `PUT /api/settings/accounting` — the `pohoda` / `money` sections; absent or `null` = keep ([money.md](money.md)).
  - `PUT /api/space` — `name` / `requireMfa`; absent = unchanged ([mfa.md](mfa.md)).
  - `PUT /api/documents/{id}` — `docType` omitted keeps the draft's type, `number` omitted keeps the draft's number
    (explicit `null` clears it, [numbering.md](numbering.md)); the rest is replaced.
- A new route does not add to this list without a reason recorded in its part.

### List shape
- Paged lists answer `{ items: T[], total }` with `limit` / `offset` query parameters: `GET /api/documents`,
  `GET /api/contacts` (and the MCP `list_*` tools).
- Every other list is a **plain JSON array** of all rows, unpaged (small per-space sets): catalog items / groups,
  bank accounts, VAT rates, number series, categories, custom fields, payments, the e-mail log, members, invitations,
  tokens, spaces, design versions. `GET /api/settings/email/templates` is `{ templates: [...] }`.

### Exports
- Exports are `GET` downloads (`Content-Disposition: attachment`). New export formats go under `/api/export/*`
  (`/api/export/csv`, `/api/export/accountant?format=csv|pohoda|money`); a new format of an existing export is a
  `format` value there, not a new route. The per-document ISDOC routes (`/api/documents/{id}/isdoc`,
  `/api/documents/isdoc`) predate the rule and stay where they are.

### Access
- Every route in `/api/openapi.json` has an access decision (public, account, space role, or exempt with a reason)
  in `tests/spaces_route_roles.rs`; a route without one fails the test.

## Parts
| File | Scope |
|---|---|
| [settings-contacts.md](settings-contacts.md) | company, bank accounts, VAT rates, number series, contacts, ARES (1a) |
| [documents.md](documents.md) | documents core: lines, computation, lifecycle, payments, ČNB (1b) |
| [advances-credit-catalog.md](advances-credit-catalog.md) | proforma, DDPP, settlement, credit notes, catalog (1c) |
| [pdf.md](pdf.md) | PDF via mdcast: design files, payload, QR, archive, preview (1d) |
| [received-import.md](received-import.md) | received documents, original PDF upload, manual import of issued, categories, custom fields (1e) |
| [doc-types.md](doc-types.md) | debit notes, DDPP corrections, simplified tax documents (1f-a) |
| [isdoc.md](isdoc.md) | ISDOC bulk import (preview → confirm) and export (1f-b) |
| [email.md](email.md) | SMTP sending, MiniJinja templates, send log (2a) |
| [csv.md](csv.md) | CSV / XLSX interchange format, import (2b) and export (2c) |
| [mcp.md](mcp.md) | MCP endpoint: read tools, drafts, issue, payments (3a) |
| [accounting.md](accounting.md) | accounting settings, Pohoda XML export (3b) |
| [money.md](money.md) | Money S3 XML export and its settings (3c) |
| [spaces.md](spaces.md) | hosts, spaces, roles, data scoping, personal API tokens (4a) |
| [auth.md](auth.md) | users, registration, login, sessions, password reset, CSRF (4a) |
| [members.md](members.md) | members, roles management, invitations, leaving a space (4b) |
| [mfa.md](mfa.md) | TOTP second factor, recovery codes, space MFA policy, step-up (4c) |
| [google.md](google.md) | Sign in with Google (OIDC + PKCE), handoff to space hosts, password-less accounts (4d) |
| [defaults.md](defaults.md) | document defaults per space (company), rounding modes none / half / unit (4e) |
| [design.md](design.md) | per-space PDF design: upload, draft, preview, activation, versions (4f) |
| [numbering.md](numbering.md) | per-client number sequences, manual numbers on drafts, number preview |
| [overview.md](overview.md) | period overview: income / expenses, VAT per rate, monthly trend, categories, receivables / payables (#54) |

Each part ends with a "Clarifications (as implemented)" section — the authoritative record of behaviour beyond the
original contract. Keep every file under 400 lines; add a new part rather than growing one past the cap.

## Clarifications (as implemented)

Cross-cutting HTTP and runtime behaviour that belongs to no single part.

- **Security headers** (`src/security_headers.rs`, outermost middleware): every response (SPA, API, errors) gets
  `Content-Security-Policy` (`CSP`: `'self'` only, no `unsafe-*`; `img-src` also `data:` (the QR code) and `blob:`;
  `object-src 'none'`, `frame-ancestors 'none'`, `base-uri` / `form-action 'self'`; no `sandbox`, so inline PDFs and
  `blob:` PDF tabs still open in the browser viewer), `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: strict-origin-when-cross-origin` and `X-Frame-Options: DENY`.
- **HSTS**: `Strict-Transport-Security: max-age=31536000; includeSubDomains` only when `INVOICEAPP__PUBLIC_URL` is https
  — read the deployment warning about `includeSubDomains` in the [README](../../README.md) first.
- A header the handler already set wins (the Google redirects' `no-referrer`). A frontend change that needs an inline
  script / style, `eval` or another origin must change `CSP` too.
- **Compression** (`src/compression.rs`, inside the security headers): brotli (quality 4) or gzip by
  `Accept-Encoding` (`Vary: Accept-Encoding`) for JSON, text, CSV, XML and the SPA's HTML / JS / CSS / SVG. Never
  compressed: bodies under 32 bytes, images (but SVG), fonts, PDF, zip / gzip, `application/vnd.*` (zip-based office
  files), `application/octet-stream`, `text/event-stream` — they keep their `Content-Length` and stream as before.
  Responses carrying a fresh secret (API token create, MFA setup / enable / recovery codes) are marked
  `Content-Encoding: identity` (`Identity(..)`) and never compressed (BREACH).
- **Expired-row purge** (`src/cleanup.rs`): hourly, first run at start: sessions past idle (14 d) or absolute expiry,
  `user_tokens`, `mfa_logins`, `google_flows` / `google_handoffs`, `space_invites` past `expires_at`, `email_sends`
  older than 2 days. Counts logged at debug; a failure is logged and retried the next hour.
- **Database timeouts** (`src/db.rs`): the serve pool runs with server-side `statement_timeout` and
  `idle_in_transaction_session_timeout` (30 s, none with `INVOICEAPP__DB_PGBOUNCER`); migrations run first on a separate
  plain connection. `db::allow_idle` lifts the idle timeout for the design editor's lock transaction, and
  `space::limits::lock` lifts `statement_timeout` for the rest of its transaction, as a design upload may hold the
  quota lock across a storage put.
- **Health**: `GET /api/health` answers on any host without auth: `{"status":"ok","version"}`, or 503
  `{"status":"degraded"}` when the DB ping fails.
- **Start-up**: `invoiceapp serve` probes the storage (`Storage::probe`) and only logs a failure; it shuts down
  gracefully on SIGINT / SIGTERM.
