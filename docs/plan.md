# Invoice — agreed plan

Single-user invoice management: issued and received invoices, metadata, Czech VAT,
PDF rendering through the mdcast service (`xmiksay/mdcast`) with a per-instance design.

Domain logic is ported and simplified from `nexial/infra/src/invoice` (no tenants,
no OAuth, no stock/task/cost-center links). Module layout follows the infra
"Service Module Pattern": `mod.rs` (`Deps`, `connect`, `router`), `config.rs`,
`error.rs`, `openapi.rs`, `entity/`, `repo/`, `handlers/`.

## Decisions

| Topic | Decision |
|---|---|
| Users | Single user until Phase 4: one static Bearer token `INVOICE__API_TOKEN`. **Since 4a:** spaces, users, sessions and personal API tokens ([api/spaces.md](api/spaces.md), [api/auth.md](api/auth.md)). |
| Binary | One crate, one binary: `invoice serve` (runs pending migrations on start), `invoice migrate up\|status\|down`. |
| Frontend | Vue 3 + TS + Vite + Tailwind + Pinia + vue-i18n (cs/en), embedded via rust-embed. |
| Scope | Czech OSVČ / s.r.o., one own company per instance, CZK + foreign currencies. |
| Own company & settings | In DB, edited in the UI: company profile, VAT payer flag, bank accounts per currency, VAT rates (seeded 21/12/0, user-editable), number series. |
| Document types | Invoice (tax document), credit note and debit note (reference the original), simplified tax document, proforma; for VAT payers a tax document for a received advance payment (DDPP) after the proforma is paid (and its correction), and "issue final invoice from proforma" deducting the advance and its VAT. |
| VAT | Rate per line, VAT computed from the per-rate recap (§37 ZDPH) rounded to 0.01. Modes: standard, reverse charge, exempt. Optional rounding of the payable total to whole CZK (rounding line). Foreign-currency invoices also show VAT in CZK. |
| Exchange rate | ČNB daily rate at the tax point date (received invoices: date of receipt), stored on the invoice, manually overridable; if ČNB is unreachable the user must enter it. |
| Numbering | Configurable pattern per document type, e.g. `{YYYY}{NNNN}`, yearly reset, number assigned at issue (drafts have none). Counter is manually settable in Settings. Imported invoices keep their own number and do **not** move the counter. |
| Lifecycle | `draft → issued → sent → paid`, `cancelled`. On issue the invoice is locked, supplier/customer snapshots are stored and the PDF is rendered and archived; issuing fails if mdcast is down. "Overdue" is derived, never stored. Payments: date + amount. |
| PDF | mdcast `/v1/render/template` (typst + JSON data). Default design (minimalist, Inter) embedded in the binary; storage keys `design/…` override it file by file (`invoice.typ`, fonts, logo, signature). Preview of a sample invoice in Settings → Design. Invoice language cs/en per document. SPAYD QR payment code. Details: [api/pdf.md](api/pdf.md). |
| Received invoices | Metadata entered manually + original PDF upload. |
| File storage | `Storage` over `object_store`: filesystem (`INVOICE__STORAGE_DIR`, PVC) or S3-compatible bucket (#9); DB keeps key + sha256. |
| Contacts | One address book for customers and suppliers, ARES lookup by IČO. Invoices store a snapshot, so editing/deleting a contact never changes an invoice. ISDOC import matches contacts by IČO or creates one. |
| Received invoices numbering | Internal evidence number from its own series (e.g. `P{YYYY}{NNNN}`) assigned on save, plus the supplier's original number and VS. |
| Payments | Multiple (partial) payments per document (date, amount, note). Status becomes `paid` automatically when payments cover the payable amount. A DDPP is issued per received advance payment. |

## Phases

Each phase is its own branch + PR into `master`. Phase 1 is split into seven PRs:
- 1a settings + contacts + ARES (done)
- 1b document core: `documents` table, invoice lines (item / text / subtotal with collapse, % discount), VAT recap + rounding, ČNB rates, issue (numbering, snapshots), cancel, payments, counter guard
- 1c proforma → DDPP (auto on payment) → final invoice settlement, credit notes, catalog (items + groups)
- 1d PDF via mdcast + design dir + QR + archive on issue
- 1e received documents (all four types, VAT recap only) + original PDF upload + manual import of issued documents + categories + custom fields
- 1f-a debit notes, DDPP corrections, simplified tax documents (all seven ISDOC document types, both directions)
- 1f-b bulk ISDOC import (`.isdoc`/`.isdocx`, zip-in-zip, preview → confirm) + ISDOC export (moved from phase 2)

### Phase 1 — core
Settings, contacts + ARES, all document types incl. DDPP and proforma settlement,
VAT recap, ČNB rates, numbering, lifecycle + payments, PDF via mdcast with design
dir override + QR, received invoices with PDF upload, import of issued invoices
(manual PDF + metadata; bulk `.isdoc`/`.isdocx`).

ISDOC import notes (FakturaOnline export, ISDOC 6.0.2):
- The export is a **zip inside a zip** — unpack recursively.
- `TaxPointDate` may be missing (non-VAT payer) — fall back to `IssueDate`.
- `PaymentMeans/Payment/PaidAmount` equals the total even for unpaid invoices — do not treat it as a payment.
- Test fixtures must be anonymized (no real IČO/DIČ/IBAN/names).

Document decisions (1b/1c):
- One `documents` table with `direction` (`issued` / `received`); received documents carry only a VAT recap, issued ones have lines and a stored recap computed from them.
- Lines: `item` (description, quantity, unit, unit price excl. VAT, % discount, VAT rate), `text` (no amounts), `subtotal` (ported from infra: references other lines by position, nested allowed, no cycles, all members share one VAT rate; `collapse` shows only the subtotal on the PDF). Catalog groups (1c) insert their members + a collapsed subtotal.
- Credit notes are entered with **positive** amounts; the sign comes from the document type (reports/exports negate).
- Issued documents are locked: only payments, mark-sent, cancel, credit note and the internal note change afterwards. Drafts can be deleted.
- ČNB rate is fixed at issue for the tax point date (latest published on or before it) unless entered manually; cached in `exchange_rates`; ČNB down + no manual rate → issue fails with 422.
- Proforma payments by a VAT payer automatically issue a DDPP (tax point = payment date, VAT from above, split proportionally by the proforma's rates); the final invoice deducts DDPPs per rate (non-payer: deducts the paid amount).
- Deleting a proforma payment cancels its DDPP (reason recorded); refused once that DDPP is deducted by an issued final invoice.
- Foreign-currency proforma payments take an optional manual rate; without it the ČNB rate for the payment date is used, and if ČNB is down the payment is rejected (422) so no DDPP is half-created.
- Credit notes: only for an issued (not cancelled) invoice, correction reason required, tax point = correction date, exchange rate copied from the original invoice, and all non-cancelled credit notes together may not exceed the original's base per VAT rate.
- Catalog items carry name, unit, price, currency and VAT rate; inserting into a document in another currency leaves the price empty (no conversion). Catalog groups = name + member items with quantities; inserting adds the members plus a collapsed subtotal.
- Setting a number-series counter below the highest number already issued from that series/year is rejected.

PDF decisions (1d):
- Drafts can be downloaded as a live render with a "NÁVRH / DRAFT" watermark (no QR, never stored).
- The PDF archived at issue is immutable — never re-rendered (not after cancel, not after a design change).
- Issuing fails (nothing changes) when mdcast is unavailable; an auto-issued DDPP does not block the payment — its PDF is rendered right after the payment, or on first download.
- SPAYD QR only on invoices and proformas paid by bank transfer with an IBAN and a positive payable amount (any currency).
- `INVOICE__MDCAST_URL` defaults to `https://mdcast.nexial.cz`.

Received & import decisions (1e/1f):
- Received documents mirror the issued types (invoice, credit note, proforma, DDPP), each with its own internal number series; no draft — saved = recorded, editable and deletable at any time. Links between a received proforma, its DDPP and the final invoice are informational only (amounts as on the supplier's documents).
- Received metadata: supplier number, received date (ČNB rate date), VAT deductible flag, category, note, custom fields. Original PDF optional, can be uploaded/replaced later.
- Categories: one list with kind expense/income. Custom fields defined in Settings (text/number/date/bool/select, for issued/received/both), stored in `custom_fields jsonb`.
- Manual import of issued documents: any of the four types, full line editor with its own number, issued without number allocation or PDF render; the original PDF can be uploaded. Imported documents never get our rendered PDF.
- ISDOC (1f-b): direction by the supplier IČO vs. company IČO; lines stored, recap/totals taken verbatim from the ISDOC; preview → confirm; duplicates skipped; "mark as paid" option (payment of the full amount on the due date, default on). Per-document outcome: confirm imports only the OK rows, each in its own transaction, and reports per file. Foreign-currency rate from the ISDOC (`CurrRate`/`RefCurrRate`), else ČNB. Batch options for received documents: category (optional), VAT deductible (default yes), received date = tax point date; edited per document afterwards. Limits 50 MiB upload / 500 documents / 3 zip levels / 200 MiB unpacked; confirm re-sends the files (stateless). `OriginalDocumentReference` is linked when found (DB or same batch). Export: issued non-draft documents as `.isdocx` (ISDOC + archived PDF or original), from the detail and in bulk from the list filter (≤ 1000). Details: [api/isdoc.md](api/isdoc.md).

Document type decisions (1f-a, details in [api/doc-types.md](api/doc-types.md)):
- Debit note (`V…`): created empty from an issued invoice / simplified document, reason required, original's rate, payable with QR; issued debit notes raise the credit-note cap.
- DDPP correction (`OP…`): created from an issued DDPP like a credit note (lines copied, cap = DDPP base per rate); the final invoice deducts the DDPP net of its corrections; refused once the DDPP is deducted.
- Simplified tax document (`ZD…`): its own type and series; customer optional; > 10 000 CZK only warns in the UI; credit and debit notes allowed, no advances.
- Received counterparts `PV…`, `POP…`, `PZD…`; manual import accepts all seven types.

### Phase 2 — interchange
Order: storage abstraction (#9, pulled forward) → 2a e-mail → 2b CSV/XLSX import → 2c CSV export, one PR each.

Storage (#9, before 2a):
- `Storage` over the `object_store` crate: put (sha256, atomic), get (stream), delete, exists, list by prefix. Backend by
  `INVOICE__STORAGE_KIND` = `fs` (default, `INVOICE__STORAGE_DIR`) or `s3` (`INVOICE__S3__ENDPOINT`, `BUCKET`, `REGION`,
  `ACCESS_KEY_ID`, `SECRET_ACCESS_KEY`, `PATH_STYLE`); the Docker image works without S3.
- Everything goes through it: PDF archive, originals, e-mail templates (2a) **and the design** (keys `design/…`, overlaid
  on the embedded default file by file as before). `INVOICE__DESIGN_DIR` is removed; files reach storage via
  `invoice design push|pull|ls` or straight into the bucket (UI comes with #10). Keys stay as today
  (`documents/{year}/…`); the `spaces/{id}/` prefix arrives with #3.
- Design reads list the `design/` prefix per render and cache file contents by key + ETag/size, so unchanged fonts are
  not downloaded again.
- Storage unreachable → 503 `storage_unavailable` (issue fails and changes nothing, like mdcast down).
- `invoice storage migrate --from-dir <path> [--design-dir <path>]` copies the fs archive (and an old design dir) into
  the configured backend, verifies sha256, is idempotent and never deletes the source.
- Tests: one shared suite against fs and S3. S3 = test bucket `invoice-test` on our Garage (`s3.mmik.cz`, path-style,
  region `garage`), `TEST_S3_*` in `.env` locally and GitHub secrets in CI; every test uses a random prefix and cleans
  up. Fails, not skips, without it.
- As implemented: `invoice design rm <path>…` added next to push / pull / ls; `migrate` never overwrites an object with
  different content (reported, exit 1); the design cache compares the listed version (ETag + size + modification
  time); `GET /api/pdf/design` returns `storage: "fs" | "s3"` instead of `designDir`; an ISDOC export whose PDF cannot
  be read because the storage is down fails with 503 instead of silently leaving the PDF out (the bulk export stops at
  the first such document); a leftover `INVOICE__DESIGN_DIR` refuses the start; `migrate` / `design push` decide
  "identical" by size + MD5 ETag where the backend offers one (full download otherwise) and report unreadable local
  entries (exit 1); a custom endpoint without path-style gets the bucket put into its host; an ISDOC import confirm whose
  storage is down fails that entry with `storage_unavailable` (like `rate_unavailable`).

2a e-mail (SMTP via `lettre`):
- SMTP from env only (`INVOICE__SMTP__HOST/PORT/USERNAME/PASSWORD/FROM/TLS`); Settings shows configured yes/no and
  sends a test e-mail.
- Send dialog: To (prefilled from the contact, several addresses), Cc, Bcc, optional Bcc to the company e-mail
  (ticked), subject and body rendered from the template and editable before sending; attachments PDF and plain
  `.isdoc` (checkboxes). Any issued, non-draft document of ours (cancelled too); never received ones.
- Templates: MiniJinja, plain text, subject + body per locale (cs/en), defaults embedded, overrides stored in Storage;
  Settings → E-mail editor with a preview on a sample document and "restore default". Saving compiles and renders the
  template on the sample in strict mode; an error → 422 with line + message, the active template stays.
- Sending is synchronous (30 s timeout); every attempt is logged (to/cc/bcc, subject, time, ok/error, Message-ID);
  success sets `sentAt`; re-sending allowed; the history is shown on the document detail.
- Decided in the 2a grill: one template set for all document types (the type is a variable); `Reply-To` = the
  company e-mail when set; the log stores the body too. Contract: [`docs/api/email.md`](api/email.md).

2b CSV/XLSX import: one template for both directions and all seven types, one row = one document, VAT recap in
per-rate columns (`base_21`, `vat_21`, `base_12`, `vat_12`, `base_0`), contacts matched by IČO or created, sample
downloadable in the UI, preview → confirm like ISDOC.
- Decided in the 2b grill: one format for import and export (export → import round-trips); recap per rate **in CZK
  only** (foreign documents: `total` in their currency + `exchange_rate`, the currency recap is derived on import);
  rate columns dynamic from Settings → VAT rates; UTF-8 with Windows-1250 fallback; `paid_date` = one full payment;
  contacts matched by IČO → DIČ → exact name, else created; unknown categories created; limits as ISDOC (50 MiB /
  500 rows); 2b and 2c stay separate PRs. A per-space accounting currency and rounding are noted on #3.
  Contract: [`docs/api/csv.md`](api/csv.md).
- As implemented: the ISDOC import pipeline (lookups, per-entry check, transaction, wire types) moved to
  `src/import/` and both imports use it; the format lives in `src/csvio/format.rs` + `write.rs` for 2c to reuse;
  details in the csv.md Clarifications.

2c CSV export for the accountant: issued and received per the list filter, one row = one document, `;` separator,
UTF-8 with BOM, decimal comma, dates `dd.mm.yyyy`; columns direction, type, number, supplier number, dates,
counterparty + IČO/DIČ, currency, rate, base/VAT per rate in CZK, total, paid, category; credit notes negative.
- Decided in the 2c grill: English header names stay (export = import format); export from the issued / received
  list (current filter) and an "accountant" export by tax-date period for both directions in one file; drafts and
  cancelled never exported, proformas only through a list filter that includes them. Contract: the Export section
  of [`docs/api/csv.md`](api/csv.md).
- As implemented: `src/csvio/export*.rs` on the 2b writer (`write::head` + `write::body` chunks), streamed 500
  documents at a time; details in the csv.md Clarifications (Export).

### Phase 3 — accounting & MCP
Pohoda XML (Stormware) and Money S3 XML export of issued + received invoices per
period; MCP endpoint (list/create/issue invoices) behind the same Bearer token.

Order (3 grill): 3a MCP → 3b Pohoda XML → 3c Money S3 XML, one PR each.
- 3a MCP: streamable HTTP, stateless, `POST /api/mcp` (`rmcp`), same Bearer token. Tools: read (documents, contacts,
  ARES, catalog, settings, compute) + create contact, create / update draft, issue (annotated destructive, no extra
  confirm step), add payment, mark sent. No cancel / delete / credit notes / e-mail / received writes. No PDF over
  MCP — results carry `pdfUrl` for a REST download with the same token. Contract: [`docs/api/mcp.md`](api/mcp.md).
- 3b / 3c (both programs are used): summary per VAT rate in CZK (no lines), like the CSV export; accounting codes
  (předkontace, členění DPH) configurable per document type in Settings, empty by default for the accountant to fill.
- 3b grill: Pohoda / Money are formats of the 2c accountant export (same period, direction and document set: no
  proformas; summary per VAT rate in CZK, no lines); DDPPs and their corrections are exported as advance tax
  documents; accounting codes per direction × document type in Settings → Accounting, empty by default.
  Contract: [`docs/api/accounting.md`](api/accounting.md).
- 3c grill: Money S3 native XML (UTF-8, `MoneyData`), validated against the Money S3 XSDs (vendored from the public
  WeblateOrg/website copy). DDPPs and their corrections → Money's own `SeznamFaktVyd_DPP` / `SeznamFaktPrij_DPP`
  lists. A number longer than Money's 10-char `Doklad` → `Doklad` omitted (Money numbers it from the series), ours
  kept in `EvCisDokl` / `PrijatDokl`. Codes: own `money` section shaped like Pohoda (≤ 10, series ≤ 5, agenda IČO).
  Non-deductible received document with no code → `KodDPH` left out. Any VAT rate maps (`SeznamDalsiSazby`).
  Contract: [`docs/api/money.md`](api/money.md).

### Phase 4 — multi-tenancy (epic #11)
Order (4 grill): 4a spaces + users + login + personal tokens (#3, #4, #7 and the storage side of #10) → 4b
invitations + members (#8) → 4c TOTP (#5) → 4d Google (#6) → 4e space settings (accounting currency, rounding,
defaults; #3 comments) → 4f per-space design versions / preview (rest of #10). One PR each.
- 4a: one space = one own company. Space per **subdomain** `{slug}.{base host}` of `INVOICE__PUBLIC_URL`
  (`invoiceapp.cz`); the base host hosts registration, login and the "my spaces" hub. Slug immutable, validated,
  reserved names. `INVOICE__API_TOKEN` removed; **public registration** (e-mail verification required, SMTP
  required) behind `INVOICE__REGISTRATION`; no CLI bootstrap, no superadmin, no limit on spaces per user. Sessions
  host-only (log in again on each space host, no handoff); idle 14 d / absolute 90 d; password ≥ 12, argon2id;
  reset by e-mail. Roles owner / admin / member / accountant with a fixed matrix; tokens bound to one user + space
  with role ≤ membership. Root tables carry `space_id`, children via parent, repo API takes `SpaceId`; ČNB rates
  global. Storage keys and the mdcast design overlay per space under `spaces/{id}/`. No default space, no data
  migration (no existing data). Owner deletes a space (slug + password), DB rows and storage prefix.
  Contracts: [`docs/api/spaces.md`](api/spaces.md), [`docs/api/auth.md`](api/auth.md).
- 4b grill: admin and owner manage members (admin up to `admin`, only an owner grants / changes `owner`; the last
  owner cannot be demoted, removed or leave). Invitations bound to the e-mail, 7 days, single use, sent by e-mail and
  shown as a copyable link; a pending one for the same e-mail is replaced; inviting a member → 422. Accept in one step
  on the space host (existing account: password; new account: name + password, verified, works with registration
  off) → membership + session. Members list admin+ only; anyone may leave. Removal / leave deletes the member's
  sessions and tokens in that space at once. Contract: [`docs/api/members.md`](api/members.md).
- 4c grill: TOTP per user, secrets encrypted with the new required `INVOICE__SECRET_KEY` (AES-256-GCM), 10 recovery
  codes. Owner can require MFA per space: a member without TOTP is refused at login (enrol on the base host), the
  policy applies from the next login (existing sessions / tokens keep working). Disabling TOTP is allowed but drops
  the user's sessions and tokens in requiring spaces at once. Invitations into such a space need TOTP enabled
  first. Step-up code for password change, space delete, token create, invite accept, MFA management. No "trust
  this browser". Contract: [`docs/api/mfa.md`](api/mfa.md).
- 4d grill: Google OIDC (code + PKCE); the only redirect URI is on the base host, so a Google login for a space goes
  through a one-time 60 s handoff token to the space host (password login still never crosses hosts). Unknown
  e-mail → new verified account when registration is on, or via a matching invitation; an existing account with the
  same e-mail is linked automatically. Google-only accounts must set a password (reset e-mail, or within 10 min of a
  Google login) before password-requiring actions; unlink needs a password. TOTP and the space policy apply as for
  password login. Contract: [`docs/api/google.md`](api/google.md).
- 4e grill: the accounting currency stays CZK for every space (a non-CZK space means another country's VAT law —
  a separate epic when needed). `roundTotal` is replaced by `roundingMode` none / half (0.50) / unit for every
  currency; the company profile (= the space) gains document defaults: rounding for cash and for other payment
  methods, payment method, currency, header / footer notes per locale. Contract: [`docs/api/defaults.md`](api/defaults.md).
- As implemented (4a): migration `m20261016_000001_spaces` (refuses existing data), modules `src/auth/` (hosts,
  sessions, tokens, CSRF, rate limits) and `src/space/` (`SpaceId`, roles, per-space seeds), storage scoped to
  `spaces/{id}/`; details in the Clarifications of [spaces.md](api/spaces.md) and [auth.md](api/auth.md).

## Out of scope (for now)
Deployment manifests, VAT return / control statement XML (EPO), automatic payment
reminders, multiple own companies per space (one space = one company; several companies = several spaces), Pohoda/Money S3 import.
