# API — advances, DDPP, credit notes, catalog (phase 1c)

Creatable doc types grow to `invoice` and `proforma` (POST). `advance_tax_doc` (DDPP) is only ever created by the
server; `credit_note` only via the credit-note endpoint. Everything below extends the 1b contract.

## Document additions
```
DocumentInput += {
  docType: "invoice" | "proforma" | "credit_note",   // credit_note only valid on PUT of a credit-note draft
  correctionReason: string|null                       // credit_note only; required at issue; <=500
}
DocumentLine += | { kind: "advance", advanceDocumentId: uuid }
  // response adds: description (server-generated, e.g. "Odpočet zálohy DP20260003" / en), base: string (negative),
  // recap: [{ vatRate, base, vat }] (negative values, the deducted amounts per rate)
Document += {
  relatedDocumentId: uuid|null,            // credit_note → invoice, invoice → proforma it settles, DDPP → proforma
  paymentId: uuid|null,                    // DDPP only: the proforma payment it documents
  relatedDocuments: [{ id, docType, number, status, payable }],  // documents whose relatedDocumentId = this id
  settled: boolean|null,                   // proforma only: a non-cancelled invoice settles it
  sign: 1 | -1                             // -1 for credit_note (amounts are stored positive)
}
DocumentSummary += { sign, relatedDocumentId }
Payment += { exchangeRate: string|null /* request-only, proforma in foreign currency */, advanceDocumentId: uuid|null /* response: the DDPP it created */ }
```

## Proforma
- Not a tax document: `taxPointDate` must be null (422 `invalid` otherwise); numbered from the `proforma` series at issue.
  Other rules as an invoice (lines, vatMode, bank account, lock after issue, cancel).
- Cancel is refused (409 `invalid_state`) while it has payments.

## DDPP (automatic)
When a payment is added to an issued proforma **and** the proforma's supplier snapshot has `vatPayer = true` **and**
`vatMode = "standard"`, the same transaction issues an `advance_tax_doc`:
- number from the `advance_tax_doc` series (year of the payment date); `issueDate = taxPointDate = dueDate = payment.date`;
  contact, customer/supplier/bank snapshots, currency, locale, vatMode copied from the proforma; `relatedDocumentId` =
  proforma, `paymentId` = payment.
- Exchange rate (foreign currency): `payment.exchangeRate` if given (source `manual`), else ČNB for the payment date
  (fetched **before** the transaction); none available → 422 `{"fields":{"exchangeRate":"required"}}` and the payment
  is not stored.
- Amounts — VAT from above, split by the proforma's recap: gross per rate `G_r = base_r + vat_r`, `T = Σ G_r`.
  Share `P_r = round2(P × G_r / T)` for every rate except the highest-gross one, which gets `P − Σ others` (so the
  shares sum to P exactly). `vat_r = round2(P_r × rate / (100 + rate))`, `base_r = P_r − vat_r`.
  Stored as one item line per rate: description "Přijatá záloha k zálohové faktuře {number}" (en: "Advance payment
  received for proforma {number}"), quantity 1, unitPrice = `base_r`, vatRate = rate; its stored recap is exactly
  (`base_r`, `vat_r`) — the server must not recompute VAT for a DDPP from base × rate.
- A DDPP has no payments of its own (`paymentState` null), cannot be edited, and cannot be cancelled directly (409
  `invalid_state`).
- `DELETE /api/documents/{proformaId}/payments/{paymentId}` cancels the linked DDPP (`cancelReason` "Platba smazána" /
  stored as given, `cancelledAt` now) in the same transaction; if that DDPP is deducted by an issued, non-cancelled
  invoice → 409 `{"code":"advance_settled"}` and nothing changes.

## Settlement (final invoice)
`POST /api/documents/{proformaId}/settle` → 201 Document: a draft `invoice` with header + lines copied from the
proforma (taxPointDate = today), `relatedDocumentId` = proforma, plus:
- payer case: one `advance` line per issued (non-cancelled) DDPP of the proforma;
- non-payer / non-standard vatMode: one `advance` line with `advanceDocumentId` = the proforma itself, deducting
  `proforma.paid` with zero VAT (recap row at rate "0").
Only for an issued proforma that is not yet `settled` (else 409 `invalid_state`).

Advance line rules (save, compute, issue): the referenced document must be an issued DDPP of the same contact and
currency (or, for the non-payer form, the related proforma itself); a DDPP may be referenced by at most one
non-cancelled invoice (422 `lines.N.advanceDocumentId: invalid` / `duplicate`). Only `invoice` documents may hold
advance lines.

Totals with advances (extends 1b computation): per rate `base_r = Σ item base − Σ advance base`,
`vat_r = round2(Σ item base × rate/100) − Σ advance vat` (standard mode; other modes vat 0);
`total = Σ base_r + Σ vat_r`; rounding/payable as in 1b; `payable` may be 0 or negative (overpaid advance).
CZK (foreign currency): item part at the invoice rate, advance part at each DDPP's own CZK amounts:
`baseCzk_r = round2(Σ item base_r × rate) − Σ advance baseCzk_r` (same for vat).

## Credit notes
`POST /api/documents/{invoiceId}/credit-note` body `{ correctionReason?: string }` → 201 draft `credit_note`:
copies contact, currency, locale, vatMode, bank, paymentMethod and all lines (advance lines dropped) from the invoice;
`issueDate = taxPointDate = today`, `dueDate` = today + company due days; `relatedDocumentId` = invoice;
`exchangeRate` = the invoice's rate (stored, source `"original"`, not editable — PUT ignores a different value).
Only for an issued, non-cancelled invoice (else 409 `invalid_state`).
- Amounts are entered and stored **positive**; `sign = -1`. Numbered from the `credit_note` series.
- Issue additionally requires `correctionReason` and checks the cap: for every VAT rate, Σ base of all
  non-cancelled credit notes of the invoice (including this one) ≤ the invoice's base for that rate, and no rate absent
  from the invoice → 422 `{"fields":{"lines":"exceeds_original"}}`.
- Payments on a credit note are refunds (same endpoints, same paymentState semantics).

## Catalog — `/api/catalog`
```
CatalogItem { id, name /* required <=200 */, unit: string|null, unitPrice: string /* 4 dp */, currency /* ISO */,
  vatRate: string, active: boolean, note: string|null, createdAt, updatedAt }
CatalogGroup { id, name /* <=200, becomes the subtotal description */, collapse: boolean /* default true */,
  members: [{ itemId, quantity: string, position, item: CatalogItem }], createdAt, updatedAt }
```
- `GET /api/catalog/items?q=&active=true` → CatalogItem[] (by name); `POST`, `GET/PUT/DELETE /api/catalog/items/{id}`.
  Deleting an item removes it from groups (cascade).
- `GET /api/catalog/groups?q=` → CatalogGroup[]; `POST`, `GET/PUT/DELETE /api/catalog/groups/{id}`; body members
  `[{ itemId, quantity }]` (order = position, ≥1 member, unique itemId).
- Insertion into a document is done by the client: an item becomes an `item` line (price left empty when the
  catalog currency differs from the document currency); a group becomes its member lines followed by a `subtotal`
  over them with the group's `collapse` (members must share one VAT rate — enforced on group save:
  `{"fields":{"members":"mixed_vat"}}`).

## Clarifications 1c (as implemented)
Additive details settled during the Phase 1c backend; nothing above changed.

Errors
- New: 409 `{"code":"advance_settled"}`, `{"code":"advance_in_use"}`, `{"code":"catalog_item_in_use"}`; reason codes `exceeds_original` (credit-note cap), `mixed_vat` (catalog group / item rate).

Document types
- `POST`: `docType` `invoice` (default) or `proforma`; `credit_note` / `advance_tax_doc` / anything else → `docType: invalid`.
  `PUT`: `docType` omitted keeps the draft's type; any value other than the draft's own type → `docType: invalid`
  (no invoice ↔ proforma conversion).
- Proforma: `POST` does not default `taxPointDate`; a non-null one → `taxPointDate: invalid` (save and issue). Its ČNB
  rate at issue is for `issueDate`.
- `correctionReason` is kept for credit notes only (ignored on other types), ≤ 500 (`too_long`). A credit-note `PUT`
  ignores `exchangeRate` (stays the invoice's, `exchangeRateSource: "original"`) and requires the invoice's
  `currency`, `contactId` and `vatMode` (else `invalid` on that field).
- `relatedDocuments` are ordered by creation. `settled` counts draft invoices too (a settlement draft settles the
  proforma); cancelling the final invoice makes the proforma settleable again. `sign` is in `Document` and
  `DocumentSummary`.
- `Document.parent: { id, docType, number, status, payable } | null` — the document `relatedDocumentId` points at
  (credit note → invoice, final invoice / DDPP → proforma), same shape as a `relatedDocuments` entry.

Payment state
- `payable ≤ 0` keeps the 1b rule: nothing paid on a non-positive payable → `paid` (so a fully deducted final invoice
  is `paid` right after issue); any payment on it → `overpaid`. `overdue` only when `paid < payable`.
- A DDPP has `paymentState: null`, `overdue: false`, and is excluded from the `paymentState` / `overdue=true` list
  filters. Payments on a DDPP → 409 `invalid_state` (mark-sent and the internal note stay allowed).

Advance lines
- Errors: `lines.N.advanceDocumentId` `required` (missing), `invalid` (unknown id; not issued; other currency; a DDPP
  of another or no contact; a DDPP on an invoice whose `vatMode` is not `standard` — the DDPP already taxed the
  advance, while the zero-VAT non-payer form is allowed in every mode; a proforma that is not this invoice's `relatedDocumentId` or whose payments issue DDPPs),
  `duplicate` (twice in one document, or already on another non-cancelled invoice — drafts included);
  `lines.N.kind: invalid` on a non-invoice. Advance lines are never subtotal members (`lines.N.refs: invalid`).
- The deducted amounts are a snapshot stored on the line at save and refreshed (and re-validated) at issue.
  Writes lock the referenced documents; if the paid amount of a non-payer proforma changed between validation and
  the write → 409 `conflict` (retry).
- `POST /api/documents/compute` accepts `documentId` (the edited draft: its type, related proforma and own
  references), `contactId`, `docType` (default: the draft's, else `invoice`) and `locale` (description language;
  default: the draft's, else the company's) — advance lines follow the same rules as on save. With `documentId` of
  a credit note the preview uses the invoice's rate (the request's `exchangeRate` is ignored), as save and issue do.
- Totals: recap rows cover every rate of items and advances. In non-`standard` modes advances deduct their base
  only (VAT 0). The non-payer deduction has no CZK amounts of its own and converts at the invoice's rate;
  `totalCzk = round2(payable × rate)` as in 1b. Response amounts are never `-0.00`.

DDPP
- Issued for every payment on such a proforma (overpayments too). Split edge cases: a tie for the highest gross
  goes to the higher rate (the highest-**gross** row takes the remainder, whatever its rate); zero shares are dropped; `ΣG ≤ 0` gives the whole payment to the highest-gross rate; a
  share ≥ 10^12 or an overflow → `amount: invalid` (payment not stored).
- `variableSymbol` = the proforma's; `constantSymbol`, `orderRef`, bank, payment method copied; no notes;
  `roundingMode` `none` (4e: the amounts are the payment, never rounded). Line `unitPrice` = `base_r`, `quantity` 1.
- The recap is produced by `src/document/ddpp.rs` and stored; a DDPP is created issued and never passes through
  save or issue, so nothing recomputes it from base × rate.
- Payment `exchangeRate` is validated on every payment (> 0, ≤ 6 dp) but only used for the DDPP of a
  foreign-currency proforma.
- Deleting the payment: the DDPP's `cancelReason` is `"Platba smazána"` (locale `en`: `"Payment deleted"`); the
  deleted payment's `paymentId` becomes `null` on the cancelled DDPP (FK `ON DELETE SET NULL`).
- Deleting a proforma payment whose DDPP (non-payer form: the proforma itself) is deducted by an **issued**,
  non-cancelled invoice → 409 `advance_settled`; by a **draft** invoice → 409 `advance_in_use` (remove the advance
  line from the draft first). Nothing changes in either case.

Settlement
- The draft: `issueDate = taxPointDate = today`, `dueDate` = today + (contact ?? company) due days, `exchangeRate`
  null (fixed at its own issue), `variableSymbol` null (derived at issue); `constantSymbol`, `orderRef`, header and
  footer notes, `roundingMode`, bank, payment method copied; the internal note is not.
- Payer case = the proforma's supplier snapshot `vatPayer` and `vatMode = "standard"`. DDPPs are added oldest first;
  one already on another non-cancelled invoice is skipped. Non-payer form: the advance line is added only when
  `paid > 0`; an unpaid proforma gives a plain copy.

Credit notes
- Not copied: `variableSymbol`, `constantSymbol`, `orderRef`, notes; `roundingMode` copied from the original (4e; a native DDPP's is `none`); `dueDate` = today + (contact ??
  company) due days — the same shared default as document create and settlement. Subtotal refs are renumbered after dropping advance lines.
- The cap compares with the invoice's **item** bases per rate (before advance deductions — the supply it
  documents) and sums the issued credit notes plus the one being issued (other drafts are not counted). Issue locks
  the invoice; an invoice cancelled meanwhile → 409 `invalid_state`. Only `invoice` documents can be credited.

Catalog
- Item: `unitPrice` required (4 dp, `|x| < 10^12`), `vatRate` required, `currency` default `CZK`, `unit` ≤ 20,
  `note` ≤ 2000, `active` default `true`; `?active=` filters both ways; `q` is trimmed.
- Group: `members.N.itemId` `required` / `duplicate` / `invalid` (no such item), `members.N.quantity` `required` /
  `invalid` (non-zero, 4 dp, `|x| < 10^12`), more than 1000 members → `members: too_long`. Member `position` is
  1..n in responses. Items of other currencies may share a group.
- Item `PUT` changing `vatRate` so that one of its groups would mix rates → 422 `{"fields":{"vatRate":"mixed_vat"}}`.
  Item `DELETE` removes it from its groups (cascade), except when it is the **last** member of a group → 409
  `catalog_item_in_use` (delete or edit the group first).
