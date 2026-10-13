# Accounting exports: settings, Pohoda XML (3b)

Exports for the accountant's program, chosen as a **format** of the 2c accountant export
([csv.md](csv.md#get-apiexportaccountantfromyyyy-mm-ddtoyyyy-mm-dddirectionissuedreceivedboth--for-the-accountant)):
same period (tax date), same direction choice, same document set (no drafts, no cancelled, no proformas), same
10 000 cap, same consistent snapshot and skip-and-log of a broken document. Content = document header + VAT summary
per rate **in CZK** (no lines), like the CSV.

## Route
`GET /api/export/accountant?from&to&direction&format=csv|pohoda|money` — `format` default `csv` (2c unchanged).
`pohoda` → `200 application/xml`, filename `pohoda-{from}-{to}.xml`. `money` → [money.md](money.md) (3c). Unknown
format → 422
`format: invalid`.

## Settings → Accounting (`/api/settings/accounting`)

Per program, per **direction × document type** (the 6 exported types: invoice, credit_note, debit_note,
advance_tax_doc, advance_credit_note, simplified) a set of optional codes; empty = the element is not written and the
accountant fills it in the program.

```
AccountingSettings {
  pohoda: {
    ico: string | null,        // dataPack/@ico (the accounting unit); null → the company IČO
    codes: [ { direction, docType, accounting: string|null /* předkontace, typ:ids */,
               classificationVat: string|null /* členění DPH, typ:ids */, numberSeries: string|null /* číselná řada, typ:ids */ } ]
  },
  money: { ico, codes: [ … ] }   // 3c, see money.md
}

AccountingResponse = AccountingSettings + {
  limits: { pohoda: { code: 19, numberSeries: 19 }, money: { code: 10, numberSeries: 5 } }
}
```
- `GET` → `AccountingResponse`: the full object, every direction × type row present (missing rows → all null), plus
  `limits` — the longest code (`accounting`, `classificationVat`, `classificationVatNonDeductible`) and number series
  per program, in characters after trimming, as `PUT` enforces them. `PUT` answers the same shape; `limits` in a `PUT`
  body is ignored.
- `PUT` → replaces each section present in the body, keeps an absent one (3c, [money.md](money.md#settings--accounting)); each code ≤ 19 chars for Pohoda (see Clarifications), ≤ 10 for Money (series ≤ 5, see money.md), trimmed, `""` → null; unknown direction / docType or a duplicate row → 422
  (`pohoda.codes.N.docType: invalid` / `duplicate`). Stored as one JSON settings row (migration).

## Pohoda XML (3b)

- Stormware data exchange XML, `dataPack` **version 2.0** (`dat:dataPack` with `id`, `ico`, `application="InvoiceApp"`,
  `version="2.0"`, `note`), one `dat:dataPackItem` (`id` = document id, `version="2.0"`) per document holding an
  `inv:invoice` (`version="2.0"`). Encoding **Windows-1250** (`<?xml version="1.0" encoding="Windows-1250"?>`, as
  Pohoda imports it; characters outside 1250 → numeric character references). Namespaces `dat`, `inv`, `typ` as in
  the official schemas.
- **Validation:** the official XSDs (data.xsd, invoice.xsd, type.xsd + their imports, version 2) are vendored under
  `tests/fixtures/pohoda/schema/` (with their source URL and date in a README) and every test export is validated with
  `xmllint` (already in CI for ISDOC).
- **invoiceType** by direction × doc type:

  | doc type | issued | received |
  |---|---|---|
  | invoice, simplified | `issuedInvoice` | `receivedInvoice` |
  | credit_note | `issuedCreditNotice` | `receivedCreditNotice` |
  | debit_note | `issuedDebitNote` | `receivedDebitNote` |
  | advance_tax_doc, advance_credit_note | `issuedAdvanceInvoice` | `receivedAdvanceInvoice` |

  (the exact enumeration values and the right agenda for the DDPP and its correction are confirmed against the XSD and
  Stormware's documentation; deviations go to Clarifications).
- **Header (`inv:invoiceHeader`):**
  - `invoiceType`; `number/numberRequested` = our number (received: our internal number) when no series code is set,
    else `number/ids` = the configured series; `symVar` = variable symbol; `originalDocument` = the supplier number
    (received) / our number (issued); `date` = issue date, `dateTax` = tax date, `dateAccounting` = tax date
    (received: received date), `dateDue`; `accounting/ids`, `classificationVAT/ids` from settings when set;
    `text` = a short description (doc type label + number);
  - `partnerIdentity/address`: company, name, street, city, zip, `ico`, `dic`, country `ids` — from the document's
    counterparty snapshot (contactless simplified → omitted);
  - `paymentType/paymentType` from the payment method (`draft` = bank transfer, `cash`, `creditcard`, …; exact
    values per XSD); `account` for issued documents only when the snapshot has an account / bank code;
  - corrections (credit / debit notes, DDPP corrections): the original's number in the element the XSD provides for
    the linked document (`originalDocumentNumber` or equivalent), when linked;
  - received `vatDeductible` false → the classification chosen in settings applies; nothing extra is invented.
- **Summary (`inv:invoiceSummary`):** `roundingDocument` none; CZK documents → `homeCurrency` with `priceNone`
  (0 %), `priceLow` + `priceLowVAT` (the lower rate), `priceHigh` + `priceHighVAT` (21 %), `price3` + `price3VAT`
  (second reduced rate when a document has one), and `round/priceRound` = rounding; amounts in CZK from the stored
  recap (credit notes / corrections **negative**). Foreign documents → `homeCurrency` in CZK (the stored CZK recap)
  **plus** `foreignCurrency` (`currency/ids`, `rate`, `amount` 1, `priceSum` = total in the currency). A rate that
  maps to none of Pohoda's slots (not 0 / 21 / 12 / a configured third) → the document is skipped and logged (2c rule).
  Rate → slot mapping: 21 % high, 12 % low, 0 % none; other active Settings rates in order to `price3`; documented in
  Clarifications.
- `dat:dataPack/@note` = `Export z Invoice {from}–{to}`.

## UI
- "Export pro účetní" dialog: a format select (CSV / Pohoda XML; Money S3 appears in 3c), default CSV; filename from
  the response.
- Settings → "Účetnictví" tab: Pohoda section — IČO override, a table rows = direction × doc type, columns
  předkontace / členění DPH / číselná řada; Save. (Money S3 columns come in 3c.)

## Tests
- Pure mapping unit tests (doc type → invoiceType, rate slots, negatives, foreign currency, 1250 encoding incl. a
  character outside 1250).
- Integration: a period with every exported type in both directions incl. EUR, validated against the XSD; settings
  codes present / absent; skipped document; settings GET/PUT validation.

## Clarifications (as implemented)
Additive details and deviations settled during the Phase 3b backend. Element names and enumerations were checked
against the official version 2 XSDs (vendored in `tests/fixtures/pohoda/schema/`, downloaded 2026-10-10 from
`https://www.stormware.cz/xml/schema/version_2/`).

Route
- `format`: `csv` (default) | `pohoda` | `money` (3c). Anything else → 422 `format: invalid`. It is reported
  together with the other query errors.
- `Content-Type: application/xml; charset=windows-1250`, `Cache-Control: no-store`. It uses the 2c document set,
  order, 10 000 cap and snapshot transaction. Documents are loaded 500 at a time without payments or categories.
  Unlike the CSV, **the whole file is built inside the snapshot before anything is sent**: the accountant never
  gets a file with documents silently missing.
- **Empty period** → 422 `{"code":"validation","fields":{"from":"empty"}}`. An empty `dat:dataPack` would not pass
  the XSD (`dataPackItem minOccurs=1`). The CSV keeps its header-only file.
- **Unexportable document** → 422 with no file:
  `{"code":"validation","fields":{"documents":"unexportable"},"detail":"<number>: <reason>; <number>: <reason>"}`.
  - `<number>` is our number for an issued document and the supplier number for a received one (else the internal
    number, else the id).
  - The detail is capped at 2000 characters and then ends with `; … (+N more)`. Every failure is logged at `warn`.
  - Reasons:
    - a recap rate with no Pohoda slot (`VAT rate 10 % maps to no Pohoda rate slot`);
    - `number` or `originalDocument` longer than 32 characters, or `symVar` longer than 20 (numbers are never
      cut);
    - an undecodable snapshot;
    - an unknown stored value (direction, type, VAT mode, payment method);
    - a foreign recap with neither a CZK amount nor an exchange rate.
  - The CSV keeps skip-and-log. Its logs name the export as `list csv` / `accountant csv`.
- OpenAPI declares both `200` content types.

Settings
- **Code length ≤ 19** (not 20): Pohoda's `typ:idsType` has `maxLength` 19. Longer → `too_long`.
- 422 keys:
  - `pohoda.ico`: `invalid_ico` (mod-11 check, inner spaces removed);
  - `pohoda.codes.N.direction` / `docType`: `invalid`;
  - `pohoda.codes.N.docType`: `duplicate`;
  - `pohoda.codes.N.{accounting|classificationVat|classificationVatNonDeductible|numberSeries}`: `too_long`;
  - `pohoda.codes.N.classificationVatNonDeductible`: `invalid` on an issued row.

  `N` is the index in the request. Invalid input is the wrong value inside a valid JSON shape; a wrong JSON type is
  400 `bad_request`.
- PUT takes any subset of rows. The stored value and both responses always hold the 12 rows: issued first, types in
  the order invoice, credit_note, debit_note, advance_tax_doc, advance_credit_note, simplified. Since 3c a PUT
  replaces only the sections it holds: `{"pohoda": {}}` clears the Pohoda codes, `{}` changes nothing.
- **Additive field** `classificationVatNonDeductible: string | null` on every code row (received rows only;
  always null on issued rows): the členění DPH of a received document without the VAT deduction
  (`vatDeductible` false).
- **Money S3:** the `money` section (same shape, own limits) came with 3c:
  [money.md](money.md#settings--accounting).
- Storage: table `accounting_settings` (singleton `id = 1`, `data jsonb`), migration
  `m20261015_000001_accounting_settings`.

Pohoda XML
- `dataPack`: `id` = `invoice-{from}-{to}`, `ico` = the settings IČO, else the company IČO, else omitted (it is
  optional). `dataPackItem/@id` = the document UUID.
- **Agenda / `invoiceType`** (enumeration `inv:invoiceTypeType`): invoice and simplified →
  `issuedInvoice` / `receivedInvoice`; credit_note → `issuedCreditNotice` / `receivedCreditNotice`; debit_note →
  `issuedDebitNote` / `receivedDebitNote`.
  - **Deviation: DDPP (`advance_tax_doc`) and its correction (`advance_credit_note`), both directions, go to
    Interní doklady (`int:intDoc` version 2.0).** They do not go to `issuedAdvanceInvoice` /
    `receivedAdvanceInvoice`. A Pohoda advance invoice (zálohová faktura) is not a tax document:
    `classificationVAT` "is not used for advance invoices" per the XSD, and it never enters the VAT return. Pohoda
    itself creates a DDPP from a paid advance invoice in Interní doklady ("Záznam / Vytvořit daňový doklad"), and
    received DDPPs are recorded there as well.
  - The correction is the same agenda with negative amounts.
  - `int:intDoc` has no `invoiceType`, `originalDocument`, `dateDue`, `paymentType` or `account`, so none is
    written.
  - `issuedCorrectiveTax` / `receivedCorrectiveTax` (CZ "opravný daňový doklad") are Pohoda's §44 bad-debt
    corrections, so they are not used for credit notes.
- **Header** (`invoiceHeader` / `intDocHeader`, both `xsd:all`, written in XSD order):
  - `number` (`typ:numberType` is `xsd:all`, both children allowed): `typ:numberRequested` = our number (received:
    the internal number) **always**, plus `typ:ids` = the series code when one is set. Pohoda keeps our number, and
    `checkDuplicity` (default `true`) refuses a second import of the same number.
  - `symVar`; `originalDocument` (invoice agenda only): issued = our number, received = the supplier number.
  - `date` = issue date; `dateTax` = DUZP (received: else the received date). `dateAccounting`: issued = the tax
    date, received = the received date (else the tax date). `dateDue` (invoice agenda only).
  - `accounting/typ:ids` and `classificationVAT/typ:ids` come from the settings row of the document's direction ×
    type. No code → the element is left out.
  - **Non-deductible received documents** (`vatDeductible` false) take `classificationVatNonDeductible` instead,
    never the deductible code. When it is empty they get `classificationVAT/typ:classificationVATType` =
    `nonSubsume` ("Nezahrnovat do DPH", the schema's only other value). Leaving the element out would not work:
    Pohoda's default is `inland`, which deducts. The XSD has no other element that marks non-deductible VAT.
  - **Linked original:** the CZ schema has no element for it. `originalDocumentNumber` is marked "pouze SK verze",
    and `links` / `linkedDocuments` / `correctiveDocument` need the original to exist in Pohoda. So the original's
    number (issued: its number, received: its supplier number) goes into `text`.
  - `text` = label + shown number (+ ` k {original}`), cut to 240. Labels are the UI's Czech names, e.g.
    `Faktura FA-1`, `Dobropis DB-1 k FA-1`, `Přijatá faktura FV-42`,
    `Opravný daňový doklad k platbě OD-1 k DD-1`.
  - `partnerIdentity/typ:address`, from the snapshot:
    - `company` = the name (our snapshot has one name; `name` is not written), then `city`, `street`, `zip`, `ico`,
      `dic`, `country/typ:ids`;
    - empty values are left out;
    - text is cut to the XSD lengths (company 255, city 45, street 64, zip 15, ico 15, dic 18);
    - no snapshot (contactless simplified) → no element.
  - `paymentType/typ:paymentType` (enumeration `typ:paymentType`): bank_transfer → `draft`, cash → `cash`, card →
    `creditcard`; `other` → no element (Pohoda's default).
  - `account` (issued, invoice agenda only): the bank snapshot's `accountNumber` split at `/` into `typ:accountNo` +
    `typ:bankCode` (≤ 34 / ≤ 11). IBAN-only or too long → no element.
  - The VAT modes add nothing beyond the slots below.
- **Summary** (`invoiceSummary` / `intDocSummary`):
  - `roundingDocument` (enumeration `typ:typeRoundingDocument`), chosen so Pohoda's total equals our payable:
    - no rounding → `none`;
    - otherwise the mathematical mode that reproduces the stored rounding exactly from `total`: `math2one` (our
      "round the total"), then `math2half`, then `math2tenth`;
    - no mode matches (an imported odd rounding) → the element is left out, so it cannot contradict `priceRound`.
      This applies to foreign documents too (their home amounts carry no `round`).
  - `homeCurrency` holds only the slots the document uses: `priceNone`, `priceLow` + `priceLowVAT`, `priceHigh` +
    `priceHighVAT`, `price3` + `price3VAT`. Amounts are in CZK from the stored recap, 2 dp, `.` decimal point.
  - Credit notes and DDPP corrections (both directions) are negative.
  - CZK documents also carry `typ:round/typ:priceRound` = `rounding` (signed).
  - **Rate slots:** 21 % → high, 12 % → low, 0 % → none. `price3` = the **first active** Settings → VAT rate
    (Settings order) that is not 0 / 12 / 21. Only that one rate can use it. Any other rate → skip + log.
  - Exempt and non-payer documents put every base in `priceNone` (they have no VAT; a rate slot would make Pohoda
    tax it). Reverse charge keeps the rate slot with VAT 0, because Pohoda's PDP classification computes the
    self-assessed VAT from that slot's base.
  - **Foreign currency:** `homeCurrency` (stored CZK recap, no `round`) plus `foreignCurrency`: `currency/typ:ids`,
    `rate` (the stored rate when there is one), `amount` 1, `priceSum` = total + rounding in the currency (signed).
    Per the XSD, Pohoda ignores the home amounts of a foreign document on import and computes them from the
    foreign ones. They are written for completeness.
  - `*VAT` elements are annotated "jen pro export" in `type.xsd`. The contract wants them and the XSD allows them,
    so they are written.
- **Encoding:** the text is encoded with `encoding_rs` WINDOWS_1250. A character outside it becomes a decimal
  numeric character reference (`Ж` → `&#1046;`), valid in both text and attributes.
