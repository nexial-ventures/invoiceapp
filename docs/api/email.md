# E-mail (phase 2a)

Issued documents are sent by e-mail over SMTP (crate `lettre`, tokio + rustls). The subject and body come from
MiniJinja plain-text templates (one set for all document types, per locale), editable in Settings → E-mail.

## Configuration (env only)

| Variable | Meaning |
|---|---|
| `INVOICE__SMTP__HOST` | SMTP server; unset → e-mail not configured (sending → 503 `smtp_not_configured`) |
| `INVOICE__SMTP__PORT` | default by TLS mode: `starttls` 587, `tls` 465, `none` 25 |
| `INVOICE__SMTP__TLS` | `starttls` (default) \| `tls` (implicit TLS) \| `none` (plain, tests / local relay only) |
| `INVOICE__SMTP__USERNAME`, `INVOICE__SMTP__PASSWORD` | both or neither (one alone → `serve` refuses to start); with `TLS=none` only to a loopback HOST (`localhost`, `127.0.0.0/8`, `::1`), else `serve` refuses to start |
| `INVOICE__SMTP__FROM` | sender, `addr@x` or `Name <addr@x>`; required when HOST is set |
| `INVOICE__EMAIL_DISABLED` | `true` = the operator's kill switch (default `false`): every e-mail sent on behalf of a space (document e-mails, the test e-mail) → 503 `email_disabled` (also without SMTP — it wins over `smtp_not_configured`); account e-mails (verify, "already registered", reset, invitations) are skipped and logged at `warn` — an invitation is still created, with `emailSent: false` and no `url` ([members.md](members.md)) |

- Invalid config (bad FROM, unknown TLS mode, HOST without FROM) → `serve` refuses to start with a clear message.
- Every SMTP exchange (connect + send) has a 30 s total timeout.
- Headers: `From` = FROM, `Reply-To` = company e-mail (Settings → Company) when set, `To`/`Cc` as given, `Bcc` never
  written to the message (envelope only), `Date`, `Message-ID: <{uuid}@{domain of FROM}>`, `MIME-Version`;
  body `text/plain; charset=utf-8`; attachments base64.

## Templates

- One template set for all seven document types: `subject` + `body` per locale (`cs`, `en`). Defaults are embedded
  in the binary (`src/email/defaults/{cs,en}/{subject,body}.txt`); an override lives in Storage as one object per
  locale, `email/templates/{locale}.json` = `{"subject": "…", "body": "…"}` (one atomic write on save, so a save
  never leaves half an override). Storage unreachable →
  503 `storage_unavailable`.
- MiniJinja, `UndefinedBehavior::Strict`, no HTML auto-escape, `none` renders as an empty string. The rendered
  subject is trimmed and newlines in it are replaced by spaces; the body keeps its text, trailing whitespace trimmed.
- **Isolation and limits.** A template is member input, and MiniJinja's fuel bounds work, not memory (a string
  repeated and concatenated with itself reaches gigabytes in a few steps). So the server never renders one in its
  own process: every render — save validation (all sample variants in one child), preview and the send prefill —
  runs its own binary as the hidden `invoice render-template` (`src/email/sandbox.rs`; the job as JSON on stdin,
  progress lines and the result on stdout, empty environment). On Linux the child gets `RLIMIT_DATA` 256 MiB and
  `RLIMIT_CPU` 5 s (`setrlimit` between fork and exec; elsewhere only the wall clock applies); after 5 s wall
  clock it is killed. Inside it the engine (`src/email/template_render.rs`) still applies fuel 100 000
  instructions per part (the defaults use a few hundred), recursion limit 100 and at most 1 MiB output per part.
  Every limit is 422 `template_invalid` for the part being rendered, `detail` (led by the sample variant's prefix
  where one applies): `line N: too much work (more than 100000 steps)`, a recursion-limit message, `the output is
  larger than 1024 KB`, `the template needs too much memory or time` (the child died: memory or CPU limit), or
  `rendering took longer than 5 s` (killed).
- At most 2 children run at once per instance; a render that waits over 2 s for one → 503 `{"code":"busy"}`.
  Template save + preview share a per-space budget of 60 / hour (bucket `template-space`, in memory) → 429
  `rate_limited` + `Retry-After`; the send prefill has its own 120 / hour per space (bucket `prefill-space`) → 429
  `rate_limited` + `Retry-After`. A `busy` refusal gives the attempt back in both.
- Context (all display values are pre-formatted strings in the template's locale, same formatting as the PDF payload;
  missing values are `none`):

```
doc: {
  type: "invoice" | "proforma" | "credit_note" | "advance_tax_doc" | "debit_note" | "advance_credit_note" | "simplified",
  typeLabel,            // the PDF title, e.g. "Faktura – daňový doklad"
  number, issueDate, taxDate, dueDate,
  total,                // "1 234,50 Kč"
  payable,              // amount left to pay after advances / payments; "0,00 Kč" when settled
  currency,             // "CZK"
  paid: bool, cancelled: bool,
  variableSymbol, bankAccount, iban,
  originalNumber        // corrections: the corrected document's number, else none
}
company: { name, email, phone, web }
contact: { name, email } | none   // none for a contactless simplified document
```

- Defaults (cs; en analogous):
  - subject: `{{ doc.typeLabel }} {{ doc.number }} – {{ company.name }}`
  - body: greeting, "v příloze Vám zasíláme doklad č. {number} ({typeLabel}) na částku {total}", due date + VS
    when payable > 0 and not paid, "Neplaťte – již uhrazeno." when paid, signature `company.name`.
- **Validation** (save and preview): compile both templates, then render them in strict mode on the sample context
  (the PDF preview's sample invoice, with a contact). The first error →
  422 `{"code":"template_invalid","fields":{"subject"|"body":"template_invalid"},"detail":"line 3: undefined value"}`
  (`detail` = line + MiniJinja message; subject checked first). Size limits: subject 500, body 20 000 chars
  (`too_long`), subject empty → `required`.

## Routes

### Settings
- `GET /api/settings/email` → `{ configured: bool, disabled: bool, from: string | null, replyTo: string | null }`
  (`disabled` = `INVOICE__EMAIL_DISABLED`; the tab then shows the notice and blocks the test e-mail)
  (`replyTo` = company e-mail).
- `POST /api/settings/email/test` `{ to?: string }` (default: company e-mail; neither → 422 `to: required`) → 204;
  sends a short fixed test message (no template), not in the document log but counted by the e-mail limits.
  503 `email_disabled` / `smtp_not_configured`, 429 `rate_limited`, 502 `smtp_failed` + `detail`.
- `GET /api/settings/email/templates` → `{ templates: [ { locale, subject, body, custom: bool } ] }` (cs, en; the
  effective text, `custom` = an override exists).
- `PUT /api/settings/email/templates/{locale}` `{ subject, body }` → 200 the template entry; invalid → 422 as above,
  the active template stays. Unknown locale → 404 (checked first). Both parts are required: a missing `subject` /
  `body` is 422 `required` (not a 400), also for the preview.
- `DELETE /api/settings/email/templates/{locale}` → 200 the default entry (`custom: false`); removes the override
  (idempotent).
- `POST /api/settings/email/templates/{locale}/preview` `{ subject, body }` → `{ subject, body }` rendered on the
  sample; 422 as above. Nothing stored.

### Sending
- `GET /api/documents/{id}/email?locale=cs|en` → the dialog prefill:
  ```
  { configured: bool, disabled: bool, locale, to: [string], cc: [], bcc: [string], subject, body,
    attachments: { pdf: { available: bool, filename }, isdoc: { available: true, filename } } }
  ```
  `locale` defaults to the document's locale; `to` = the contact's e-mail when set (else `[]`); `bcc` = the company
  e-mail when set; `pdf.available` false only for an imported document without an original. Only for an issued
  (non-draft, cancelled included) document of direction `issued`; a draft → 409 `invalid_state`, a received
  document → 404. Template error on real data (cannot normally happen after validation) → 422 `template_invalid`.
  120 prefills / hour per space → 429 `rate_limited` + `Retry-After`; no free renderer → 503 `busy` (not counted).
- `POST /api/documents/{id}/email` `{ to: [string], cc: [string], bcc: [string], subject, body, attachPdf: bool,
  attachIsdoc: bool }`:
  - 422: `to` empty → `to: required`; an unparsable address → `to.0: invalid` (same for cc/bcc); more than **10**
    recipients in total (to + cc + bcc) → `to: too_long`; subject empty → `required`, > 500 → `too_long`, newline in subject →
    `invalid`; body > 100 000 → `too_long`; `attachPdf` while `pdf.available` is false → `attachPdf: invalid`.
  - Same state rules as the GET. E-mail disabled → 503 `email_disabled`; SMTP not configured → 503
    `smtp_not_configured`.
  - **Limits** ([spaces.md](spaces.md#limits)): the space's and the sending user's e-mails per hour / day (defaults
    20 / 100 each); used up → 429 `{"code":"rate_limited"}` + `Retry-After`, nothing sent or logged.
  - Attachments are prepared before sending: `{number}.pdf` (the archive; lazy render like `GET …/pdf`, the imported
    original for imported documents), `{number}.isdoc` (`application/xml`, the plain XML of `GET …/isdoc`, no
    `.isdocx`). Preparation errors (`pdf_unavailable`, `pdf_render_failed`, `storage_unavailable`) abort before
    sending and are **not** logged.
  - Then the message is sent synchronously and the attempt is logged either way:
    - success → 200 the log entry (`ok: true`) and the document's `sentAt` = now (overwrites, like mark-sent);
    - SMTP failure / timeout → 502 `{"code":"smtp_failed","detail":"<SMTP error>"}`, entry logged with `ok: false`,
      `sentAt` unchanged.
  - Re-sending is allowed any number of times.
- `GET /api/documents/{id}/emails` → `[EmailLogEntry]`, newest first (404 for an unknown or received document; a
  draft has none).

```
EmailLogEntry { id, createdAt: datetime, to: [string], cc: [string], bcc: [string], subject, body,
                attachments: [string] /* filenames */, ok: bool, error: string | null, messageId: string | null }
```

## Storage / DB

- Table `document_emails` (append-only log): `id uuid pk`, `document_id uuid fk → documents on delete cascade`,
  `created_at timestamptz`, `to_addrs`, `cc_addrs`, `bcc_addrs text[]`, `subject text`, `body text`,
  `attachments text[]`, `ok bool`, `error text null`, `message_id text null`; index `(document_id, created_at desc)`.
- `smtp_failed.detail` is the SMTP server's response / transport error text (no credentials); everything else follows
  the usual error rules.

## UI

- Document detail (issued direction, non-draft): "Send by e-mail" button → dialog: locale switch (re-fetches the
  prefill; edits are replaced after a confirm if the subject/body were changed), To / Cc / Bcc as chip inputs,
  "Bcc to me" checkbox (adds/removes the company e-mail), subject, body (textarea), attachment checkboxes (PDF
  disabled when unavailable), Send. Disabled with a hint when not configured. Errors shown inline (`smtp_failed`
  with its detail; `email_disabled`). While `disabled`, the button is disabled with a hint and the dialog shows the
  reason instead of sending. A 429 here (and on the test e-mail and invitations) reads as the hourly / daily e-mail
  limit, with the wait from `Retry-After` (minutes below an hour, else hours). Success → toast, detail reloads (`sentAt`, history).
- Detail: "E-mail history" section (date, recipients, subject, result; expanding shows body + attachments + error).
- Settings → E-mail tab: configuration status (from, reply-to, configured yes/no), test e-mail form, template editor
  per locale (subject input + body textarea, variable reference list, Preview → rendered subject/body, Save, Restore
  default with confirm), `template_invalid` shown with its line message.

## Tests

- An in-process mock SMTP server (`tests/common/smtp.rs`, plain SMTP, `TLS=none`) captures messages and can be told
  to reject (5xx) — no external SMTP in CI.

## Clarifications (as implemented)
- Context additions / precision: `doc.toPay: bool` (not paid, not cancelled, and owed to us — false for credit notes
  and DDPP corrections); the default body uses it for the due-date line. `doc.payable` = `max(payable − payments, 0)`
  with the document's sign (credit notes negative, like the PDF); `doc.paid` = that remainder is zero, or always for a
  DDPP. `doc.dueDate` is always set for an issued document. `company` is the current Settings → Company (not the
  issue-time snapshot); `contact.name` is the customer snapshot's name (as printed), `contact.email` the live
  contact's e-mail when set, else the snapshot's; `contact` is `none` when the document has no customer snapshot.
  In strict mode `contact.*` on a `none` contact is an error — guard it with `{% if contact %}`.
- Validation (save and preview) renders on a matrix of variants of the PDF preview's sample invoice, in this order,
  so strict errors inside branches the base sample does not take are caught at save time. A failure on a variant is
  422 `template_invalid` with `detail` led by the variant's prefix (exact strings, for localization):

  | Prefix | Variant |
  |---|---|
  | *(none)* | base: unpaid invoice with bank account, VS, due date and a contact — the preview shows this rendering |
  | `paid: ` | fully paid (`doc.paid` true, `doc.toPay` false, `doc.payable` zero) |
  | `cancelled: ` | cancelled (`doc.cancelled` true, `doc.toPay` false) |
  | `credit note: ` | credit note (`doc.type` `credit_note`, negative amounts, `doc.originalNumber` set) |
  | `no bank account: ` | `doc.bankAccount`, `doc.iban`, `doc.variableSymbol`, `doc.taxDate`, `doc.dueDate` all `none` |
  | `without contact: ` | contactless simplified document (`contact = none`, `doc.type` `simplified`) |

  E.g. `without contact: line 2: undefined value`, `paid: line 1: undefined value`.
- Subject normalization: each line trimmed, empty lines dropped, the rest joined with single spaces.
- `template_invalid.detail` = `line N: <MiniJinja error kind>[: <detail>]` (e.g. `line 3: undefined value`). Template
  `PUT` / `preview` need both `subject` and `body` as strings (missing → 400 `bad_request`); an empty body is allowed.
  An unknown locale → 404 before any validation.
- Order of checks on `POST …/email`: 404 / 409 (state) → 503 `email_disabled` → 503 `smtp_not_configured` → 422 →
  attachment preparation → 429 e-mail limits → SMTP. The test route checks `email_disabled` / `smtp_not_configured`
  before its 422, then the e-mail limits (it counts as one e-mail, kind `test`); its body is optional (empty →
  company e-mail).
  The test message is fixed text in the company's default locale (cs "Testovací e-mail", en "Test e-mail"), with
  `Reply-To` like a document e-mail.
- Sending a **cancelled** document sets `sentAt` too (`lifecycle::email_sent`); `POST …/mark-sent` still refuses
  cancelled documents.
- After the SMTP server accepted a message, logging it and setting `sentAt` are best effort: a failure there is
  logged server-side and the route still answers 200 with the entry (so the user does not send it again); that entry
  is then missing from the history. A failed attempt that cannot be logged still answers 502 `smtp_failed`.
- Log: `to` / `cc` / `bcc` are stored trimmed as entered (display names kept, e.g. `Odběratel <a@x.cz>`);
  `attachments` lists the filenames actually attached; `messageId` is set only on `ok: true` entries (`null` when the
  SMTP exchange failed). History of a draft → `[]`; unknown / received → 404.
- Attachment filenames use the document number made header-safe like the PDF download (`[A-Za-z0-9._-]`, anything
  else → `_`): `FV-2019/0042` → `FV-2019_0042.pdf`. `pdf.available` is true for a native document whose archive is not
  rendered yet (rendered lazily during the send).
- A company e-mail that is not a valid address is left out of `Reply-To` (warning logged) instead of failing the send.
- Timeout: `lettre`'s per-command timeout is 30 s and a 30 s total timeout wraps connect + send; hitting it →
  502 `smtp_failed` with `detail` `timed out after 30 s`. `smtp_failed.detail` is lettre's error text, e.g.
  `permanent error (550): 5.7.1 relay denied`.
- `GET /api/settings/email` `from` is the configured mailbox as formatted by lettre (`Name <addr@x>` or `addr@x`).
- TLS uses rustls with the bundled webpki root certificates (no system store needed in the image).
