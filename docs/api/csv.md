# CSV / XLSX interchange (phases 2b import, 2c export)

One tabular format, shared by the import (2b) and the accountant export (2c), so an export imports back unchanged.
One row = one document (no lines); amounts are the VAT recap per rate.

## Format

- **CSV as written by the export:** separator `;`, UTF-8 with BOM, CRLF, RFC 4180 quoting (`"` doubled), decimal
  comma without thousands separators (`1234,50`), dates `dd.mm.yyyy`, empty cell = no value.
- **What the import also accepts:**
  - Encoding: UTF-8 (BOM optional); a file that is not valid UTF-8 is decoded as Windows-1250.
  - Separator: detected from the header line (`;`, `,` or TAB, the one that yields the most known columns).
  - Numbers: decimal comma or dot; spaces / NBSP as thousands separators are ignored; `1.234,50` and `1,234.50` are
    accepted (the last `,`/`.` is the decimal mark when both occur).
  - Dates: `d.m.yyyy` (with or without spaces after the dots) and `yyyy-mm-dd`.
  - **XLSX:** the first worksheet (crate `calamine`), row 1 = header; numeric cells are numbers, date cells
    (Excel serials) are dates, text cells parse as in CSV.
  - Header names are matched case-insensitively with surrounding whitespace trimmed; unknown columns are ignored;
    column order is free. Completely empty rows are skipped.
- **Header names** are fixed machine names (English, snake_case) — they are the contract, not UI text.

### Columns

| Column | Meaning | Import |
|---|---|---|
| `direction` | `issued` \| `received` | required |
| `doc_type` | `invoice` \| `proforma` \| `credit_note` \| `advance_tax_doc` \| `debit_note` \| `advance_credit_note` \| `simplified` | required |
| `number` | issued: our number; received: our internal number | issued: required; received: ignored (a new internal number is allocated from the series) |
| `supplier_number` | received: the supplier's document number | received: required |
| `related_number` | corrections / settlement: the original's number (issued: our number, received: supplier number) | optional, linked if found |
| `issue_date` | | required |
| `tax_date` | DUZP | optional; empty → `issue_date` (proforma: always null) |
| `due_date` | | optional; empty → `issue_date` (received DDPP may stay empty) |
| `received_date` | received only | optional; empty → `tax_date` |
| `counterparty_name` | customer (issued) / supplier (received) | required except `simplified` |
| `counterparty_ico`, `counterparty_dic` | | optional |
| `counterparty_street`, `counterparty_city`, `counterparty_zip`, `counterparty_country` | country ISO alpha-2 | optional (country default `CZ`) |
| `currency` | ISO 4217 | optional, default `CZK` |
| `exchange_rate` | CZK per 1 unit, ≤ 6 dp | required when `currency` ≠ CZK, must be empty / `1` for CZK |
| `vat_mode` | `standard` \| `reverse_charge` \| `exempt` \| `non_payer` | optional, default `standard` |
| `base_{rate}`, `vat_{rate}` | VAT recap per rate, **in CZK** | see below |
| `rounding` | in the document currency | optional, default 0, \|x\| < 100 |
| `total` | total incl. VAT + rounding, **in the document currency** | required |
| `total_czk` | total in CZK | ignored on import (export only) |
| `paid_date` | date of full payment | optional |
| `variable_symbol` | digits ≤ 10 | optional |
| `vat_deductible` | received: `1`/`0`, `ano`/`ne`, `true`/`false`, `yes`/`no` | optional, default per 1e (true for `standard`) |
| `category` | category name | optional |
| `note` | issued: `headerNote`, received: `note` | optional |

**Rate columns** are dynamic: the export writes `base_{r}` and `vat_{r}` for every VAT rate in Settings → VAT rates
(active and inactive), ordered by rate descending; for rate 0 only `base_0`. `{r}` is the rate with the decimal point
written as `_` and trailing zeros dropped (`21`, `12`, `0`, `12_5`). The import recognises any `base_{r}` / `vat_{r}`
header, also rates not in Settings (as received documents allow); a `vat_{r}` without its `base_{r}` column is a file
error. A rate takes part in a row when its base or VAT cell is non-empty.

**Signs:** credit notes and DDPP corrections (`credit_note`, `advance_credit_note`) are written **negative** (all
amount columns); the import takes their absolute value. Every other type must be ≥ 0 (`invalid_amount`).

## Import — `POST /api/import/csv/preview` and `POST /api/import/csv/confirm` (2b)

- `multipart/form-data`, one part `file` (`.csv`, `.txt` or `.xlsx`, detected by content: ZIP magic → XLSX).
  `confirm` adds a text part `options` = JSON `{ selected: string[] }` (keys from the preview, status `ok` only).
  Stateless like ISDOC: the client re-sends the file with confirm, the server parses it again.
- **Limits** (as ISDOC): 50 MiB → 413 `too_large`; more than 500 data rows → 422 `{"fields":{"file":"too_many"}}`.
- **File errors** → 422 `{"code":"validation","fields":{"file":"<reason>"}}`: `invalid` (unreadable / not CSV or
  XLSX / XLSX without a sheet), `empty` (no data rows), `missing_column` (a required column is absent; `detail` names
  it, e.g. `"detail":"supplier_number"`), `invalid_column` (`vat_{r}` without `base_{r}`, or a rate header that does
  not parse; `detail` names it).
- **Entry key** = `row:{n}`, `n` = the 1-based line / sheet row number (header = 1).

### Preview response
`200 { entries: PreviewEntry[] }` in file order, the same shape as the ISDOC preview
([isdoc.md](isdoc.md#preview-response)) with `hasPdf: false` always, plus:

```
  row: number,
  field: string | null,        // the column an error refers to (camelCase-free: the CSV header name)
  categoryMatch: "existing" | "new" | null
```

### Row errors (`status: "error"`, never imported)
- `missing_field` (`field` set), `invalid_value` (`field` set; unknown direction / doc_type / vat_mode, bad
  country / currency / VS / boolean), `invalid_date`, `invalid_amount` (unparsable, ≥ 10^12, wrong sign, > 2 dp for
  amounts / > 6 dp for the rate), `no_vat_rows` (no rate column filled), `total_mismatch` (see Amounts),
  `not_allowed` (combinations the domain forbids: proforma with `tax_date`, `simplified` received without
  `counterparty_name`, VAT on a non-`standard` row, `paid_date` on an `advance_tax_doc`).

### Warnings
`contact_created`, `category_created`, `category_inactive` (an inactive category matched by name → imported without a
category), `related_not_found`.

### Duplicates (`status: "duplicate"`)
Exactly as ISDOC: issued — same docType + `number`; received — same supplier IČO (else name) + `supplier_number`;
a repeat inside the file is a duplicate of the earlier row.

### Mapping
- **Counterparty:** an existing contact matched by IČO, else by DIČ, else by exact name (case-insensitive, trimmed);
  none → a contact is created from the row (warning `contact_created`, `contactMatch: "new"`). The document snapshot
  is taken from the row as written. `simplified` without a name → no contact (as 1f-a). Our own IČO as the
  counterparty → `invalid_value` on `counterparty_ico`.
- **Category:** matched by name case-insensitively within the kind of the direction (issued `income`, received
  `expense`); none → created (active, appended last; warning `category_created`).
- **Amounts:**
  - The recap is stored in CZK as given (`baseCzk` / `vatCzk`).
  - **CZK document:** document recap = the CZK recap; `Σ(base + vat) + rounding` must equal `total` exactly, else
    `total_mismatch`.
  - **Foreign currency:** document recap per rate = `round(czk / exchange_rate, 2)` (half away from zero). The
    difference `total − rounding − Σ(base + vat)` is added to the VAT of the highest rate with VAT (else to the base of
    the highest rate) when |difference| ≤ 0.01 × the number of recap rows; larger → `total_mismatch`.
  - `payable` = `total` (imports carry no advance deductions); stored rate source `manual`.
- **Issued rows:** created `imported: true`, status `issued`, number from the row, no number allocation, no PDF render,
  no lines (recap-only, like an ISDOC import without lines); supplier snapshot = current company; bank account = the
  default one of the currency (none → null); payment method `bank_transfer`; locale = the company default.
- **Received rows:** internal number from the received series of the type; `receivedDate`, `vatDeductible`,
  `supplierAccount` null.
- **Related document:** `related_number` resolved as in ISDOC (same direction, type allowed by the 1f-a link table,
  rows earlier in the file count; originals are imported first); not found → warning, no link.
- **`paid_date`:** one payment of `payable` on that date (credit notes: a refund), skipped when `payable` is 0;
  an imported proforma payment never creates a DDPP.
- Imported documents relax the 1e rules (caps, `correctionReason`) as ISDOC does.

### Confirm
Each selected `ok` row in its own transaction, response as ISDOC:
`200 { results: [{ key, status: "imported" | "skipped" | "failed", documentId, error }] }`.

### Sample — `GET /api/import/csv/sample`
`200 text/csv` (`Content-Disposition: attachment; filename="import-sample.csv"`), written exactly as the export:
the header with the current rate columns and three example rows (an issued CZK invoice, a received EUR invoice with a
rate, an issued credit note with negative amounts), using fictitious parties.

## UI (2b)
Issued and received list → "Import CSV / XLSX" (next to the ISDOC import): file picker, "Download sample", preview
table (row, status, direction, type, number, counterparty + contact match, dates, total, category match, error /
warnings with the column), select all `ok`, Confirm, result table with links. Reuses the ISDOC import components where
they fit.

## Export (2c)

Written exactly in the [Format](#format) above (the writer behind the sample): `;`, UTF-8 BOM, CRLF, decimal comma,
`dd.mm.yyyy`, every column of the table in its order, rate columns from Settings → VAT rates (plus any other rate that
occurs in the exported documents, so no amount is ever dropped). Drafts and cancelled documents are **never**
exported. Rows are ordered by direction (issued first), tax date (issue date when none), number.

### `GET /api/export/csv?{list filters}` — the list export
- The same filters as `GET /api/documents` (`direction` required, `docType`, `status`, `paymentState`, `overdue`,
  `contactId`, `q`, `from`, `to`, `categoryId`, `imported`, …; `limit` / `offset` ignored) → what the list shows,
  minus drafts and cancelled. A proforma is included when the filter includes it (e.g. its tab).
- Filename `doklady-{issued|received}-{yyyy-mm-dd}.csv` (today).

### `GET /api/export/accountant?from=YYYY-MM-DD&to=YYYY-MM-DD&direction=issued|received|both` — for the accountant
- Documents whose **tax date** (DUZP; received: tax date, else received date) is in `[from, to]` (both required,
  `from ≤ to`, at most 366 days apart → else 422 `from`/`to` `invalid`). `direction` default `both`.
- Every type **except proforma** (not a tax document); drafts and cancelled excluded.
- Filename `ucetni-{from}-{to}.csv`.
- `format=csv` (default) | `pohoda` (3b, [accounting.md](accounting.md)); anything else → 422 `format: invalid`.

### Both
- `200 text/csv; charset=utf-8`, `Content-Disposition: attachment`. More than 10 000 matching documents →
  422 `{"fields":{"filter":"too_many"}}` (checked before the file is built). An empty result is a header-only file.
- **Values per column:**
  - `number`: our number (received: the internal number), `supplier_number`: received only.
  - `related_number`: the linked original (issued: its number; received: its supplier number), else empty.
  - Dates as stored; `received_date` received only.
  - Counterparty = the document's snapshot (issued: customer, received: supplier); a contactless simplified document
    leaves them empty.
  - `currency`; `exchange_rate` only for a foreign currency (≤ 6 dp, trailing zeros dropped).
  - `base_{r}` / `vat_{r}`: the stored recap **in CZK** (CZK document: the recap itself; foreign: `baseCzk` / `vatCzk`).
    A final invoice exports its stored recap as is (the advance deductions included in it).
  - `rounding`, `total` in the document currency; `total_czk` = `totalCzk` (CZK document: `total`).
  - `paid_date`: the date of the payment that made the document fully paid (`paymentState` `paid` / `overpaid`),
    else empty — so a partially paid document exports unpaid.
  - `variable_symbol`, `vat_deductible` (received: `1`/`0`; issued empty), `category` (name), `note` (issued
    `headerNote`, received `internalNote`).
  - Credit notes and DDPP corrections: every amount column negative (as stored positive × −1).
- **Round trip:** importing an export reproduces each document's direction, type, numbers, dates, counterparty, VAT
  mode, CZK recap, totals, category and full payment (a final invoice comes back with `payable` = `total`, without its
  deduction link; partial payments are not exported).

### UI (2c)
- Issued and received lists: "Export CSV" button next to the ISDOC export → downloads the current list filter
  (tab + filters, no paging); `too_many` explained.
- "Export pro účetní" (on both lists): a small dialog — period (from / to, prefilled with the previous calendar
  month), direction (both / issued / received) → downloads `GET /api/export/accountant`.

## Clarifications (as implemented)
Additive details settled during the Phase 2b backend. No field of the contract changed shape.

Upload and file errors
- Exactly one `file` part: none → 422 `file: required`, more → `file: invalid`. Confirm `options` missing or malformed
  → 422 `options: invalid`; `selected` defaults to `[]`. File errors carry `detail` next to `fields`:
  `{"code":"validation","fields":{"file":"missing_column"},"detail":"supplier_number"}`.
- **`missing_column`** is raised when a data row needs an absent column: `direction`, `doc_type`, `issue_date`,
  `total` always, `number` for issued rows, `supplier_number` for received rows, `counterparty_name` for every
  type but `simplified`, `exchange_rate` for foreign currencies. The first such row (file order) names the column.
  A column that no row needs may be absent.
- **`invalid_column`** also covers a known header that appears twice, and a rate given twice (`base_21` +
  `base_21_0`). A rate header accepts `_`, `.` or `,` as the decimal mark (0–100, ≤ 2 dp). `vat_0` is accepted.
  Duplicate unknown headers are ignored. A header-only file → `empty`. More than 50 rate columns (the recap rows a
  received document holds, `MAX_RECAP_ROWS`) → `invalid_column` naming the 51st `base_{r}` in header order.
- **XLSX:** more than 200 MiB actually decompressed (zip bomb) → 422 `file: too_large`. A ZIP with more than
  10 000 entries, or one that is no workbook → `invalid`. The first sheet is read cell by cell: the used range it
  declares is never allocated (A1 + XFD1048576 is a few KB). Only non-empty cells count. A cell right of column 200
  → `invalid`. More than 500 rows after the header → `too_many`. The header is the first row with a non-empty cell.
  Keys and `row` use the real sheet row numbers.
- CSV line numbers are the line where a record starts. Blank lines and lines inside quoted cells count. CRLF and
  lone CR are read as line ends.

Values
- **Encoding:** a leading UTF-8 BOM is dropped before the Windows-1250 fallback decodes the rest.
- **Numbers:** a leading `+`, `-` or `−` is allowed. A single mark that occurs twice (`1.234.567`) is invalid.
  Trailing zeros do not count towards the dp limit (`1,500` = 1.5). XLSX numeric cells are taken at Excel's 15
  significant digits, so binary noise disappears (`0.1+0.2` → `0.3`, `1234.5` → `1234.50`).
- **XLSX cell types:** a number in a date column → `invalid_date`; a date in an amount column → `invalid_amount`.
  A numeric IČO is left-padded to 8 digits. A boolean cell reads as `true` / `false`. A number in a text column is
  written without trailing zeros.
- **`invalid_value`** also covers:
  - a bad IČO checksum or DIČ format;
  - `exchange_rate` other than empty / `1` for CZK;
  - text over its limit: `number` / `supplier_number` / `related_number` 40, `counterparty_name` 200,
    `counterparty_street` 200, `counterparty_city` 100, `counterparty_zip` 20, `category` 100, `note` 2000.
- **`invalid_amount`** also covers a foreign `exchange_rate` ≤ 0, and a conversion that would not fit a money
  column (reported on `exchange_rate`). `|rounding|` must be < 100.
- **Signs:** a `credit_note` / `advance_credit_note` with a negative `total` is negated as a whole, rounding
  included. A positive one is taken as is. After that every `base_*`, `vat_*` and `total` must be ≥ 0, else
  `invalid_amount` on the first offending column (mixed signs). `rounding` may have either sign in every type.
- Ignored by direction: issued rows ignore `received_date` and `vat_deductible`; received rows ignore `number`.

Mapping
- **Totals:** stored `total` = Σ(base + vat), `rounding` beside it, `payable` = the `total` column (as received
  documents store them). CZK documents keep no `baseCzk` / `vatCzk` / `totalCzk`, like every CZK document. Foreign
  ones store the CZK recap as given and `totalCzk` = `round2(total × rate)`, as a manual received document does.
  The difference goes to the first candidate that stays ≥ 0: the derived VAT of each rate with VAT ≠ 0, then each
  base, highest rate first. None qualifies → `total_mismatch`. A recap amount never turns negative. Example: EUR
  at 25, `base_21` 2500, `vat_21` 0,20, `base_12` 250, `vat_12` 0,10, `total` 109,99 → 21 %: 99,98 + 0,01;
  12 %: 10,00 + 0,00.
- **Counterparty match:**
  - A candidate whose IČO or DIČ contradicts the row's is never matched. So a row with an IČO matches by DIČ or
    name only contacts without an IČO.
  - The name compare is `lower(btrim(name))`. The earliest created contact wins.
  - A created contact takes the row's name, IČO, DIČ, address and country. Repeats of a new party in one file
    create it once (later rows match it in their own transaction).
- **Snapshots:** an issued document's supplier snapshot is the whole company profile, incl. registration and
  contact lines. A received supplier snapshot is the row party with `vatPayer` null, as for a manual received
  document. Both directions take the company's default locale.
- **Default `vat_mode`:** `standard`, except for issued rows when the company is not a VAT payer: `non_payer`, as for a
  native issued document. VAT on such a row → `not_allowed`.
- Issued rows: `variableSymbol` only from the row (never derived from the number), no constant symbol, an issued
  DDPP without `due_date` gets the issue date. `due_date` before `issue_date` is accepted (import).
- **Category:** an inactive match → `categoryMatch: null` + `category_inactive`. A new name repeated in the file
  shows `new` / `category_created` on every row; it is created once.
- **Preview:** an error row still shows `direction`, `docType`, `number` (issued: `number`, received:
  `supplier_number`) and `counterparty` when those cells are readable; the rest is null. Warnings are listed
  category first, then `contact_created`, then `related_not_found`.
- **Confirm** runs on the ISDOC pipeline (now `src/import/`):
  - rank order (originals first);
  - one transaction per row with the advisory lock and the duplicate re-check;
  - failure codes `duplicate`, `number_taken`, `storage_unavailable`, `internal` (CSV rows carry no PDF, so the
    space's storage quota never refuses them);
  - the database lookups only for the selected rows (one query per kind for the whole file, as in the preview).
- **Sample:** the rate columns are Settings → VAT rates plus 21 % (the rate the example rows use). Dates are fixed in
  2026. Categories are `Služby` (issued) and `Software` (received). The parties are IČO 12345679 / 87654326.
  `Content-Type: text/csv; charset=utf-8`. The sample imports back without errors for any company whose IČO
  differs from the sample parties.

### Export (2c, as implemented)
Additive details settled during the Phase 2c backend. No field of the contract changed shape.

- **Statuses:** only `issued` documents are exported (the status filter of the list still applies, so
  `status=draft` / `status=cancelled` give a header-only file).
- **List export `direction`:** absent → 422 `direction: required`; anything but `issued` / `received` → `invalid`
  (`both` is accountant-only). Other filters behave exactly as in `GET /api/documents`.
- **Accountant query:** `from` / `to` absent or not `YYYY-MM-DD` → that field `invalid`; `from > to` or more than
  366 days apart → both `from` and `to` `invalid`; `direction` other than `issued` / `received` / `both` →
  `direction: invalid`. Errors of several fields are reported together.
- **Order:** direction, then the tax date as the accountant period uses it (DUZP, received: else the received
  date), else the issue date, then `number_year`, `number_seq` (allocated numbers in series order), then the number
  text (imported numbers), then id.
- **One snapshot:** the id list, the rate columns of the header and every row are read in one read-only
  `REPEATABLE READ` transaction; the whole file is built inside it and sent after the commit, so documents
  edited, cancelled or re-dated meanwhile are exported as they were when the export started, and no recap rate is
  ever missing from the header.
- **Rows** are loaded and written 500 documents at a time. A document whose row cannot
  be built (an undecodable counterparty snapshot, an unknown stored value, a foreign recap row with neither a
  stored CZK amount nor an exchange rate) is **skipped and logged** with its id; the rest of the file is written.
  A database failure while building fails the request (500 `internal`, logged) — no partial file is sent.
- **Formula guard (CSV injection):** a free-text cell (`number`, `supplier_number`, `related_number`, the
  counterparty cells, `variable_symbol`, `category`, `note`) that starts with `=`, `+`, `-`, `@`, TAB or CR —
  also behind leading `'`s — is written with one `'` in front. Amount, date, rate and code cells are never
  guarded. The import (2b, CSV and XLSX text cells) drops one leading `'` when such a character follows (after
  further `'`s), so guarded cells import back unchanged; a hand-written `'=x` imports as `=x`.
- **Values:**
  - `total` = stored `total` + `rounding` (= `payable` without advance deductions); `total_czk` of a CZK document =
    that `total`.
  - Foreign recap rows without a stored `baseCzk` / `vatCzk` (none are written today) fall back to
    `round2(amount × rate)`; without a rate either the document is skipped (above).
  - `paid_date`: payments ordered by date (then creation); the first one at which their running sum reaches
    `payable`. A document settled without any payment (payable 0, e.g. fully covered by advances) → empty.
  - `related_number` is the document `relatedDocumentId` points at, whatever its type (a final invoice → its
    proforma; a DDPP → its proforma).
  - The counterparty address cells come from the snapshot; `counterparty_country` is written for every party.
- **Headers:** both exports and `GET /api/import/csv/sample` send `Cache-Control: no-store` (shared download
  helper, as the ISDOC export).
- **Round trip:** besides the received internal number (allocated anew), an empty `received_date` comes back as
  the tax date and an empty `due_date` as the issue date (the import defaults).
