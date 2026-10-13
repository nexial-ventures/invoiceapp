# Overview (#54)

The space's landing view: one period's income vs expenses, VAT per rate, the monthly trend and categories, plus the
receivables / payables as of today. One read-only route, every role (`accountant`+).

## Route

`GET /api/overview?from=YYYY-MM-DD&to=YYYY-MM-DD` → `200 Overview` (JSON, camelCase, decimals as strings).

- `from` / `to`: inclusive, `from ≤ to ≤ from + 366 days` — the accountant export's period check
  ([csv.md](csv.md#get-apiexportaccountantfromyyyy-mm-ddtoyyyy-mm-dddirectionissuedreceivedboth--for-the-accountant)).
  Missing, not a date, reversed or longer → 422 `{"code":"validation","fields":{"from":"invalid","to":"invalid"}}`
  (only the bad field when one date alone does not parse).
- Read in one read-only repeatable-read snapshot, so the period figures and the open documents agree.

## Document set and amounts

**Period figures** (`income`, `expense`, `vat`, `months`, `categories`) use the accountant export's set: issued and
received documents with status `issued` (no drafts — also not an unlocked one —, no cancelled), **no proformas**, whose
**tax date** `COALESCE(tax_point_date, received_date)` is in the period. DDPPs and their corrections count.

- Every amount is **CZK**: a CZK document as stored, a foreign one from its stored `baseCzk` / `vatCzk` recap and
  `totalCzk` (a legacy recap row without them: × the document rate) — the same conversion as the CSV / Pohoda export.
- **Sign**: credit notes and advance credit notes (stored positive) count negative, in both directions.
- **Amount basis** (`vatPayer` = the company's VAT-payer flag):
  - payer → `amount` = Σ recap base; for a received `standard` document **without** VAT deduction
    (`vatDeductible: false`) base + VAT (the whole total is the expense);
  - non-payer → `amount` = the document total (`total + rounding`, foreign: `totalCzk`).
- `base`, `vat`, `total` are shown on the tiles as secondary lines whatever the basis.

**Open documents** (`receivables`, `payables`) are **not** bound to the period: every status-`issued` document with
`paid < payable` today (`today` = Europe/Prague), except DDPPs (they take no payments). Remaining = `payable − paid`
in CZK (foreign: × the document rate, 2 dp), credit notes negative.

## Response

```
Overview {
  from, to: date,
  today: date,                      // the day the open documents are counted at
  vatPayer: bool,                   // false → the client hides `vat`
  income:  OverviewTotals,          // issued documents of the period
  expense: OverviewTotals,          // received documents of the period
  difference: decimal,              // income.amount − expense.amount
  vat: OverviewVat,
  receivables: OverviewOpen,        // issued, as of today
  payables:    OverviewOpen,        // received, as of today
  months: OverviewMonth[],
  categories: { income: OverviewCategoryAmount[], expense: OverviewCategoryAmount[] }
}

OverviewTotals { count: int, amount, base, vat, total: decimal }

OverviewVat {
  output: OverviewVatRow[],         // issued `standard` documents per rate, highest rate first
  input:  OverviewVatRow[],         // received `standard` documents with vatDeductible, per rate
  outputVat, inputVat: decimal,     // Σ of the rows
  result: decimal,                  // outputVat − inputVat: > 0 to pay, < 0 refund
  reverseChargeIssued: decimal,     // Σ base of issued `reverse_charge` documents (no VAT computed)
  reverseChargeReceived: decimal,   // Σ base of received `reverse_charge` documents (no VAT computed)
  nonDeductible: OverviewVatRow     // received `standard` without deduction, rate 0 (spans rates); informative
}
OverviewVatRow { rate, base, vat: decimal }

OverviewOpen {
  count: int, amount: decimal,              // unpaid / partially paid tax documents (no proformas)
  overdueCount: int, overdueAmount: decimal,// of those, dueDate < today
  topOverdue: OverviewOpenDoc[],            // ≤ 5, earliest due date first (then number)
  advances: { count: int, amount: decimal } // unpaid proformas no non-cancelled invoice settles: expected advances
}
OverviewOpenDoc { id, docType, number /* issued: ours; received: the supplier's, else ours */,
                  counterparty /* snapshot name */, dueDate, remaining: decimal }

OverviewMonth { month: "YYYY-MM", income, expense: decimal, inPeriod: bool }
OverviewCategoryAmount { id: uuid | null /* null = uncategorized */, name: string | null, count: int, amount: decimal }
```

- **VAT block**: only `standard` documents have rate rows; `exempt` and `non_payer` documents appear in the totals
  only. The block is always computed; the client hides it for a non-payer (`vatPayer: false`).
- **Months**: the calendar months the period touches. Two or more → those months, each bar over the part of the
  period in it (the bars add up to the tiles). Fewer (a month or a part of one) → the trailing 12 **whole** months
  ending with the period's month; `inPeriod` marks the months that overlap the period (highlighted by the client).
  Bars use the amount basis, by tax date.
- **Categories**: per direction, the period's documents grouped by `categoryId` (`null` → one uncategorized bucket),
  amount by the basis; largest amount first (equal amounts: categorized first, then by name).

## Frontend

`/` on a space host (route name `home`, first nav item "Přehled" / "Overview") — the space's landing page instead of
the former redirect to the invoice list; unknown paths and refused routes land there too.

- Period selector: month / quarter / year with ‹ › (previous / next), or a custom from–to (client check `from ≤ to`;
  the 366-day cap is the server's). Default the current month. State in the URL query
  `?period=month|quarter|year|custom&from=…&to=…`; a preset is normalized to the one containing `from`, anything
  unusable → the current month.
- Tiles (income, expenses, difference; base / VAT / total as secondary lines), a hand-rolled SVG bar chart (no
  dependency; income blue, expenses orange — a colour-blind-safe pair in both themes; per-month tooltip, a
  screen-reader table), the VAT table (payer only), receivables / payables with the top overdue documents (linked)
  and the expected advances, category breakdowns.

## Clarifications (as implemented)

- Same document set and tax-date expression as the accountant export (`export::period` holds both); unlike the
  export there is no 10 000 cap — the overview reads a few columns per document of at most 13 months, then the
  recap rows in chunks of 500.
- A foreign document without a CZK amount or a rate breaks the CZK invariant → 500 `internal` (logged), as in the
  exports; issued and received foreign documents always get a rate.
- For a non-payer, a final invoice deducting a paid proforma counts only its remainder (proformas are not tax
  documents and stay out of the period figures, as in the export).
- Receivables use the stored `paid` (payment sum); an overpaid document is not open.
