# API — settings, contacts, ARES (phase 1a)

Conventions and error shapes: [README.md](README.md).

## Company profile (singleton) — `GET /api/settings/company`, `PUT /api/settings/company`
```
Company {
  name: string,            // required, <=200
  ico: string | null,      // 8 digits, checksum validated (mod 11 rule)
  dic: string | null,      // e.g. "CZ12345678" / "CZ1234567890", <=14
  vatPayer: boolean,
  street: string, city: string, zip: string,   // may be "" until filled
  country: string,         // ISO-3166 alpha-2, default "CZ"
  email: string | null, phone: string | null, web: string | null,
  registration: string | null,  // e.g. "Fyzická osoba zapsaná v živnostenském rejstříku", <=300
  defaultDueDays: number,  // 0..365, default 14
  defaultLocale: "cs" | "en",  // default "cs"
  // Document defaults (4e, see defaults.md); optional in the PUT body, absent / null = keep
  defaultRoundingCash: "none" | "half" | "unit",    // default "unit"
  defaultRoundingOther: "none" | "half" | "unit",   // default "none"
  defaultPaymentMethod: "bank_transfer" | "cash" | "card" | "other",  // default "bank_transfer"
  defaultCurrency: string,                          // ISO 4217, default "CZK"
  defaultHeaderNote: { cs: string|null, en: string|null },  // <=2000 each
  defaultFooterNote: { cs: string|null, en: string|null }
}
```
GET returns the row; a migration seeds an empty row (name "") so GET never 404s. PUT replaces all profile fields
(the document defaults only when present), returns the saved object. `vatPayer`, `defaultDueDays` and
`defaultLocale` are required in the PUT body: a missing one is 422 `{"<field>":"required"}` (not a 400).

## Bank accounts — `/api/settings/bank-accounts`
```
BankAccount { id, label: string|null, currency: string /* ISO 4217, 3 upper letters */,
  accountNumber: string|null /* CZ format "[prefix-]number/bank", validated loosely: ^(\d{1,6}-)?\d{2,10}/\d{4}$ */,
  iban: string|null /* normalized: uppercase, no spaces; checksum mod-97 validated */,
  bic: string|null, isDefault: boolean }
```
At least one of accountNumber/iban required. `GET` list (ordered currency, isDefault desc, label), `POST` create, `PUT /{id}`, `DELETE /{id}` (204).
`isDefault=true` makes it the only default for its currency (unset others in the same transaction). The first account of a currency becomes default automatically.

## VAT rates — `/api/settings/vat-rates`
```
VatRate { id, rate: string /* decimal 0..100, max 2 dp */, label: string /* <=100 */, isDefault: boolean, active: boolean, position: number }
```
Seeded by migration: 21 "Základní" (default), 12 "Snížená", 0 "Nulová". GET list ordered by position. POST/PUT/DELETE like bank accounts. Exactly one default at a time (setting one unsets others). Rate unique among rows (422 duplicate). Invoice lines will store the rate VALUE, not an FK, so delete is always allowed.

## Number series — `/api/settings/number-series`
Doc types (enum, snake_case strings): `invoice`, `credit_note`, `proforma`, `advance_tax_doc`, `received`.
```
NumberSeries { docType, pattern: string, counters: [{ year: number, lastNumber: number }], nextNumberPreview: string /* for current year */ }
```
Seeded: invoice `{YYYY}{NNNN}`, credit_note `D{YYYY}{NNNN}`, proforma `Z{YYYY}{NNNN}`, advance_tax_doc `DP{YYYY}{NNNN}`, received `P{YYYY}{NNNN}`.
- `GET /api/settings/number-series` → list of all 5.
- `PUT /api/settings/number-series/{docType}` body `{pattern}` → saved NumberSeries.
- `PUT /api/settings/number-series/{docType}/counters/{year}` body `{lastNumber}` (>=0) → saved NumberSeries (upsert).
Pattern rules: tokens `{YYYY}`, `{YY}`, `{N…}` (1–9 N's = zero-padded width); exactly one `{N…}` token and exactly one year token (`{YYYY}` or `{YY}`) required — counters reset per year; other chars literal `[A-Za-z0-9/_-]`; <=40 chars; else 422 `invalid_pattern`.
Backend exposes a pure `Pattern::parse(&str)` + `Pattern::format(year, n)` with unit tests. Allocation (next number in a transaction, `SELECT … FOR UPDATE`/upsert) is also implemented now as `allocate_number(txn, doc_type, year)` with an integration test (concurrent allocations yield distinct consecutive numbers), but no route uses it yet.

## Contacts — `/api/contacts`
```
Contact { id, name: string /* required <=200 */, ico: string|null, dic: string|null,
  street, city, zip: string, country: string /* default "CZ" */,
  email: string|null, phone: string|null, note: string|null,
  defaultDueDays: number|null, defaultLocale: "cs"|"en"|null, defaultCurrency: string|null,
  createdAt, updatedAt }
```
- `GET /api/contacts?q=<text>&limit=50&offset=0` → `{ items: Contact[], total: number }`; q matches name/ico/dic/city case-insensitively (ILIKE). Ordered by name.
- `POST`, `GET /{id}`, `PUT /{id}`, `DELETE /{id}` (204).
- `ico` unique when not null → 422 `{"fields":{"ico":"duplicate"}}`.

## ARES — `GET /api/ares/{ico}`
Calls `GET {INVOICEAPP__ARES_URL}/ekonomicke-subjekty/{ico}` (default base `https://ares.gov.cz/ekonomicke-subjekty-v-be/rest`), 10 s timeout. Maps to a contact draft (no id):
```
AresSubject { name, ico, dic|null, street, city, zip, country: "CZ" }
```
Mapping from ARES JSON: `obchodniJmeno` → name, `ico`, `dic`, `sidlo.textovaAdresa` fallback; street = `nazevUlice` + " " + `cisloDomovni` [+ "/" + `cisloOrientacni` + `cisloOrientacniPismeno`] (if no street, use `nazevObce`/`nazevCastiObce` + number), city = `nazevObce`, zip = `psc` as 5-digit string. ARES 404 → `ares_not_found`; network/5xx/parse error → `ares_unavailable` (log cause). Tests use a local mock HTTP server (axum on 127.0.0.1:0) with fixture JSON, configured via the env/config base URL — never hit the real ARES in tests.

## Clarifications (as implemented)
Additive details settled during the Phase 1a backend; nothing above changed.

General
- `POST` creates return **201** with the created object; `PUT` returns 200 with the saved object; `DELETE` 204 with no body.
- **400 `{"code":"bad_request"}`**: malformed JSON, a wrong JSON type (e.g. `vatPayer: "yes"`), or an unparsable query parameter (`limit=abc`).
- A path id that is not a UUID (or an unknown `docType`, or a non-numeric `year`) → 404 `not_found`.
- 409 `conflict` only arises from a race on a unique index (e.g. two concurrent "make default" writes).
- Normalization before validation: strings are trimmed; `""` in a nullable field is stored as `null`; IČO has whitespace removed;
  DIČ, IBAN, BIC have whitespace removed and are uppercased; `country`/`currency` are uppercased; empty `country` → `"CZ"`.
- Length limits (`too_long`): name 200, street 200, city 100, zip 20, phone 50, email 200 (must contain `@`, else `invalid`),
  web 200, registration 300, contact note 2000, bank `label` 100, VAT `label` 100.
- DIČ: two letters + 2–12 alphanumerics, ≤14 (`invalid` / `too_long`) — covers CZ and other EU VAT ids.

Request bodies (response shape minus server fields)
- `BankAccountInput { label?, currency, accountNumber?, iban?, bic?, isDefault? = false }`; both `accountNumber` and `iban`
  missing → `{"accountNumber":"required"}`. BIC: 8 or 11 alphanumerics.
- `VatRateInput { rate: string, label, isDefault? = false, active? = true, position? }`; `position` omitted → appended after the
  last on create, unchanged on update. `rate` is returned normalized (`"21.00"` → `"21"`).
- `ContactInput` = `Contact` without `id`, `createdAt`, `updatedAt`; `PUT` replaces every field (omitted → empty/null).
- `Company` PUT body = `Company`; `vatPayer`, `defaultDueDays`, `defaultLocale` must be present (else 400).

Defaults
- A non-empty group always keeps exactly one default (per currency for bank accounts, global for VAT rates). Un-flagging the
  default or deleting it promotes another (bank: oldest account of the currency; VAT: first by position), preferring a row
  other than the one just un-flagged — so un-flagging the only row keeps it default. Moving an account to another currency
  re-checks both currencies.
- The default VAT rate is always active: `isDefault: true` with `active: false` → `{"isDefault":"invalid"}`. Deactivating
  the current default promotes the first other active rate by position; if no other rate is active the update is rejected
  with the same 422. Promotion only ever picks active rates, so with no active rate left (e.g. after a delete) there is no
  default.

Number series
- `counters` are ordered newest year first. `nextNumberPreview` uses the server's current local year.
- Counter `PUT`: `year` 1..9999 (else `{"year":"invalid"}`), `lastNumber` 0..2147483647 (else `{"lastNumber":"invalid"}`).
- A pattern without a year token (or with two) is `invalid_pattern`. A pattern equal to another doc type's pattern →
  `{"pattern":"duplicate"}` (both series would issue the same numbers).
- Rust API: `Pattern::parse(&str) -> Result<Pattern, InvalidPattern>`, `Pattern::format(year, n) -> String`;
  `allocate_number(&txn, DocType, year) -> Result<String, AppError>`. A sequence wider than its `{N…}` token keeps all digits.

Contacts
- `limit` defaults to 50 and is clamped to 1..200; `offset` defaults to 0 and is clamped to 0..=i64::MAX (a negative or
  non-numeric value is 400 `bad_request`). `q` is trimmed; `%`, `_`, `\` match literally.

ARES
- The IČO in the path may contain spaces (removed). A record without `obchodniJmeno` or an unparsable body → `ares_unavailable`.
- Street: `nazevUlice` + `cisloDomovni`[`/cisloOrientacni` + `cisloOrientacniPismeno`]; without a street name the number is
  prefixed with `nazevCastiObce`, else `nazevObce`; with no structured address at all, `textovaAdresa`. `psc` as a number is
  zero-padded to 5 digits; `country` is always `"CZ"`.

---

