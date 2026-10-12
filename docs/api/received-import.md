# Received documents, categories, custom fields, manual import (phase 1e)

Bulk ISDOC import is phase 1f (separate part). Everything here extends the existing `documents` API
([documents.md](documents.md)); unchanged rules (payments, payment state, overdue, list paging) apply to received
documents too.

## Received documents (`direction = "received"`)

Doc types: `invoice`, `credit_note` (sign −1), `proforma`, `advance_tax_doc` — the same `docType` values as issued
documents, distinguished by `direction`. No lines: the supplier's amounts are entered as a **VAT recap**.

### Lifecycle
- No draft: `POST /api/documents` with `direction: "received"` stores the document with `status: "issued"` (UI label
  "Zaevidováno" / "Recorded") and allocates the internal number from the series of its doc type in the same
  transaction (year = `receivedDate` year).
- Editable at any time with `PUT /api/documents/{id}` (the internal number never changes, also not when the year of
  `receivedDate` changes). `DELETE` allowed at any time (the number is not returned to the series; payments and the
  stored original PDF are deleted with it).
- `cancel`, `mark-sent`, `issue`, `settle`, `credit-note` on a received document → 409 `invalid_state`.
- Payments work as for issued documents (outgoing payments). Payments on a received proforma **never** create a DDPP.

### Number series
New series (seeded, editable in Settings like the others; `docType` keys of `/api/settings/number-series`):
| key | received doc type | seed pattern |
|---|---|---|
| `received` | invoice | `P{YYYY}{NNNN}` (existing) |
| `received_credit_note` | credit_note | `PD{YYYY}{NNNN}` |
| `received_proforma` | proforma | `PZ{YYYY}{NNNN}` |
| `received_advance_tax_doc` | advance_tax_doc | `PDP{YYYY}{NNNN}` |
The counter guard and "no duplicate pattern" rule apply unchanged. Internal numbers are unique per (direction,
docType).

### Wire (Document, received)
Fields beyond / instead of the issued shape:
```
direction: "received",
docType: "invoice" | "credit_note" | "proforma" | "advance_tax_doc",
number: string,                       // internal evidence number (read-only)
supplierNumber: string,               // required, <=40, the supplier's own document number
contactId: uuid,                      // required: the supplier (from the shared address book)
issueDate: date,                      // required
taxPointDate: date | null,            // required except proforma (must be null for proforma)
receivedDate: date,                   // required; default (client + server) = taxPointDate ?? issueDate
dueDate: date | null,                 // required except advance_tax_doc
currency, exchangeRate, exchangeRateDate, exchangeRateSource,   // as issued; ČNB rate for receivedDate,
                                      // fixed on create/when currency or receivedDate changes, manual override wins
vatMode: "standard" | "reverse_charge" | "exempt" | "non_payer",   // non_payer = supplier is not a VAT payer
vatRecap: [ { rate: string, base: string, vat: string } ],   // >=1 row, rate unique, entered as on the document
                                      // (positive for credit notes); vat must be 0 unless vatMode standard
rounding: string,                     // default "0", entered as on the document, |rounding| < 100
total: string,                        // read-only = Σ(base+vat) + rounding
payable: string,                      // required, entered (e.g. a final invoice after advances), >= 0
vatDeductible: boolean,               // default true when vatMode = standard, else false
variableSymbol, constantSymbol: string | null,
supplierAccount: string | null,       // <=60, free text (IBAN or CZ account) to pay to
relatedDocumentId: uuid | null,       // informational link: received DDPP / final invoice → received proforma,
                                      // received credit note → received invoice; must be a received document
categoryId: uuid | null, customFields: object, internalNote: string | null,
original: { sha256, size, uploadedAt } | null,   // uploaded original PDF
```
`totalCzk` / recap CZK computed from the rate as for issued documents. `supplier` snapshot = the contact at the last
save (refreshed on every PUT). `relatedDocuments` lists the link in both directions.

422 field keys for recap rows: `vatRecap.N.rate|base|vat` (0-based). Unknown rate value (not in Settings VAT rates) is
**allowed** for received documents (foreign VAT, old rates); rate 0..100, 2 dp.

## Original PDF (received and imported issued documents)
- `PUT /api/documents/{id}/original` — `multipart/form-data`, one part `file`; must start with `%PDF-`, ≤ 20 MB
  (413 `{"code":"too_large"}`; other → 422 `{"fields":{"file":"invalid"}}`). Replaces any previous original. Stored
  under the storage key `documents/{year}/{id}-original-{sha8}.pdf` (atomic put + sha256 like the archive; see
  [pdf.md § Storage](pdf.md#storage)); storage failure → 503 `storage_unavailable`, nothing changed. Over the space's
  storage quota ([spaces.md](spaces.md#limits); a replacement counts only what it adds) → 422
  `{"code":"quota_exceeded","detail":"<used> / <limit> MB"}`, nothing changed.
  Allowed for every received document and for imported issued documents (any status); a non-imported issued
  document → 409 `invalid_state` (its PDF is the rendered archive).
- `DELETE /api/documents/{id}/original` → 204 (same rules).
- `GET /api/documents/{id}/pdf` for received / imported documents serves the original; none → 404
  `{"code":"pdf_missing"}`. Never rendered with our design.
- `pdf` (rendered archive) stays `null` for received / imported documents; `original` is `null` for native issued ones.

## Manual import of issued documents (`imported = true`)
- Any of the four issued doc types. Created as a **draft** with `imported: true` and a required `number` (<=40,
  unique per issued docType → 422 `{"fields":{"number":"duplicate"}}`); full line editor and computation as native
  drafts. Optional `relatedDocumentId` (DDPP / final invoice → proforma, credit note → invoice) instead of the
  native settlement/credit flows; deduction lines (`advance`) are not available.
- `POST /api/documents/{id}/issue` on an imported draft: same validation, snapshots and ČNB rules, but **no number
  allocation** (keeps its `number`; `numberYear` = issueDate year, `numberSeq` null) and **no PDF render**. Payments
  on an imported proforma never create a DDPP. The counter guard ignores imported documents (unchanged).
- Body of create/PUT for issued drafts gains `imported: boolean` (default false; immutable after create → 422
  `{"fields":{"imported":"invalid"}}` if changed) and `number` (required iff imported, otherwise must be absent/null).
- A native issue whose allocated number collides with an imported one → 409 `{"code":"number_taken"}` (set the
  counter in Settings).

## Categories — `/api/settings/categories`
```
Category { id, name: string /* required <=100, unique per kind */, kind: "expense" | "income", active: boolean, position: number }
```
`GET` (ordered kind, position, name), `POST`, `PUT /{id}`, `DELETE /{id}` (204; used by a document → 409
`{"code":"category_in_use"}` — deactivate instead). Documents: `categoryId` optional; received documents accept only
`expense`, issued only `income` (else 422 `categoryId: invalid`); an inactive category stays on documents that have it
but cannot be newly assigned (422 `categoryId: inactive`).

## Custom fields — `/api/settings/custom-fields`
```
CustomField { id, key: string /* ^[a-z][a-z0-9_]{0,39}$, unique, immutable */, label: string /* <=100 */,
  type: "text" | "number" | "date" | "bool" | "select", options: string[] /* select only, 1..50 unique, each <=100 */,
  appliesTo: "issued" | "received" | "both", required: boolean, active: boolean, position: number }
```
`GET`, `POST`, `PUT /{id}` (key and type immutable → 422 `invalid`), `DELETE /{id}` (204; values stay in documents'
JSON and are ignored).
- Documents carry `customFields: { [key]: value }` (`documents.custom_fields jsonb NOT NULL DEFAULT '{}'`). Values:
  text string <=500, number decimal string, date `YYYY-MM-DD`, bool boolean, select one of `options`; `null` /
  missing = empty. Validation on every save against the **active** definitions that apply to the document's
  direction: unknown key → 422 `customFields.<key>: unknown`; wrong type → `invalid`; required + empty → `required`
  (required is enforced on create/PUT and — for issued drafts — at issue). Keys of inactive/deleted definitions
  already stored are kept untouched on PUT if sent back unchanged.

## Metadata after issue
`PUT /api/documents/{id}/metadata` body `{ categoryId, customFields, internalNote }` → Document; allowed in every
status for both directions (replaces the 1b `PUT …/internal-note`, which stays as an alias). Same validation.

## List
`GET /api/documents?direction=received` (existing filter) + new filters `categoryId=`, `imported=true|false`. List
items gain `supplierNumber`, `categoryId`, `hasPdf` (rendered archive or original present). `q` also matches
`supplierNumber`.

## Errors (new)
- 404 `pdf_missing`, 413 `too_large`, 409 `category_in_use`, 409 `number_taken`.

## Clarifications 1e (as implemented)
Additive details settled during the Phase 1e backend; nothing above changed.

Wire (agreed with the frontend)
- `DocumentSummary` gains `imported`, `supplierNumber`, `categoryId`, `hasPdf`; for received documents the counterparty
  (supplier) name is in the existing `customerName` (snapshot, else the live contact name). `dueDate` is nullable in
  `Document` and `DocumentSummary` (only a received DDPP may lack it).
- Every `Document` gains `imported`, `supplierNumber`, `receivedDate`, `vatDeductible`, `supplierAccount`, `categoryId`,
  `customFields` (`{}` when none), `original`, and top-level `rounding` / `total` / `payable` (= `totals.*`).
  `vatRecap` (`[{rate, base, vat}]`, rate desc) is set for received documents and `null` for issued ones; received
  documents also carry the usual `totals` (recap incl. `baseCzk` / `vatCzk`, `totalCzk`) and `lines: []`.
- `totals` keeps the issued convention for both directions: `totals.total` = Σ(base + vat), `totals.rounding`,
  `totals.payable` (received: the entered payable). The top-level `total` of a received document is the contract's
  Σ(base + vat) + rounding; for issued documents the top-level `total` equals `totals.total`.
- `relatedDocuments[]` / `parent` entries gain `currency` (the linked document's own; received links may differ).
- Issued draft bodies may carry `relatedDocumentId` (used only for imported drafts; ignored on native drafts, `null`
  fine), `categoryId`, `customFields`. `POST /api/documents/compute` accepts `docType: "advance_tax_doc"` with item
  lines (imported DDPP editor).

Received documents
- Dispatch: `POST` with `direction: "received"` records a received document; `PUT` / `DELETE` follow the stored
  direction. A `PUT` of a received document with another `direction` → `direction: invalid`; `docType` omitted keeps
  the stored type, a different one → `docType: invalid`.
- `POST` defaults `docType` = `invoice`, `currency` = `CZK`, `vatMode` = `standard`; `PUT` requires `currency` and
  `vatMode` (`required`). `issueDate`, `supplierNumber`, `contactId`, `payable` and `vatRecap` are always required;
  `contactId` naming no contact → `invalid`.
- `vatRecap`: at most 50 rows (`vatRecap: too_long`); `rate` 0..100, 2 dp, unique (`duplicate`); `base` / `vat`
  required, 2 dp, `|x| < 10^16`, any sign; sums that would not fit → `vatRecap: invalid`, CZK conversion overflow →
  `exchangeRate: invalid`. `rounding` 2 dp; `payable` 2 dp, `>= 0`. `vatDeductible` may be `true` in any VAT mode
  (reverse charge is self-assessed); only its default depends on the mode.
- Stored as: `status: issued`, locale = the company's default, no bank account, `paymentMethod: bank_transfer`,
  `supplier` = contact snapshot (`registration` / `vatPayer` null), `customer: null`.
- ČNB: a stored `cnb` rate is kept while `currency` and `receivedDate` are unchanged, also when the request echoes
  that rate back in `exchangeRate`; any other `exchangeRate` is a manual rate; `exchangeRate: null` after a manual rate
  fetches ČNB again. ČNB failure without a manual rate → `exchangeRate: required`.
- `relatedDocumentId` (received and imported): the target must be a non-draft document of the same direction, not the
  document itself, of the type in the table above (proforma → nothing) → else `relatedDocumentId: invalid`.
- `settle` / `credit-note` / `cancel` / `mark-sent` / `issue` on a received document → 409 `invalid_state`; payments on
  a received DDPP are refused like on an issued DDPP. Received documents can never be deducted by an `advance` line.
- An allocated number that collides (counter set below an existing number) → 409 `number_taken`; the same applies to
  native issue and to an auto-issued DDPP.

Original PDF
- 20 MB = 20 MiB (20 971 520 bytes, inclusive). The route's body limit is 20 MiB + 64 KiB; a larger body → 413
  `too_large` as well. Parts other than `file` are ignored; no `file` part → `file: required`; a malformed multipart
  body → 400 `bad_request`. Success → 204 without a body.
- Stored as `documents/{year}/{id}-original-{sha8}.pdf` (first 8 hex digits of the sha256; year = `numberYear` ??
  `issueDate` year) instead of `{id}-original.pdf`: a replacement is written under its own name inside the
  transaction, the previous file is removed only after the commit (the new one if the commit fails); a same-content
  re-upload reuses the same file. `DELETE …/original` is idempotent (204 without an original). A native document
  (draft included) → 409 `invalid_state`.
- The download filename is the document's `number` (imported drafts included).

Manual import
- `imported` omitted on `PUT` keeps the stored flag. On a native draft a non-empty `number` → `number: invalid`.
- `number` duplicates are checked against every issued document of the type, imported drafts included.
- An imported credit note is not bound to an invoice (no fixed currency / contact / rate, no per-rate cap); its rate is
  manual or ČNB like an invoice's. `advance` lines on an imported document → `lines.N.kind: invalid`. At issue a null
  `variableSymbol` is derived from the number as for native documents.
- `settle` / `credit-note` stay available on imported proformas / invoices (native follow-ups). An issued imported
  DDPP can be cancelled (it has no payment link); a native DDPP still only through its payment.
- `direction` is trimmed before `POST` picks the received or issued body.

Categories and custom fields
- Category: `active` default `true`, `position` default `0`; name uniqueness is case-insensitive per kind. Changing the
  `kind` of a category used by a document → `kind: invalid`. `categoryId` naming no category → `invalid`.
- Custom field: the wire name of the type is `type`; `appliesTo` default `both`, `active` default `true`, `position`
  default `0`; `GET` ordered by position, key. `options` on a non-select type → `options: invalid`; select:
  `options: required` / `too_long` (> 50), per option `options.N: required | too_long | duplicate`; options trimmed.
- Values: text and select trimmed (`""` = empty); number = decimal string, stored normalized (`"1.50"` → `"1.5"`);
  date strictly canonical `YYYY-MM-DD` (`2026-1-5` → `invalid`). A value sent back exactly as stored is kept without
  validation, whatever its definition is now (deactivated, deleted, recreated with another type, options changed);
  only new or changed values are validated. `null` for a stored key without an active definition removes it; stored
  keys omitted from the request are dropped. `required` is checked on every save, on `PUT …/metadata`, and again at issue against the
  current definitions.
- `PUT …/metadata` replaces all three fields (omitted = cleared). `PUT …/internal-note` only changes the note.
