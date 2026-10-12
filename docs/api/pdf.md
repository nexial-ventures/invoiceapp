# PDF (phase 1d)

Documents are rendered to PDF by the mdcast service (`POST {INVOICE__MDCAST_URL}/v1/render/template`, crate
`mdcast-client` 0.4.2): one typst template (`invoice.typ`) + a JSON payload the template reads with
`json("/data.json")` + an asset bundle (design files, fonts, the generated QR code).

## Design files

- The default design is embedded in the binary from the repo directory `design/`: `invoice.typ`, `fonts/Inter-*.ttf`
  (+ `fonts/OFL.txt`). Minimalist layout, black/grey, one accent colour, no logo.
- The storage keys `design/…` (see [Storage](#storage)) override it **file by file**: the effective set is embedded ∪
  `design/`, the stored file wins on the same relative path (`design/logo.svg` → `logo.svg`). The prefix listing of a
  render is reused for up to 30 s per space and dropped by an activation / restore / reset (design edits apply
  without a restart; a CLI `design push` / `rm` or a direct bucket write shows within 30 s; a listed file
  deleted meanwhile makes the render list again once instead of failing). The invalidation is in-process: one server
  instance is assumed; another instance would see an activation within 30 s); file contents are cached in memory by key + version (ETag,
  size, modification time from the listing), so only new or changed files are downloaded. Hidden files
  (`.`-prefixed path segments) are ignored; a file > 10 MB → render fails (`pdf_render_failed`, checked on the
  listed size before any download). Files get there with `invoice design push <dir>` (or straight into the bucket);
  `invoice design pull <dir>` / `ls` / `rm <path>…` complete the set.
- Every `fonts/**/*.ttf|*.otf` is registered with the typst font book (request `fonts`, sorted by path); every other
  file goes into the bundle under its relative path, so `#image("logo.png")` / `#import "parts.typ"` resolve like on a
  filesystem.
- Optional well-known files: `logo.svg` or `logo.png` (svg wins), `signature.png` (stamp/signature above the
  "Issued by" line). The template cannot test file existence, so the payload carries `assets.logo` /
  `assets.signature` = the relative path or `null`.
- `qr.svg` is reserved: generated per render, it replaces any design file of that name.

## Payload (`/data.json`)

Every displayed value is a **pre-formatted string** in the document's locale (the template does no number/date
formatting); labels are resolved server-side, so the template carries no hardcoded text. Missing values are `null`.

```
{
  locale: "cs" | "en",
  draft: boolean,                       // true → template draws the "NÁVRH" / "DRAFT" watermark
  docType: "invoice" | "proforma" | "credit_note" | "advance_tax_doc",
  vatMode: "standard" | "reverse_charge" | "exempt" | "non_payer",
  title: string,                        // see Titles
  number: string | null,                // null on a draft
  supplier: Party, customer: Party,     // issued: the snapshots; draft: live company / contact
  dates: [ { label, value } ],          // ordered rows: issue date, tax point (DUZP), due date, payment date (DDPP)
  payment: [ { label, value } ],        // ordered rows: method, account number, IBAN, BIC, VS, KS, order ref
  headerNote: string | null, footerNote: string | null,
  reference: string | null,             // credit note: "Opravný daňový doklad k faktuře 20260001" + reason line;
                                        // DDPP: "K zálohové faktuře Z20260003"; settled invoice lists nothing here
  showVat: boolean,                     // false for non_payer: VAT columns + recap hidden
  columns: { description, quantity, unitPrice, discount, vatRate, base, … },   // header labels
  hasDiscount: boolean,                 // false → discount column hidden
  lines: [ Line ],
  vatRecap: { title, rows: [ { rate, base, vat, total } ], total: { base, vat, total } } | null,
  vatRecapCzk: { title, rateNote, rows: [ … ], total } | null,   // foreign currency + standard VAT only (§ 37)
  totals: [ { label, value, strong: boolean } ],  // total excl. VAT, VAT, total, advances deducted, rounding, payable
  legalNote: string | null,             // by vatMode, see Legal texts
  paidNote: string | null,              // DDPP: "Neplaťte – již uhrazeno."
  qr: { image: "qr.svg", label: "QR platba" } | null,
  assets: { logo: string | null, signature: string | null },
  footer: { registration: string | null, pageLabel: "Strana" | "Page", issuedBy: "Vystavil" | "Issued by" }
}
Party { title /* "Dodavatel"/"Odběratel" */, name, lines: [string] /* street, "zip city", country name unless CZ */,
        ids: [string] /* "IČO: …", "DIČ: …" — only present ones */, contact: [string] /* email, phone, web */ }
Line  { kind: "item" | "text" | "subtotal" | "advance", description, quantity /* "2 ks" */, unitPrice, discount,
        vatRate /* "21 %" */, base, strong: boolean }
```

- Lines follow the document order; members of a **collapsed** subtotal are omitted, the subtotal itself is printed
  as a regular row (`kind: "subtotal"`, its quantity/unit price `null`). Non-collapsed subtotals print as a bold
  summary row after their members. `text` lines carry only `description`. Advance (deduction) lines print with a
  negative base. Credit-note amounts print **negated** (the stored positive values × sign).
- Money: `cs` → `1 234,50 Kč` (NBSP thousands, comma decimal, currency symbol for CZK/EUR/USD else the code after),
  `en` → `CZK 1,234.50`. Quantities trimmed of trailing zeros. Dates: `cs` `8. 10. 2026`, `en` `8 Oct 2026`.
- Exchange rate note (`vatRecapCzk.rateNote`): `Kurz ČNB 24,400 CZK/EUR ze dne 7. 10. 2026` (source `manual` →
  "Kurz 24,400 CZK/EUR", `original` → same as cnb of the original).

### Titles
| docType | cs | en |
|---|---|---|
| invoice (standard / reverse_charge / exempt) | Faktura – daňový doklad | Invoice – tax document |
| invoice (non_payer) | Faktura | Invoice |
| proforma | Zálohová faktura | Proforma invoice |
| credit_note | Opravný daňový doklad | Credit note |
| advance_tax_doc | Daňový doklad k přijaté platbě | Tax document for a received payment |

The tax point (DUZP) row is omitted for proformas and non-payers; the due date row is omitted for DDPPs.

### Legal texts
- `reverse_charge`: cs "Daň odvede zákazník (přenesení daňové povinnosti, § 92a zákona o DPH)." / en "Reverse charge –
  VAT to be accounted for by the customer."
- `exempt`: cs "Plnění osvobozené od DPH." / en "VAT exempt supply."
- `non_payer`: cs "Dodavatel není plátcem DPH." / en "The supplier is not a VAT payer."

### SPAYD QR
Present only when **all** hold: not a draft, `docType ∈ {invoice, proforma}`, `paymentMethod = bank_transfer`, the bank
snapshot has an IBAN, `payable > 0`. Generated server-side as SVG (crate `qrcode`), string:
`SPD*1.0*ACC:{IBAN}[+{BIC}]*AM:{payable, 2 dp, '.'}*CC:{currency}*DT:{dueDate YYYYMMDD}[*X-VS:{vs}][*X-KS:{ks}]*MSG:{title number, ≤60 chars}`;
`*` inside values is stripped, MSG is ASCII-folded (no diacritics).

## Archive

- Columns on `documents`: `pdf_path text null` (the storage key `documents/{numberYear}/{id}-{sha8}-{nonce}.pdf`,
  first 8 hex digits of the sha256 + 8 random hex digits, a new key per write attempt; archives written before the
  change keep `documents/{numberYear}/{id}.pdf` — `pdf_path` is what
  counts), `pdf_sha256 text null`, `pdf_rendered_at timestamptz null`. Written atomically (see [Storage](#storage)).
- **Issue** (`POST /api/documents/{id}/issue`, all doc types) never holds a transaction, row lock or pooled
  connection across the mdcast render or the storage write: (1) a dry-run transaction (always rolled back) applies
  the issue — number, snapshots, totals — and builds the payload exactly as it will be committed; (2) the payload is
  rendered and the PDF put under its own new key, outside any transaction; (3) a short transaction applies
  the issue again (row lock, `updated_at` check, number allocation) and commits only when it yields the same payload,
  recording the archive in the same commit. Render or storage failure → nothing is issued, no number used (503
  `pdf_unavailable` / 502 `pdf_render_failed` / 503 `storage_unavailable`). The draft edited or the number taken by
  another issue while rendering → the object is removed and the issue starts over with the current data (one
  re-render for an edit; up to 5 re-renders, 20–120 ms apart, when only the number was taken by a concurrent issue of
  the series); edited again → 409 `conflict`, nothing issued. A failed final transaction or commit → the written
  object is removed (best effort, only when no row points at it).
- **DDPP** (auto-issued with a payment): the payment never fails because of PDF. After the payment transaction
  commits, the handler tries to render + archive once (failure logged). A DDPP without an archive is rendered and
  archived on its first `GET …/pdf` (render and object write outside any transaction, then `UPDATE … WHERE
  pdf_path IS NULL`; the loser of concurrent first downloads removes its object and serves the winner's).
- The archive is **immutable**: never re-rendered, also not after cancel or a design change.

## Routes

- `GET /api/documents/{id}/pdf?download=1` → `200 application/pdf`, `Content-Disposition: inline` (`attachment` with
  `download=1`), filename `{number}.pdf` (draft: `draft-{first 8 chars of id}.pdf`), `Cache-Control: no-store`.
  - draft → rendered live, `draft: true`, never stored — a **preview** ([Render limits](#render-limits));
  - issued / cancelled with an archive → the stored file, streamed with `Content-Length` (sha256 not re-checked);
  - issued / cancelled without an archive (DDPP only) → render + archive, as above;
  - archive object missing → 500 (logged); storage unreachable → 503 `storage_unavailable`.
- `GET /api/pdf/preview?locale=cs|en` → `200 application/pdf` of a sample invoice (never stored): supplier = the
  company profile (placeholder name when empty), fictional customer, CZK, standard VAT, items at 21 % and 12 %, one
  with a 10 % discount, a collapsed subtotal group, a text line, bank = default CZK account (if any → QR shown),
  `draft: false`, number `{YYYY}0001`-style sample. `locale` missing → company `defaultLocale`; invalid → 422. A
  **preview** ([Render limits](#render-limits)).
- `GET /api/pdf/design` → `{ storage: "fs" | "s3", files: [ { path, source: "custom" | "default", size } ] }`
  (effective set, sorted by path; `custom` = from the storage's `design/`).
  4f extends this response with `draft` and `quota` and adds draft / activation / version routes (admin+):
  [design.md](design.md). Renders (issue, preview) read the live set only; the draft preview and the activation /
  restore checks render with an overlay (`design::Layers`; draft and version files bypass the cache, only live files
  are cached), and an activation / restore / reset invalidates the space's design cache.

Fetch the PDF with the Bearer header (blob → object URL); a plain link cannot carry the token.

## Render limits

- **Instance-wide**: at most `INVOICE__MDCAST_CONCURRENCY` (default 4) mdcast renders run at once (a semaphore around
  the mdcast call; design listing / downloads happen before it). A render waits at most 10 s for a slot, then 503
  `{"code":"pdf_unavailable","detail":"busy"}` + `Retry-After: 10`. This applies to every render, issue included.
- **Previews** — `GET /api/pdf/preview`, `GET /api/pdf/design/draft/preview`, `GET /api/documents/{id}/pdf` of a
  draft — are additionally limited **per space**: one in flight (a second concurrent one → 429 `rate_limited`,
  `Retry-After: 1`) and 60 per hour (bucket `preview-space`, in memory per instance; then 429 `rate_limited` +
  `Retry-After` until the oldest leaves the hour). Both are checked before the design is loaded; a preview refused
  by a full render queue (503 busy) does not count against the hour. A preview's mdcast call times out after
  **20 s** (→ 503 `pdf_unavailable`); issue, archive-on-download and the design activation / restore checks keep
  the 60 s client timeout.
- **Issuing is never refused by the preview limits** (only the instance-wide slots apply), nor is an archive render
  on first download, an e-mail attachment or an ISDOC export (they serve the archive). The activation / restore
  checks have their own `design-space` limit ([design.md](design.md)).

## Errors
- 503 `{"code":"pdf_unavailable"}` — mdcast unreachable, token rejected, gateway 502–504, a preview over its 20 s;
  with `"detail":"busy"` + `Retry-After: 10` when every render slot stayed taken for 10 s.
- 429 `{"code":"rate_limited"}` + `Retry-After` — a preview over the per-space limits.
- 503 `{"code":"storage_unavailable"}` — the storage (directory or bucket) failed: design listing / download, archive
  write or read. The cause is logged only.
- 502 `{"code":"pdf_render_failed","detail":"…"}` — mdcast answered but the template failed, or a design file is too
  large. `detail` is mdcast's message: mdcast sanitises it to `render failed (incident <id>)` — the typst diagnostics
  are only in mdcast's logs, found by the incident id (no DB/internal data either way).
- Document `pdf` field (added to the Document DTO): `{ sha256, renderedAt } | null`.

## Configuration
| Env var | Default | Notes |
|---|---|---|
| `INVOICE__MDCAST_URL` | `https://mdcast.nexial.cz` | empty → default |
| `INVOICE__MDCAST_TOKEN` | — | optional Bearer for mdcast |
| `INVOICE__MDCAST_CONCURRENCY` | `4` | concurrent mdcast renders of the instance (≥ 1), see [Render limits](#render-limits) |
| `INVOICE__STORAGE_KIND` | `fs` | `fs` or `s3` (see [Storage](#storage)) |
| `INVOICE__STORAGE_DIR` | `./data` | `fs` root, created at start; Docker image uses `/data` (volume) |
| `INVOICE__S3__ENDPOINT` / `BUCKET` / `REGION` / `ACCESS_KEY_ID` / `SECRET_ACCESS_KEY` / `PATH_STYLE` | — / — / `us-east-1` / — / — / `false` | `s3` only; endpoint empty → AWS |

mdcast HTTP timeout 60 s (previews 20 s).

## Storage

All files (archives, originals, design overrides) go through one `Storage` (`src/storage/`, crate `object_store`):
put (returns sha256; atomic — `fs` writes a temp file, fsyncs it, renames and fsyncs the directory; an S3 PUT is
atomic), get (whole or streamed), delete (idempotent), exists / head, list by prefix. Keys are `/`-separated, never
contain empty, `.` or `..` segments: `documents/{year}/{id}-{sha8}-{nonce}.pdf` (older archives `{id}.pdf`), `documents/{year}/{id}-original-{sha8}-{nonce}.pdf`,
`design/…`.

- `fs` (default): `INVOICE__STORAGE_DIR`. `s3`: any S3-compatible service; bucket, access key id and secret are
  required, `PATH_STYLE=true` for Garage / MinIO. Invalid settings (unknown kind, missing bucket / credentials,
  endpoint without `http(s)://`, unparsable `PATH_STYLE`) → the server refuses to start. Reachability is probed once
  at start and only logged.
- S3 requests: 5 s connect timeout, 30 s read timeout (each wait for headers or the next chunk; no total timeout, so a
  slow 20 MB download is not cut off), at most 2 retries within 15 s — then 503 `storage_unavailable`.
- Virtual-hosted style (`PATH_STYLE=false`) with a custom endpoint puts the bucket into the host
  (`https://s3.example` → `https://{bucket}.s3.example`; unchanged when the host already starts with `{bucket}.`).
- A leftover `INVOICE__DESIGN_DIR` refuses the start (its files would silently stop applying).
- A failed issue, original upload or ISDOC / CSV import (write, final transaction or commit) removes the object it
  wrote, unless a document row points at that key. Every write attempt gets its own key (`…-{sha8}-{nonce}.pdf`), so
  the cleanup never removes an object another attempt (a double-clicked issue, an identical upload) just wrote.
- **Crash orphans**: objects are written before the transaction that references them, so a crash (or a lost
  connection) between the put and the commit leaves an unreferenced object behind (harmless, never served). The
  **orphan sweep** (`src/storage/sweep.rs`) runs in `serve` right after start and then daily, and on demand as
  `invoice storage sweep [--dry-run]` (needs the database): it lists `spaces/*/documents/`, then reads every
  `documents.pdf_path` / `original_path`, and deletes the objects no row references whose modification time is
  more than **1 hour** old (younger ones may belong to an issue or upload still before its commit). Nothing else is
  touched or recomputed (stored-byte usage comes from the rows, so it does not change). It logs the counts (examined,
  too young, deleted, failed); the CLI prints each orphan (`--dry-run`: lists only) and exits 1 when a deletion
  failed.
- `invoice storage migrate --from-dir <old INVOICE__STORAGE_DIR> [--design-dir <old INVOICE__DESIGN_DIR>]` copies
  every non-hidden file into the configured backend (keys = relative paths; the design dir under `design/`), reads
  each one back and compares sha256. Identical objects are skipped (re-runs are no-ops; decided by size + the MD5
  ETag of single-part S3 objects without a download, by downloading when the ETag is no MD5, e.g. fs); an existing
  object with different content is reported and kept; unreadable local entries (a dangling symlink, an unreadable
  `lost+found`) are skipped and reported. Any problem → exit code 1 after the rest is copied. The source is never
  modified. `invoice design rm` of a path that is not stored prints "not found" and exits 1.

## Clarifications (as implemented)

- **Party snapshots** gained optional `email`, `phone`, `web` (company: all three, contact: email + phone); snapshots
  from before 1d read them as `null`. `Party.contact` is built from them; `Party.lines` adds the country name (16
  common codes in cs/en, else the code) unless `CZ`. A draft without a contact gets an empty customer party.
- **Extra payload fields** (so the template stays free of text): `watermark` (`"NÁVRH"`/`"DRAFT"` on a draft,
  `"STORNO"`/`"CANCELLED"` on a cancelled document rendered without an archive, else `null`); recaps carry `columns: { rate, base, vat, total }` header labels and `rateNote` (`null` on `vatRecap`);
  a recap `total` row's `rate` holds the "Celkem"/"Total" label; `columns.base` is "Základ"/"Net", for a non-payer
  "Celkem"/"Amount".
- **Empty env values** mean the default: `INVOICE__MDCAST_URL`, `INVOICE__MDCAST_TOKEN`, `INVOICE__STORAGE_KIND`,
  `INVOICE__STORAGE_DIR`, every `INVOICE__S3__*`.
- **Formatting**: negative money `-1 234,50 Kč` / `CZK -1,234.50`; `en` percent without a space (`21%`), `cs` with
  NBSP (`21 %`); dates use plain spaces; exchange rates print with at least 3 decimals.
- **Lines**: collapsed subtotals hide their members recursively (a member that is itself a subtotal hides its own
  members too). `vatRate` is `null` when `showVat` is false; `discount` is `null` for 0 %. An advance line prints as
  one row (description + negative base, no rate). Credit notes negate unit price, base, recaps and totals; the
  quantity stays positive.
- **Totals rows**: stored totals are net of advance deductions, so with advance lines the rows show total excl. VAT /
  VAT / total incl. VAT *before* the deduction, then "Odpočet záloh" (negative gross), then rounding (only when
  non-zero) and the payable (`strong`). Non-payer: one "Celkem" row instead of the three. DDPP: excl. VAT, VAT,
  total incl. VAT (`strong`), no payable row. The payable label is "K úhradě"/"Amount due" for every type (negative
  on a credit note).
- **Rows**: `payment` lists account number / IBAN (grouped by 4) / BIC only for `bank_transfer`; VS, KS and order
  reference whenever set. The DDPP "payment date" row is its tax point date.
- **Exchange-rate note** for `original` (credit notes): the original invoice's source and date are used (ČNB → dated
  note, manual → "Kurz …").
- **Archive on first download** applies to any issued/cancelled document without an archive (DDPPs, and documents
  issued before 1d), not only DDPPs. A cancelled one renders with the "STORNO" watermark and never a QR (the QR rule
  requires status `issued`). The DDPP archive after a payment runs in a background task once the payment has
  committed — the 201 is not delayed; a failure is only logged. The loser of the `WHERE pdf_path IS NULL` race
  serves the winner's stored file (not its own render), so every download returns the archived bytes.
- **Download**: `download=1` or `download=true` → `attachment`. The filename stem is reduced to `[A-Za-z0-9._-]`
  (others → `_`). A missing archive file → 500 `{"code":"internal"}`.
- **Error detail**: `detail` is only returned for genuine template failures (mdcast `render_failed`, whose message
  mdcast sanitises to `render failed (incident <id>)`) and our own design checks (file > 10 MB, non-UTF-8 `invoice.typ`). Any other
  mdcast failure (its 500 / `internal`, `payload_too_large`, an undecodable or unexpected answer) is 502
  `pdf_render_failed` **without** `detail`; the upstream body is logged only.
- **Design**: font files are bundled too (request `fonts` are keys into the bundle); `invoice.typ` is sent as the
  template source, not as an asset; `GET /api/pdf/design` also lists files over 10 MB (they only fail a render). An
  unreachable storage → 503 `storage_unavailable`. `design push` / `storage migrate` walk local directories
  following symlinks, each real directory entered once, so a link back into the tree (`shared -> .`) cannot loop.
  Embedded default files are passed to the bundle without copying in release builds; stored overrides come from the
  cache unless their listed version changed. The embedded files' digests are computed once per process and
  sent as digest-only bundle entries (bytes only if mdcast's `409` asks); stored overrides are still hashed per
  render by `AssetBundle`. A non-UTF-8 `invoice.typ` → `pdf_render_failed`.
- **mdcast token**: without `INVOICE__MDCAST_TOKEN` a placeholder Bearer is sent (`mdcast-client` requires one; a
  server without a gate ignores it). `https://mdcast.nexial.cz` requires a real token for renders — without it every
  render is 503 `pdf_unavailable` (401 maps there).
- **Preview**: VAT mode `standard` and CZK regardless of the company's payer flag; due date = today + 14 days;
  supplier placeholder "Vaše firma s.r.o." / "Your Company Ltd." when the company name is empty; the bank is the
  default CZK account; VS = the sample number.
- **Render and template limits** (issue #40, before public registration): mdcast renders are capped instance-wide,
  previews per space with a shorter timeout ([Render limits](#render-limits)); the "busy" case reuses
  `pdf_unavailable` (with `detail: "busy"`) instead of a new code, so clients need no new handling. The MiniJinja
  e-mail templates run with fuel, a recursion limit, an output cap and a timeout ([email.md](email.md)). Orphaned
  document objects are swept ([Storage](#storage)).
