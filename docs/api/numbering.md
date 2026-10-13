# API — per-client number sequences, manual numbers

User decision ([plan.md](../plan.md), Numbering): **not too restrictive — small companies make mistakes and must be
able to fix them.** The sequence is the primary path; a manual number exists to fill gaps and fix problems. Safety
comes from the unique number index and from allocation taking the next free number, not from extra 422 guards.

## Data
- `contacts.number_code` — the customer's `{C}`: `[A-Za-z0-9/_-]{1,10}`, stored as typed, unique per space
  **case-insensitively** (`contacts_number_code_key` on `(space_id, lower(number_code))`). Free to change or clear.
- `number_series.client_pattern` — issued series only; exactly one `{C}`, one year token, one `{N…}`; unique among
  the space's series (`clientPattern: duplicate`, like `pattern`).
- `contact_number_counters (space_id, contact_id, series_id, year, last_number)`, PK `(contact_id, series_id, year)`,
  composite space-scoped FKs to the contact and the series, both `ON DELETE CASCADE`.
- `documents.number_code` — informational: the code the number was allocated / matched under (`NULL` = the global
  series, an imported document or a manual number no pattern renders); allocation does not read it; `documents.previous_number` — the number an unlocked draft replaced at its
  re-issue; `documents.unlocked_number` (internal, not on the wire) — a native document's number at its latest
  unlock, cleared at re-issue: it tells "kept number" from "edited number".

## Pattern (`src/settings/pattern.rs`)
- Tokens `{YYYY}`, `{YY}`, `{N…}` and `{C}`. `Pattern::parse` (global: rejects `{C}`), `Pattern::parse_client`
  (exactly one `{C}`); `format(year, n, code)`.
- `matches(number, code)` reads a number back → `{ year, seq, code }`: `{YY}` → 2000 + yy; the sequence may be wider
  than its token (overflow keeps all digits) but only a number the pattern would render itself matches (no extra
  leading zeros); `{C}` matches `[A-Za-z0-9/_-]{1,10}`, or — given a code — that code case-insensitively.

## Allocation (`settings::repo::numbering`)
- The customer has a `numberCode` **and** the document's series has a `clientPattern` → the customer's client counter
  and the client pattern; otherwise the global counter and pattern (as before). DDPPs issued on a proforma payment
  use the proforma's customer the same way; received documents always use the global series.
- **Next free number after the counter** (user decision): candidate = counter + 1, then + 1 while the rendered
  number is already held by any document in the unique index's scope `(space, direction, docType, number)` — native,
  drafts holding a manual / kept number, cancelled, **imported** — compared case-insensitively. The counter is then
  set to the allocated sequence: only an allocation moves a counter, a manual number never does.
- In the issue transaction the counter row is created if missing and locked (`FOR UPDATE`) first, then the held
  numbers starting like the candidate (prefix up to the `{N…}` token) are loaded once into a set; concurrent issues
  of one counter serialize. The unique index and the issue's retry stay the backstop.
- Effects: a typo manual number (`20269999`) does not move the counter; an imported `20260005` is stepped over; a
  counter set back to 0 fills gaps (1, 2, 4 held → 3, then 5); a code given to another customer skips the numbers
  issued under it (any case); a renamed code continues the customer's counter; client numbers never touch the
  global counter.
- Counter `PUT`s (global and per customer) accept any value ≥ 0 — there is no `below_issued` guard any more.
  `nextNumberPreview` (settings, contact counters) and the number preview use the same computation, read-only.

## Manual number on a native draft
- `DocumentInput.number` on a native draft is tri-state: **omitted** = keep the draft's current number (nothing on
  create; an unlocked draft keeps its kept number); explicit `null` or `""` = clear it (allocate at issue); a string =
  manual number, trimmed, `[A-Za-z0-9/_-]`, ≤ 50 characters, else 422 `number: invalid`. (An exception to "`PUT`
  replaces", listed in [README.md](README.md#put-semantics).)
- The number is unique per document type like any number (unique index; drafts hold theirs, so allocation steps
  over it): a duplicate → 422 `number: taken` on save (or, after a race, on issue).
- At issue the number is read back for its year and sequence (they order documents in exports and lists): the client
  pattern with the customer's code first, then the global pattern, then the client pattern with any code. A match
  stores `numberYear`, the sequence and the code; no match → no sequence, no code, `numberYear` = the issue year. A
  manual number never moves a counter. An unlocked draft's unchanged number keeps the year, sequence and code it had.
- Imported documents are unchanged: own number required (`number: duplicate`), no sequence, no code.

## Unlocked draft renumbering
- It keeps its number unless the user edits or clears it. At re-issue, a number different from the one it was
  unlocked with (edited, or cleared → allocated afresh) stores the old one in `previousNumber`; a `variableSymbol`
  equal to `variable_symbol(old)` (derived) is re-derived from the new number, a hand-entered one is kept. A re-issue
  under the same number leaves `previousNumber` as it was.
- Note: `ACME-2026001` and `BETA-2026001` both derive the symbol `2026001` — accepted, the symbol is editable.

## Routes
```
NumberSeries { docType, pattern, clientPattern: string|null, counters: [{year, lastNumber}],
  nextNumberPreview: string }   // global series, current year, past the counter and every number held
ContactNumberSeries { docType, clientPattern: string|null, counters: [{year, lastNumber}],
  nextNumberPreview: string|null }   // null without a numberCode (or without a clientPattern)
NumberPreview { number: string, source: "client" | "global" }
```
- `PUT /api/settings/number-series/{docType}/client-pattern` body `{ clientPattern: string|null }` (**Manage**) →
  NumberSeries. Trimmed; empty / `null` clears. 422 `clientPattern: invalid_pattern | not_issued` (a received series
  key) `| duplicate` (another series of the space has it).
- `GET /api/contacts/{id}/number-counters` (**Read**) → `ContactNumberSeries[]`: every issued series with a client
  pattern, in series order; counters newest year first. Unknown contact → 404.
- `PUT /api/contacts/{id}/number-counters/{docType}/{year}` body `{ lastNumber }` (**Manage**) → ContactNumberSeries
  (upsert). `year` 1..9999 (`year: invalid`), `lastNumber` 0..2147483647 (`lastNumber: invalid`), a received series
  key → `docType: not_issued`; a series without a client pattern is accepted (`clientPattern: null`).
- `GET /api/documents/number-preview?docType=&contactId=&issueDate=` (**Read**) → NumberPreview: what an issue would
  allocate now — same choice and next-free computation as the allocation, nothing written. `docType` an issued document type
  (default `invoice`, else `docType: invalid`); `contactId` optional (unknown → `contactId: invalid`); `issueDate`
  default today (its year picks the counter).
- `Contact` / `ContactInput` + `numberCode: string|null` (422 `numberCode: invalid | duplicate`); the contact list
  `q` also matches the code. `Document` + `previousNumber: string|null`.
- MCP: `create_contact` takes `numberCode`; `get_settings` shows `clientPattern`. The draft tools never touch a
  number: `number` is not in their input schema and a sent one is ignored ([mcp.md](mcp.md)).

## Clarifications (as implemented)
- The third matching step (client pattern with any code) only records where a manual `BETA-…` number on another
  customer's document belongs; allocation steps over it either way (held numbers).
- A manual number is reserved from the first save (the unique index covers drafts), so a duplicate is `number: taken`
  already on `POST` / `PUT`; the issue maps a race on it to the same field error. An allocated number that loses a
  race stays 409 `number_taken` / the issue's retry.
- New reason codes: `taken`, `not_issued`; `duplicate` also for `clientPattern`. `below_issued` is gone.
