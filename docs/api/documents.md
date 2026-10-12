# API — documents core (phase 1b)

Only `direction = "issued"` and `docType = "invoice"` are creatable in 1b. The schema already carries
`direction` and all doc types (proforma / advance_tax_doc / credit_note arrive in 1c, received in 1e);
creating any other combination → 422 `{"fields":{"docType":"invalid"}}`.

## Status model
- `status`: `draft` | `issued` | `cancelled` (stored).
- `paymentState` (derived, issued only, `null` for drafts/cancelled): `unpaid` | `partial` | `paid` | `overpaid` —
  compares the sum of payments with `totals.payable`.
- `overdue` (derived): `status = issued` and `paymentState ∈ {unpaid, partial}` and `dueDate < today`.
- `sentAt` (stored, nullable): set by mark-sent.

## Wire types
```
DocumentLine =
  | { kind: "item", description: string /* required <=500 */, quantity: string /* decimal, 4 dp, != 0 */,
      unit: string|null /* <=20 */, unitPrice: string /* excl. VAT, 4 dp, may be negative */,
      discountPct: string /* "0".."100", 2 dp, default "0" */, vatRate: string /* percent, e.g. "21" */ }
  | { kind: "text", description: string /* required <=500 */ }
  | { kind: "subtotal", description: string, refs: number[] /* 1-based positions of other lines */, collapse: boolean }
  // Responses add: position (1-based), and for item/subtotal: base (computed, 2 dp; subtotal = sum of referenced bases;
  // subtotal vatRate = the shared rate of its members).

DocumentInput {
  docType: "invoice", direction: "issued",
  contactId: uuid|null,                 // required at issue
  issueDate, taxPointDate: date|null, dueDate: date,
  currency: string,                     // ISO 4217
  exchangeRate: string|null,            // CZK per 1 unit; manual override (null = fetch from ČNB at issue)
  locale: "cs"|"en",
  vatMode: "standard" | "reverse_charge" | "exempt" | "non_payer",
  bankAccountId: uuid|null,             // must match currency; required at issue for paymentMethod bank_transfer
  paymentMethod: "bank_transfer" | "cash" | "card" | "other",
  variableSymbol: string|null /* digits <=10; null → derived from number at issue */,
  constantSymbol: string|null /* digits <=4 */,
  orderRef: string|null /* <=100 */, headerNote: string|null, footerNote: string|null /* <=2000, printed */,
  internalNote: string|null /* <=2000, never printed */,
  roundingMode: "none"|"half"|"unit",   // payable rounding, any currency (4e, defaults.md)
  lines: DocumentLine[]                 // order = position
}

Totals {
  recap: [{ vatRate: string, base: string, vat: string, baseCzk: string|null, vatCzk: string|null }],  // sorted rate desc
  base: string, vat: string, total: string, rounding: string, payable: string,
  totalCzk: string|null      // null for CZK documents or when no rate is known yet
}

Document = DocumentInput + {
  id, number: string|null, status, paymentState, overdue, sentAt: datetime|null, cancelledAt: datetime|null,
  cancelReason: string|null, exchangeRateDate: date|null, exchangeRateSource: "cnb"|"manual"|null,
  supplier: PartySnapshot|null, customer: PartySnapshot|null,  // filled at issue; drafts show null
  bankSnapshot: { accountNumber, iban, bic }|null,             // filled at issue
  lines: DocumentLine[] (with position/base), totals: Totals, paid: string /* sum of payments */,
  createdAt, updatedAt
}
PartySnapshot { name, ico, dic, street, city, zip, country, registration|null, vatPayer: boolean|null }

DocumentSummary { id, docType, direction, number, status, paymentState, overdue, contactId, customerName /* snapshot or
  live contact name for drafts */, issueDate, dueDate, currency, payable, paid, sentAt }

Payment { id, date, amount: string /* >0, doc currency */, note: string|null, createdAt }
```

## Computation (authoritative on the server; pure module, unit-tested)
- item `base = round2(quantity × unitPrice × (1 − discountPct/100))`, half away from zero.
- `text` lines carry no amounts. `subtotal` lines are display-only: never counted in totals.
- Recap: group item bases by `vatRate`; `vat = round2(Σbase × rate/100)` per rate (§37 ZDPH — per rate, not per line).
  For `reverse_charge`, `exempt`, `non_payer`: `vat = 0` in every recap row (rates kept for display).
  `non_payer` requires every item `vatRate = "0"` (else 422 `lines.N.vatRate: invalid`).
- `total = Σbase + Σvat`; `payable` = `total` rounded by `roundingMode` (`unit` whole units, `half` nearest 0.50,
  half away from zero, any currency; `none` → `payable = total`), `rounding = payable − total` (4e, see
  [defaults.md](defaults.md)).
- Foreign currency with a known rate: `baseCzk = round2(base × rate)`, `vatCzk = round2(vat × rate)` per recap row,
  `totalCzk = round2(payable × rate)`.
- Subtotal validation (port of infra `repo/subtotals.rs`): refs must point to existing other positions, no cycles, all
  (transitively) referenced item lines share one `vatRate` → else 422 `lines.N.refs: invalid`.
- `POST /api/documents/compute` body `{ lines, vatMode, currency, exchangeRate, roundingMode }` → `{ lines (with base),
  totals }` — no DB writes; the editor uses it for live totals (debounced). Same 422s as save.

## Defaults on create (fields omitted or null in the POST body)
**Since 4e** `paymentMethod`, `currency`, the rounding mode and the notes come from the company defaults and
`roundTotal` is replaced by `roundingMode` — see [defaults.md](defaults.md).
`issueDate` = today; `taxPointDate` = issueDate; `dueDate` = issueDate + (contact.defaultDueDays ?? company.defaultDueDays);
`locale` = contact.defaultLocale ?? company.defaultLocale; `currency` = contact.defaultCurrency ?? company.defaultCurrency;
`bankAccountId` = default account for the currency; `vatMode` = company.vatPayer ? "standard" : "non_payer";
`paymentMethod` = company.defaultPaymentMethod; `roundingMode` = the company's default for the payment method;
`headerNote` / `footerNote` = the company's notes of the locale; new item lines default `vatRate` = default VAT rate (non_payer: "0").
(The POST body may therefore omit those fields; PUT replaces all fields.)

## Routes
- `GET /api/documents?direction=issued&docType=&status=&paymentState=&overdue=true&contactId=&q=&from=&to=&limit=50&offset=0`
  → `{ items: DocumentSummary[], total }`. `q` matches number / customer name / variable symbol; `from`/`to` filter
  `issueDate` (inclusive). Ordered `issueDate desc, number desc nulls first, createdAt desc`.
- `POST /api/documents` → 201 Document (draft). `GET /api/documents/{id}`. `PUT /api/documents/{id}` (draft only).
  `DELETE /api/documents/{id}` → 204 (draft only).
- Changing a non-draft → 409 `{"code":"document_locked"}`.
- `POST /api/documents/{id}/issue` → Document. Validations (422 with fields): contactId required; at least one item line;
  `dueDate >= issueDate`; `taxPointDate` required for invoice; bank account required for bank_transfer and must match
  currency; foreign currency → rate = manual `exchangeRate` or ČNB for `taxPointDate` (ČNB failure with no manual rate →
  422 `{"fields":{"exchangeRate":"required"}}`). In one transaction: allocate number from the doc type's series for the
  **issueDate year**, store `number`, `numberYear`, `numberSeq`; snapshot supplier (company), customer (contact), bank;
  `variableSymbol` = digits of number (last 10) if null; recompute + store totals; status `issued`. The PDF is
  rendered **before** that transaction (from a rolled-back dry run of it) and recorded in its commit — see
  [pdf.md, Archive](pdf.md#archive).
- `POST /api/documents/{id}/cancel` body `{ reason?: string }` → issued only (else 409 `invalid_state`); payments stay.
- `POST /api/documents/{id}/mark-sent` body `{ sentAt?: datetime }` → issued only; idempotent (overwrites).
- `PUT /api/documents/{id}/internal-note` body `{ internalNote }` → allowed in every status.
- `GET /api/documents/{id}/payments` → Payment[] (date asc). `POST /api/documents/{id}/payments` → 201 (issued only, else
  409 `invalid_state`). `DELETE /api/documents/{id}/payments/{paymentId}` → 204 (issued only).
- `GET /api/exchange-rates/{currency}?date=YYYY-MM-DD` → `{ currency, date /* the ČNB publication date used */, rate }`;
  CZK → rate "1"; unknown currency → 404 `not_found`; ČNB unreachable → 502 `{"code":"cnb_unavailable"}`.

## ČNB
`GET {INVOICE__CNB_URL}?date=DD.MM.YYYY` (default `https://www.cnb.cz/cs/financni-trhy/devizovy-trh/kurzy-devizoveho-trhu/kurzy-devizoveho-trhu/denni_kurz.txt`),
10 s timeout. Text format: line 1 `08.10.2026 #196` (publication date), line 2 header, then
`země|měna|množství|kód|kurz` with comma decimals; `rate = kurz / množství`. ČNB returns the latest list published on or
before the requested date. Cache: table `exchange_rates (currency, requested_date) → (published_date, rate)`; a cached row is
reused without calling ČNB. A row is written only for a final list: one published for exactly the requested date, or
any list for a past date. A today-or-later request answered with an older list (before ČNB publishes, ~14:30) is not
stored; that list is reused from memory for at most **60 s** (`src/cnb/recent.rs`), so after publication a stale rate
is served for a minute at most. Tests use a local mock server with fixture text — never the real ČNB.

## Number-series counter guard (finishes the 1a review item)
`PUT /api/settings/number-series/{docType}/counters/{year}` with `lastNumber` lower than the highest `numberSeq` of a
non-imported document of that docType and `numberYear` → 422 `{"fields":{"lastNumber":"below_issued"}}`.

## Clarifications 1b (as implemented)
Additive details settled during the Phase 1b backend; nothing above changed.

Field errors
- Line errors are keyed `lines.N.<camelCaseField>` where **N is the 0-based array index** of the line in the request (not
  its 1-based `position`); `refs` values themselves stay 1-based positions. A bad subtotal is reported as `lines.N.refs`
  on the subtotal line that holds the bad refs (for a cycle: on one subtotal of the cycle). An unknown `kind` →
  `lines.N.kind: invalid`; more than 1000 lines → `lines: too_long`; a line base or total that would not fit
  `numeric(18,2)` (≥ 10^16) → `lines: invalid`; amounts that fit but whose CZK conversion does not →
  `exchangeRate: invalid`. All arithmetic is checked (never a 500).
- Subtotal refs are invalid when empty, out of range, self-referencing, duplicated, pointing at a `text` line, cyclic,
  or mixing VAT rates, or when nested subtotals sum beyond the representable range.
- Line rules (non-payer rates, subtotal refs) run only after all field-level checks pass.
- New reason code `below_issued` (counter guard).

Documents
- `docType` / `direction` may be omitted (→ `invoice` / `issued`); anything else → `docType: invalid`.
- `contactId` naming a missing contact → `contactId: invalid`. `bankAccountId` naming a missing account, or one whose
  currency differs from the document's → `bankAccountId: invalid` (on save too, not only at issue).
- `PUT` applies **no** defaults: `issueDate`, `dueDate`, `currency`, `locale`, `vatMode`, `paymentMethod` missing →
  `required`; `taxPointDate`/`bankAccountId` stay `null` when omitted, `roundingMode` → `none` (4e). Item `vatRate` defaults on both POST and PUT
  (and compute); with no default VAT rate configured a missing item `vatRate` is `required`.
- Number limits: `quantity` / `unitPrice` at most 4 dp and `|x| < 10^12`; `exchangeRate` > 0, at most 6 dp;
  `discountPct` 0..100, 2 dp. `unit` ≤ 20, line `description` ≤ 500 (subtotal description may be empty),
  `cancel.reason` ≤ 2000, payment `note` ≤ 500.
- For CZK documents `exchangeRate` is ignored (stored `null`). A non-null `exchangeRate` on save sets
  `exchangeRateSource: "manual"`; drafts with a manual rate already show the CZK recap/`totalCzk`.
- Money in responses always has 2 dp (`"0.00"`); rates and quantities are normalized (`"21"`, `"0.15512"`).
- Response lines: `item` `{kind, position, description, quantity, unit, unitPrice, discountPct, vatRate, base}`,
  `text` `{kind, position, description}`, `subtotal` `{kind, position, description, refs, collapse, base, vatRate}`.
- `GET /api/documents/{id}` returns stored totals; line `base`s are recomputed from the stored lines.
- `DELETE` / `PUT` of a non-draft → 409 `document_locked`; `issue` of a non-draft → 409 `invalid_state`.
- `POST /api/documents/compute`: body fields default to `vatMode` = company default, `currency` = `CZK`,
  `roundingMode` = `none`. It does **not** validate line text (`description`, `unit`) — only what affects amounts
  (`quantity`, `unitPrice`, `discountPct`, `vatRate`, `refs`, `kind`, `vatMode`, `currency`, `exchangeRate`, `roundingMode`).
  Save and issue keep the full validation.

Issue
- Validations run before the ČNB call; all failing fields are reported together: `contactId: required` (also when the
  contact was deleted), `lines: required` (no item line), `dueDate: invalid`, `taxPointDate: required`,
  `bankAccountId: required` (bank transfer without an existing account) / `invalid` (currency mismatch, any method).
- ČNB unreachable **or** not listing the currency, with no manual rate → `exchangeRate: required`. A manual rate gives
  `exchangeRateSource: "manual"`, `exchangeRateDate: null`; a ČNB rate gives `"cnb"` and the ČNB publication date.
- If the draft is edited, or its planned number taken, between the checks / render and the locked write, the issue
  starts over with the current data (validations, rate, render): once after an edit (edited again → 409 `conflict`,
  nothing issued), up to 5 times when only the number was taken by a concurrent issue.
  No transaction is open while the PDF renders, so editing the draft or issuing other documents is never blocked by
  a slow mdcast.
- Supplier snapshot: company fields + `registration` + `vatPayer`; customer snapshot: contact fields with
  `registration: null`, `vatPayer: null`. `bankSnapshot` is set whenever the document has a bank account.

Lifecycle and payments
- `cancel` / `mark-sent` bodies are optional (empty body allowed); a malformed body → 400. `mark-sent` without `sentAt`
  uses now.
- `paymentState`: `paid` when the payment sum equals `payable` (or nothing is paid on a non-positive payable),
  `unpaid` when nothing is paid, `partial` below, `overpaid` above. Payment `amount` > 0, at most 2 dp, `date` required.
  Payments of a cancelled document are listed but can be neither added nor deleted (409 `invalid_state`).
  A payment that would push the payment sum to 10^16 or beyond (`numeric(18,2)`) → `amount: invalid`.
- A payment id that does not belong to the document → 404.

List
- `status` / `paymentState` / `overdue` / dates / `contactId` that do not parse → 400 `bad_request`; unknown `direction` or
  `docType` values simply match nothing. `overdue=false` returns everything that is not overdue. `limit`/`offset` as for
  contacts. `paymentState` filters imply `status = issued`.
- "Today" (`overdue`, the default `issueDate`, the ČNB cache decision, `nextNumberPreview`) is the date in
  Europe/Prague, independent of the server's time zone.

ČNB / exchange rates
- `GET /api/exchange-rates/{currency}`: `date` defaults to today; a malformed currency → 404; a malformed `date` → 400.
- Rates are stored as `numeric(18,6)` (`kurz / množství` rounded half away from zero to 6 dp).
- A fetched list is cached for every currency it contains, keyed by the requested date, but only once it is final:
  published for exactly the requested date, or the requested date is already past. A request for today (or later)
  answered with an older list (before ČNB publishes ~14:30) is not cached. A currency missing from a final list is
  cached as absent (`rate` NULL), so repeated lookups stay 404 / `exchangeRate: required` without calling ČNB.

Number series
- `allocate_number(&txn, DocType, year) -> Result<(String, i32), AppError>` (number and sequence).
- The counter guard counts every non-imported document of that docType and `numberYear` with a number (cancelled
  included — their numbers stay used). The counter `PUT` locks the counter row (creating it if missing) before
  checking, so it waits for an issue in flight and sees its number.

---

