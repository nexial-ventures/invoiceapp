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
- 409 `{"code":"conflict"}` for unique violations not attributable to one field.
- ARES: 404 `{"code":"ares_not_found"}`, 502 `{"code":"ares_unavailable"}`, 422 `{"code":"validation","fields":{"ico":"invalid_ico"}}`.


Later phases add more codes (`document_locked`, `invalid_state`, `cnb_unavailable`, `advance_settled`, `smtp_failed`, `template_invalid`, …) — see each part.
422 field keys for list items use the 0-based array index: `lines.0.unitPrice`, `members.2.itemId`.

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

Each part ends with a "Clarifications (as implemented)" section — the authoritative record of behaviour beyond the
original contract. Keep every file under 400 lines; add a new part rather than growing one past the cap.
