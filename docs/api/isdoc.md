# API — ISDOC import and export (phase 1f-b)

ISDOC 6.0.2 (`http://isdoc.cz/namespace/2013`). This builds on the 1e manual import and the 1f-a document types.
Fixtures under `tests/fixtures/isdoc/` must be anonymized: no real IČO, DIČ, IBAN or names.

## Document type mapping (both directions)
| ISDOC `DocumentType` | docType |
|---|---|
| 1 | `invoice` |
| 2 | `credit_note` |
| 3 | `debit_note` |
| 4 | `proforma` |
| 5 | `advance_tax_doc` |
| 6 | `advance_credit_note` |
| 7 | `simplified` |

## Import — `POST /api/import/isdoc/preview` and `POST /api/import/isdoc/confirm`

### Upload
Both endpoints take `multipart/form-data` with one or more `files` parts (`.isdoc`, `.isdocx` or `.zip`).
`confirm` additionally takes a JSON part `options`:

```
{
  selected: string[],          // entry keys from the preview (status "ok" only)
  markPaid: boolean,           // default true
  categoryId: uuid | null,     // received only; an expense category, as in 1e
  vatDeductible: boolean       // received only; default true
}
```

The server keeps nothing between preview and confirm. The client re-sends the same files with `confirm`, and the
server parses them again.

### Limits
- 50 MiB total upload. Over the limit → 413 `too_large`.
- At most 500 documents. More → 422 `{"fields":{"files":"too_many"}}`.
- ZIPs are unpacked recursively, at most 3 levels deep.
- At most 200 MiB unpacked in total. Over the limit → 422 `{"fields":{"files":"too_large"}}`.

### Archive entries
- Inside a zip, `.isdoc`, `.isdocx` and `.zip` entries are processed; anything else is ignored.
- An `.isdocx` is a ZIP holding the ISDOC XML (the `*.isdoc` entry) and optionally a PDF. When it has exactly one PDF
  (or a `manifest.xml` names the main PDF), that PDF becomes the document's original (`documents/{year}/…`, ≤ 20 MiB
  as in 1e). A larger PDF is skipped, with a warning.

### Entry key
An entry key is the path inside the upload, e.g. `invoices.zip/2026.zip/faktura_1.isdoc`. Every document has a stable
key, so the client can echo it back in `selected`.

### Preview response
`200 { entries: PreviewEntry[] }`, in upload order:

```
PreviewEntry {
  key: string,
  status: "ok" | "duplicate" | "error",
  error: string | null,          // code, see below; set when status = "error"
  warnings: string[],            // codes, see below
  direction: "issued" | "received" | null,
  docType: string | null,
  number: string | null,         // ISDOC ID (issued: our number; received: supplier number)
  counterparty: { name: string, ico: string | null } | null,
  contactMatch: "existing" | "new" | null,
  issueDate, taxPointDate, dueDate,   // YYYY-MM-DD | null
  currency: string | null,
  total: string | null,          // the ISDOC's LegalMonetaryTotal/TaxInclusiveAmount (document currency)
  hasPdf: boolean,
  relatedNumber: string | null,  // OriginalDocumentReference ID
  relatedFound: boolean          // resolved in the DB or in this batch
}
```

### Error codes (an error entry is never imported)
- `invalid_xml`: not well-formed, or the root is not an ISDOC `Invoice`.
- `unsupported_version`: a version other than 6.x.
- `unsupported_type`: DocumentType outside 1–7.
- `foreign`: our company IČO matches neither the supplier (`AccountingSupplierParty`) nor the customer
  (`AccountingCustomerParty`).
- `ambiguous`: it matches both.
- `missing_field`: a required element is missing (ID, IssueDate, currency or the totals).
- `invalid_amount`: an amount cannot be parsed, or is ≥ 10^12.

### Warning codes
- `related_not_found`
- `pdf_skipped`
- `rate_from_cnb`: a foreign currency with no usable rate in the ISDOC.
- `contact_created`
- ~~`paid_ignored`~~: dropped (see Clarifications); `PaidAmount` is still never used.

### Direction
- Our company's IČO equals the supplier's IČO → `issued`.
- It equals the customer's IČO → `received`.
- IČO values are compared with whitespace stripped.

### Duplicates (status `duplicate`, never imported)
- **Issued:** an issued document of the same docType with the same `number` already exists (drafts included).
- **Received:** a received document with the same supplier IČO (contact snapshot) and the same `supplierNumber`
  already exists. With no supplier IČO, the match is on supplier name + `supplierNumber`.
- A second identical key in the same batch is also a `duplicate` of the first.

### Field mapping
- **Dates:** `IssueDate` is the issue date. `TaxPointDate` falls back to `IssueDate` when missing, and is null for a
  proforma. `PaymentMeans/Payment/Details/PaymentDueDate` is the due date; missing → `issueDate`. A received
  `advance_tax_doc` may have no due date.
- **Currency:**
  - `ForeignCurrencyCode` set → the document currency is that code. The rate is `CurrRate / RefCurrRate` (≤ 6 dp),
    stored as `manual`, provided the rate is > 0 and ≠ 1.
  - Otherwise ČNB at the tax point date (received: the received date), with warning `rate_from_cnb`. ČNB down at
    confirm → that entry fails with `rate_unavailable`; other entries still import.
- **Amounts:**
  - `vatMode`: every `TaxSubTotal` at 0 % with `VATApplicable` false → `non_payer` for issued documents, `exempt`
    for received ones. Otherwise `standard`.
  - Reverse charge is detected from `VATApplicable` false with a non-zero rate, or from a note containing
    "přenesen", and maps to `reverse_charge`.
  - The VAT recap is copied verbatim from `TaxTotal/TaxSubTotal` (`TaxableAmount`, `TaxAmount`, `Percent`). In ISDOC
    the plain elements are in the local currency (CZK) and the `…Curr` elements are in the foreign currency. So for a
    foreign-currency document the document amounts come from `…Curr` and `baseCzk`/`vatCzk` from the plain elements.
    The same applies to the totals.
  - Totals are copied verbatim from `LegalMonetaryTotal`.
  - Rounding is `PayableRoundingAmount`, and `payable` is `PayableAmount`.
  - Credit notes and DDPP corrections store positive amounts (the absolute value of what the ISDOC carries).
- **Lines:**
  - Each `InvoiceLine` becomes an `item`: `Item/Description`, `InvoicedQuantity` (+ `unitCode` → unit),
    `UnitPrice`, `ClassifiedTaxCategory/Percent`, discount 0.
  - A line with no amounts becomes `text`.
  - The lines are **stored only**. The recap and totals above are never recomputed from them.
- **Counterparty:**
  - An existing contact with the same IČO is used. Otherwise a contact is created from the ISDOC party (name, IČO,
    DIČ, address, country; no ARES), with warning `contact_created`.
  - The snapshot is taken from the ISDOC party data as written, not from the contact.
- **Bank:**
  - Issued: the company bank account whose IBAN or account number matches `PaymentMeans/…/ID` / `IBAN`; else null.
  - Received: `supplierAccount` is the IBAN or account number.
- **Payment fields:** `VariableSymbol`, `ConstantSymbol`; `PaymentMeansCode` 42 → `bank_transfer`, 10 → `cash`,
  48 → `card`, otherwise `other`.
- **Note and locale:** `Note` becomes `headerNote` (issued) or `note` (received). Locale `cs`.
- **Related document:**
  - `OriginalDocumentReference/ID` is resolved against a non-cancelled document of the same direction, of a type
    allowed by the 1f-a link table:
    - issued: by `number`;
    - received: by `supplierNumber` + the same supplier IČO.
  - Documents earlier in the same batch count as well; the import is ordered so that originals come first.
  - Not found → no link, with warning `related_not_found`.

### Confirm
- Each selected `ok` entry is imported in its own transaction.
- **Issued entries:**
  - Created `imported: true`, already `issued`, with the number from the ISDOC.
  - They take no number from a series and get no PDF render; the snapshots come from the ISDOC.
- **Received entries:**
  - Created with an internal number from the received series of their type.
  - `receivedDate` = the tax point date. `categoryId` and `vatDeductible` come from `options`.
- **`markPaid`:**
  - One payment of `payable` on the due date (issue date if none), for payable types: invoice, debit_note,
    proforma, simplified, plus credit_note and advance_credit_note as refunds. A DDPP gets none.
  - Skipped when `payable ≤ 0`.
  - An imported proforma payment never creates a DDPP (1e).
- **Rules relaxed for imported documents (as in 1e):** caps and the `correctionReason` requirement are not enforced.
- **Response:** `200 { results: [{ key, status: "imported" | "skipped" | "failed", documentId: uuid|null,
  error: string|null }] }`.
  - `skipped`: a key that was not selected, or that is no longer `ok` (e.g. it became a duplicate meanwhile).
  - `failed`: carries an error code; anything internal is logged and returned as `internal`.

## Export
### `GET /api/documents/{id}/isdoc`
- Only for an issued (non-draft) document with direction `issued`, cancelled ones included. Otherwise → 409
  `invalid_state`; a received document → 404.
- The response is an `.isdocx` (`application/zip`). The ZIP holds:
  - `{number}.isdoc`, the XML;
  - `{number}.pdf`, which is the archived PDF for native documents (rendered lazily, as for a DDPP in 1d) and the
    original for imported ones;
  - `manifest.xml`, per the ISDOC isdocx spec, naming both files.
- An imported document without an original, or a native one whose PDF cannot be rendered (mdcast down), gets a
  plain `.isdoc` (`application/xml`) instead. The one 5xx: the file storage is unreachable → 503
  `{"code":"storage_unavailable"}` (no export without its stored PDF).
- Filename: `{number}.isdocx` / `{number}.isdoc`.

### `GET /api/documents/isdoc?{list filters}`
- Takes the same filters as `GET /api/documents`, with `direction=issued` forced. Drafts are always excluded.
- The response is a ZIP of the per-document files described above (`isdoc-export.zip`).
- More than 1000 matching documents → 422 `{"fields":{"filter":"too_many"}}`.
- A native document whose archive is missing and cannot be rendered is exported as plain `.isdoc`.
- The storage unreachable → 503 `{"code":"storage_unavailable"}` at the first document that needs it (no partial ZIP).

### XML content
- **Header:**
  - `DocumentType` from the mapping table; `UUID` = document id; `ID` = number; `IssueDate`.
  - `TaxPointDate` (omitted for a proforma).
  - `VATApplicable` = the supplier snapshot's `vatPayer`.
  - `ElectronicPossibilityAgreementReference` is empty.
  - `LocalCurrencyCode` CZK.
  - `ForeignCurrencyCode` + `CurrRate` / `RefCurrRate` (1) when the currency is not CZK.
- **Parties:** `AccountingSupplierParty` / `AccountingCustomerParty` from the snapshots. A simplified document
  without a customer omits the customer party.
- **Lines:**
  - `item` lines are written with the full price breakdown.
  - `text` lines become `InvoiceLine` entries with zero amounts.
  - `subtotal` lines are omitted (the members are exported).
  - `advance` lines are written as `NonTaxedDeposits` / `TaxedDeposits` per the ISDOC spec.
- **Totals:** `TaxTotal` from the stored recap (with the CZK amounts for foreign currency). `LegalMonetaryTotal` from
  the stored totals; `PayableRoundingAmount` = rounding.
- **Payment:**
  - `PaymentMeans`: due date, VS, KS, and the IBAN + BIC / account number from the bank snapshot.
  - The `PaymentMeansCode` reverse map, with `other` → 97.
- **References and notes:**
  - `OriginalDocumentReference` for a credit note, debit note or DDPP correction, giving the original's number and
    issue date.
  - `Note` = header note; the correction reason goes into `Note` as well.
- **Signs:** credit notes and DDPP corrections are exported with the sign convention the ISDOC 6.0.2 spec prescribes
  for document types 2 and 6. Check it against the spec text and record the result in Clarifications. On import, the
  absolute values are stored.
- **Validation:** the output must validate against the ISDOC 6.0.2 XSD. Tests check this with a vendored copy of the
  schema in `tests/fixtures/isdoc/schema/`, if its licence allows; otherwise with structural assertions.
- **Round trip:** import(export(doc)) must reproduce the totals, the recap, the lines (items/text), the dates, the
  parties' IČO and the number. A test checks it on our own exports.

## Frontend
- **Import page:** opened from the issued and received lists by an "Import ISDOC" button.
  - A drop zone takes the files.
  - Preview table: status, direction, type, number, counterparty (with a new/existing badge), dates, total, PDF,
    warnings.
  - Checkboxes, with the `ok` rows selected by default.
  - Batch options: mark as paid (on); category and VAT deductible for received documents.
  - Confirm, then a result table with links to the imported documents.
- **Detail:** a "Stáhnout ISDOC" button on issued non-draft documents.
- **Lists:** "Exportovat ISDOC" on the issued list exports the current filter as one ZIP.

## Clarifications 1f-b (as implemented)
Additive details and spec research settled during the Phase 1f-b backend.

ISDOC 6.0.2 spec findings
- **Signs (types 2 and 6):** the spec (appendix A.6, binding since 5.2.1) says a credit note is *always* written with
  positive amounts; its type says that they subtract. Export therefore writes credit notes and DDPP corrections
  positive, as stored. On import, a type 2 / 6 document whose `TaxInclusiveAmount` is negative is negated **as a
  whole** (recap, totals, rounding, line prices), so mixed-sign lines keep their relation; a positive one is taken
  as is.
- **`.isdocx` manifest:** the manifest schema (`isdoc-manifest-6.0.2.xsd`) holds exactly one `maindocument
  filename="…"` — it cannot name the PDF. The PDF is named inside the ISDOC by
  `SupplementsList/Supplement[@preview="true"]` (`Filename`, `DigestMethod`
  `http://www.w3.org/2001/04/xmlenc#sha256`, base64 `DigestValue`). Export writes both; import picks the PDF the
  `preview` supplement names, else the only PDF of the archive — chosen from the entry names, so only that one PDF
  is decompressed and counted toward the unpacked limit. Without a manifest the root-level `.isdoc` is the
  main document (spec 3.3.1 backward compatibility), else the first `.isdoc` anywhere.
- **Deposits:** an `advance` line deducting a DDPP becomes one `TaxedDeposits/TaxedDeposit` per rate (DDPP number,
  its VS, amounts, rate) and the DDPP amounts are `AlreadyClaimed…` in `TaxSubTotal`; the non-payer form (deducting a
  proforma) becomes one `NonTaxedDeposits/NonTaxedDeposit` and `PaidDepositsAmount`. `TaxableAmount` = the stored
  (net) recap + deducted, `Difference…` = what this document taxes (A.6: `DifferenceTaxInclusiveAmount +
  PayableRoundingAmount − PaidDepositsAmount = PayableAmount`).
- **`PaymentMeansCode`:** `bank_transfer` 42, `cash` 10, `card` 48, `other` 97 (the schema enum: 10, 20, 31, 42, 48,
  49, 50, 97). Import: 42 / 10 / 48 as listed, anything else (and no `PaymentMeans`) → `other`.
- **Local currency elements:** `UnitPrice`, `LineExtensionAmountBeforeDiscount`, deposits and every plain amount are
  in the local currency (CZK); a foreign-currency document carries the `…Curr` twins.
- **Schema validity:** `tests/fixtures/isdoc/schema/` vendors `isdoc-invoice-6.0.2.xsd` and
  `isdoc-manifest-6.0.2.xsd` (the copyright notice permits copying with the notice kept). Integration tests run
  `xmllint --schema` on real exports (CZK, EUR, credit note, plain `.isdoc`) and on the manifest, so `xmllint`
  (`libxml2-utils`) is needed for `make test-integration` (CI installs it).

Import
- **Recap and totals** come from the `Difference…` elements (`DifferenceTaxableAmount` / `DifferenceTaxAmount`,
  `DifferenceTaxExclusiveAmount` / `DifferenceTaxInclusiveAmount`) instead of the gross `TaxableAmount`: they are
  equal without taxed deposits, and with them the difference is what the document taxes, which matches our stored
  net recap (round trip). Preview `total` stays `TaxInclusiveAmount`. `totalCzk` = plain `PayableAmount`.
- **Non-taxed deposits:** `PaidDepositsAmount` (VAT-free, e.g. a paid proforma without a DDPP) is subtracted from the
  0 % recap row (added with a negative base when missing — as a non-payer's `advance` line deducts a proforma) and
  from the base and total, so `payable = total + rounding` holds and import(export(doc)) reproduces our stored
  recap, total and payable.
- **CZK amounts after a save:** a received `PUT` that changes neither the recap rows (rate, base, VAT), the rounding,
  the payable nor the exchange rate keeps the stored CZK amounts (recap `baseCzk` / `vatCzk`, `totalCzk`), so an
  imported document keeps the supplier's CZK amounts through a no-op save. Any change recomputes them as for every
  received document (`round2(x × rate)`, `totalCzk` from the payable). A manually entered document already stores
  exactly those values, so for it nothing changes.
- **`paid_ignored` is not emitted:** every producer (we included) writes `PaidAmount`, so the warning was noise. The
  amount stays ignored; export keeps writing `PaidAmount` = payable (the amount to pay).
- **Local currency:** a `LocalCurrencyCode` other than CZK → entry error `unsupported_currency` (the plain amounts
  would not be CZK).
- **VAT mode** (first match): a `LocalReverseChargeFlag`, `VATApplicable` false at a non-zero rate, or a note with
  "přenesen" → `reverse_charge`; every row 0 % with `VATApplicable` false → `non_payer` (issued) / `exempt`
  (received); `VATApplicable` true and zero VAT on every row while some row's `round2(base × rate / 100)` is not
  zero → `exempt` (how our own export writes `exempt`); else `standard` (VAT rounding to 0, zero-value documents). Export writes `VATApplicable` true for standard / exempt, false + `LocalReverseChargeFlag` for reverse
  charge, false for non-payer.
- **Lines:** a line with no quantity (or 0) and zero `LineExtensionAmount` and `UnitPrice` is `text`. CZK: unit price
  = `UnitPrice`; a `LineExtensionAmountBeforeDiscount` above the base gives the discount % (2 dp). Foreign currency:
  unit price = `LineExtensionAmountCurr` ÷ quantity (4 dp), no discount. ISDOC 6.0.2 has no document-currency unit
  price or before-discount amount (`UnitPrice` and `LineExtensionAmountBeforeDiscount` have no `…Curr` twin), so a
  discounted foreign-currency line round-trips at its net unit price with 0 % (299.99 at 15 % → 254.99 at 0 %): the
  line base, recap and totals are unchanged. Export writes no before-discount amount for such a line. Quantity / price 4 dp, unit = `unitCode`
  (≤ 20 chars). Lines are stored in **both directions** and are display-only: the recap and totals still come
  from the ISDOC and are never recomputed. `Document.lines` of a received document imported from ISDOC returns
  them (every other received document keeps `lines: []`); a received `PUT` carries no lines and leaves them
  untouched. A line whose base `quantity × unitPrice` would not fit a money column → `invalid_amount`; a
  `non_payer` document stores its item lines at rate 0.
- **Parties:** IČO = `PartyIdentification/ID` without whitespace; DIČ = the first non-empty
  `PartyTaxScheme/CompanyID`; street = `StreetName` + `BuildingNumber`; country = `IdentificationCode` (else `CZ`);
  registration = `RegisterIdentification/Preformatted` (else `RegisterKeptAt RegisterFileRef`); phone / e-mail from
  `Contact`. A party without an IČO matches (and the duplicate rule compares) a contact with no IČO and exactly the
  same name; otherwise a new contact is created. Supplier snapshot `vatPayer` = header `VATApplicable`.
- **Received fields:** `note` is the document's `internalNote` (received documents have no header note);
  `receivedDate` = tax point (issue date for a proforma); `supplierAccount` = IBAN else `account/bankCode`
  (≤ 60); locale `cs`; `bankAccountId` none. Issued: `bankSnapshot` = the ISDOC payment details,
  `roundingMode` derived from the amounts (4e, [defaults.md](defaults.md)). VS / KS kept only when they are digits (≤ 10 / ≤ 4).
- **Rates:** `CurrRate / RefCurrRate` with a positive `RefCurrRate` (default 1), 6 dp, when a `ForeignCurrencyCode`
  differs from CZK.
- **More entry error codes:** `unsupported_currency` (see above), `invalid_archive` (an unreadable `.zip`), `too_deep` (a zip nested deeper than 3
  levels, the top-level upload being level 1; an `.isdocx` is not a level). `missing_field` also covers an `ID`
  longer than 40 characters, an unparsable required date and a non-ISO currency. An `.isdocx` without an ISDOC →
  `invalid_xml`. DTDs are refused (`invalid_xml`). `invalid_amount` also covers a VAT rate outside 0–100 or with
  more than 2 dp.
- **Batch:** a `.zip` holds at most 10 000 entries (else `invalid_archive`). A later document with the same identity
  as an earlier one (issued: type + number; received: supplier IČO or name + number) is a `duplicate` and keeps
  its own key. Two entries with the same path (e.g. two uploads of the same name) get unique keys: the later one is
  suffixed `#2`, `#3`, …
- **Upload:** the 50 MiB counts the `files` parts; the route body limit is 50 MiB + 1 MiB of multipart overhead (also
  → 413 `too_large`). No `files` part → 422 `files: required`. A top-level file that is neither `.zip` nor `.isdocx`
  is read as ISDOC XML; inside a zip only `.isdoc` / `.isdocx` / `.zip` count. "Unpacked" counts the bytes
  actually decompressed from archives, never the header sizes.
- **Confirm `options`:** a multipart **text** part holding JSON (no content type needed). Missing or malformed → 422
  `{"fields":{"options":"invalid"}}`; `selected` defaults to `[]`. `categoryId` must name an active expense category
  (else `categoryId: invalid` / `inactive`). Entries are imported in rank order (proforma → invoice / simplified /
  DDPP → corrections), so a selected original is committed before a document that links to it; results are listed
  in upload order. Database lookups (existing duplicate, contact, related) run only for the selected entries; the
  in-batch duplicate pass covers all. Each entry's transaction takes `pg_advisory_xact_lock` on its identity
  (direction + type / supplier + number) and re-checks the duplicate rule, so two concurrent confirms cannot import
  the same received document twice (no unique index backs supplier + supplier number). A duplicate found then is
  `failed` with `duplicate`. Failure codes: `rate_unavailable`, `storage_unavailable` (the file storage is down
  while storing the entry's original PDF; the entry's transaction is rolled back, other entries are still tried),
  `duplicate`, `number_taken` (the received series counter collides), `quota_exceeded` (the entry's PDF no longer
  fits the space's storage quota — the batch was checked up front, see below), `internal`.
- **Storage quota** ([spaces.md](spaces.md#limits)): before anything is imported, the PDFs of all selected `ok`
  entries must fit the space's storage quota, else 422 `{"code":"quota_exceeded","detail":"<used> / <limit> MB"}`
  and nothing is imported. Entries without a PDF add nothing.
- **`markPaid`:** the payment is dated `dueDate ?? issueDate`; types: all but `advance_tax_doc`, `payable > 0`.

Export
- `{number}` in file names is reduced to `[A-Za-z0-9._-]` (as the PDF download). The single export of a received
  document → 404, of a draft → 409 `invalid_state` (checked in that order).
- When the PDF cannot be obtained (mdcast down, archive object missing, no original) the export is a plain `.isdoc`;
  the failure is logged. An unreachable storage is the exception: the single export is 503 `storage_unavailable`,
  and the bulk export stops at the first such document with the same 503 (no partial ZIP).
- A simplified document without a customer writes `AnonymousCustomerParty` (`ID` `anonymous`, empty `IDScheme`):
  the schema requires a customer party, so it cannot be omitted. Import reads its absence as "no customer".
- Header: `IssuingSystem` `invoice`; `TaxPointDate` whenever stored (never for a proforma);
  `ElectronicPossibilityAgreementReference` empty; `CurrRate` = the stored rate (1 for CZK), `RefCurrRate` 1.
  `Note` = header note, then `Důvod opravy: {reason}` on its own line. Party `Country/Name` "Česká republika" for CZ,
  else the code; `BuildingNumber` empty (the street holds it); DIČ with `TaxScheme` `VAT`; registration as
  `Preformatted`.
- Lines: `ID` = 1-based counter of exported lines; `VATCalculationMethod` 0; line VAT = `round2(base × rate)` in
  VAT-charging mode, else 0; a foreign-currency line converts at the document rate (CZK 2 dp, `UnitPrice` 4 dp).
  A document with no item / text line gets one zero `InvoiceLine` (the schema needs one).
- `PaymentMeans` is always written: `PaidAmount` = payable, due date (issue date if none), account split at `/` into
  `ID` / `BankCode`, `IBAN`, `BIC` (empty elements when unknown), VS, KS.
- Bulk: ordered by issue date, number; entries are the per-document files; a name collision gets `-2`, `-3`, … before
  the extension. The 1000 limit counts after excluding drafts. Entries are written into the ZIP as they are produced
  (PDFs rendered lazily one after another). A document that cannot be exported is skipped and logged, and
  `errors.txt` in the ZIP lists `{number}: export failed ({code})` per skipped document (`code` is the generic error
  code, e.g. `internal`); when nothing at all could be exported the first error is returned instead.
