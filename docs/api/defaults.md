# Document defaults and rounding modes (4e)

Per-space defaults for new documents and a rounding **mode** replacing the `roundTotal` boolean (#3 comments). One
space = one company, so the defaults live on the company profile (Settings → Firma), next to the existing
`defaultDueDays` / `defaultLocale` ([settings-contacts.md](settings-contacts.md)). The **accounting currency stays
CZK** for every space; a non-CZK accounting currency is out of scope (see `docs/plan.md`).

## Rounding mode
`roundingMode: "none" | "half" | "unit"` on documents (replaces `roundTotal`, which is **removed** from every request
and response: documents, compute, MCP tools, drafts):
- `none` → `payable = total`, `rounding = 0`;
- `half` → `payable` = `total` rounded to the nearest **0.50** of the document currency (half away from zero);
- `unit` → `payable` = `total` rounded to whole units (half away from zero) — today's `roundTotal: true`.
- Allowed for **every currency** (the 1b "CZK only" rule is dropped). Credit notes / corrections round
  symmetrically (the sign is applied after rounding). Advance deductions are applied before rounding exactly as
  today (`payable` = rounded(total − deductions)); the existing formulas otherwise stay.
- `rounding` (the amount) and `payable` in responses are unchanged.
- **Received / imported documents** (ISDOC, CSV, manual received) keep their rounding *amount* exactly as given;
  their `roundingMode` is derived on save: `none` when the amount is 0, `unit` / `half` when `total + rounding` is a
  whole / 0.50 multiple and `rounding ≠ 0`, else `none` with the stored amount kept (an "odd" rounding; the mode
  is informational for received documents). Accounting exports keep deriving Pohoda's `roundingDocument` from the
  amounts (3b), so nothing changes there; Money keeps the amount in `Zaklad0` (3c).
- Migration (append-only): `round_total` → `rounding_mode` (`true` → `unit`, `false` → `none`; received rows by the
  rule above); the old column is dropped. CSV import / export columns are unchanged (they carry the amount).
- Validation: unknown mode → 422 `roundingMode: invalid`.

## Company defaults (Settings → Firma, section "Výchozí hodnoty dokladů")
`Company` gains (all optional in the stored row, with the defaults below):
```
defaultRoundingCash: "none" | "half" | "unit",     // default "unit"  — paymentMethod cash
defaultRoundingOther: "none" | "half" | "unit",    // default "none"  — bank_transfer, card, other
defaultPaymentMethod: "bank_transfer" | "cash" | "card" | "other",   // default "bank_transfer"
defaultCurrency: string,                           // ISO 4217, default "CZK"; must be a currency the app supports
defaultHeaderNote: { cs: string|null, en: string|null },   // ≤ 2000 each, default nulls
defaultFooterNote: { cs: string|null, en: string|null },   // ≤ 2000 each
```
- `PUT /api/settings/company` accepts them; for backward compatibility of the PUT rule ("vatPayer, defaultDueDays,
  defaultLocale must be present") the new fields are **optional** in the body — absent = keep the stored value.
  422: `defaultRoundingCash|defaultRoundingOther: invalid`, `defaultPaymentMethod: invalid`, `defaultCurrency:
  invalid`, `defaultHeaderNote.cs|en`, `defaultFooterNote.cs|en: too_long`.
- Existing company rows get the defaults above in the migration (there is no production data yet, so cash rounding
  `unit` applies everywhere).

## Defaults on create (POST, fields omitted or null) — replaces the 1b paragraph
- `paymentMethod` = `company.defaultPaymentMethod`.
- `roundingMode` = `paymentMethod == cash ? company.defaultRoundingCash : company.defaultRoundingOther`.
- `currency` = `contact.defaultCurrency ?? company.defaultCurrency`.
- `locale` = `contact.defaultLocale ?? company.defaultLocale` (unchanged); `headerNote` / `footerNote` =
  `company.defaultHeaderNote[locale]` / `defaultFooterNote[locale]` (null when empty).
- Everything else as in [documents.md](documents.md). `PUT` still applies no defaults. Imports (ISDOC, CSV) and the
  correction / settlement / DDPP generators keep their own rules (they copy from the source document), except that
  a generated DDPP / credit note takes the source's `roundingMode`.
- MCP `create_draft`: same defaults (it goes through the same create path); its schema replaces `roundTotal` by
  `roundingMode`.

## UI
- Document editor: "Zaokrouhlení" select (Bez zaokrouhlení / Na 0,50 / Na celé jednotky) instead of the checkbox;
  on a **new draft** changing the payment method re-applies the company default for the new method **unless the
  user changed the rounding select manually** in this draft. Live totals via compute as today.
- New document form starts from the server defaults (the editor shows what POST would apply: it reads the company
  profile and the selected contact; header / footer notes prefilled for the chosen locale and replaced when the
  locale changes, unless edited).
- Settings → Firma: "Výchozí hodnoty dokladů" section: payment method, currency, rounding for cash / other,
  header and footer note per language (cs / en tabs).
- Received document form and detail show the mode read-only (the amount stays editable as today).

## Tests
- Unit: rounding to 0.50 / unit / none incl. negatives and half-way cases, all currencies; mode derivation for
  received amounts.
- Integration: create defaults (payment method, rounding by method, currency precedence contact > company, notes
  by locale), PUT keeps no defaults, company PUT validation and partial body, migration mapping, MCP draft with
  `roundingMode`, issue / PDF / ISDOC export with `half`, credit note and DDPP inheriting the mode, received
  imports deriving the mode.
- Frontend: rounding select, method change re-applying the default unless edited, notes per locale, settings
  section.

## Clarifications (as implemented)
(filled during the 4e implementation)
