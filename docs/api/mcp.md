# MCP endpoint (phase 3a)

A Model Context Protocol server so an AI client (Claude Desktop / Code, …) can read documents and prepare and issue
invoices. It is a thin layer over the same services as the REST API — no business rule lives only here.

## Transport and auth

- **Streamable HTTP, stateless** at `POST /api/mcp` (crate `rmcp`, server side, `stateful_mode = false`): no
  `Mcp-Session-Id`, no server → client notifications, every request stands alone (works behind a proxy / with
  replicas). `GET` / `DELETE /api/mcp` → 405.
- On a space host, behind the auth middleware like every space route (`src/auth/ctx.rs`): a personal API token
  of that space (or a session cookie); without it → 401 before MCP handling. Read tools need the role `accountant`,
  write tools `member`; a call below that is the tool error `{"code":"forbidden"}` (4a, [spaces.md](spaces.md)).
- `initialize` answers `serverInfo { name: "invoiceapp", version: <crate version> }`, capabilities `tools` only, and
  `instructions`: a short English description of the domain (directions, document types, statuses, money as decimal
  strings, how to issue an invoice: find / create the contact → create a draft → check totals → issue).
- Protocol version: whatever `rmcp` negotiates (≥ 2025-03-26).

## Tool conventions

- Tool names snake_case, English descriptions. Input schemas generated from Rust types (`schemars`), camelCase field
  names, same validation as REST.
- Results: one `text` content item holding the JSON (the REST wire shape, pretty-printed) **and** the same value as
  `structuredContent`. Money / rates are decimal strings as in REST.
- Errors: a tool error result (`isError: true`) whose text is the REST error JSON (`{"code":"…","fields":{…}}`,
  `detail` when present); protocol errors (unknown tool, schema mismatch) are JSON-RPC errors as `rmcp` produces them.
  Internal errors are logged and returned as `{"code":"internal"}`.
- Annotations: read tools `readOnlyHint: true`; write tools `readOnlyHint: false`; `issue_document`
  `destructiveHint: true` (assigns the number, renders and archives the PDF — irreversible); the others
  `destructiveHint: false`; `idempotentHint` true only for `mark_sent` and `update_draft`.
- **PDF is not returned through MCP.** Document results carry `pdfUrl: "/api/documents/{id}/pdf"` (relative to the
  server origin); the client downloads it with the same Bearer token.

## Tools

### Read (`readOnlyHint`)
| Tool | Input | Result |
|---|---|---|
| `list_documents` | `direction` (required), `docType?`, `status?`, `paymentState?`, `overdue?`, `contactId?`, `q?`, `from?`, `to?`, `limit?` (default 20, ≤ 100), `offset?` | `{ items: DocumentSummary[] (+ pdfUrl), total }` — as `GET /api/documents` |
| `get_document` | `id` | Document (+ `pdfUrl`, + `payments`) — issued or received |
| `list_contacts` | `q?`, `limit?` (≤ 100), `offset?` | `{ items: Contact[], total }` |
| `get_contact` | `id` | Contact |
| `lookup_ares` | `ico` | the ARES result as `GET /api/ares/{ico}` (404 / 502 as tool errors) |
| `list_catalog_items` | `q?` | active CatalogItem[] |
| `get_settings` | — | `{ company, bankAccounts, vatRates (active), numberSeries }` — what a draft needs (read only) |
| `compute_document` | as `POST /api/documents/compute` | lines with bases + totals (no write) |

### Write
| Tool | Input | Result |
|---|---|---|
| `create_contact` | the REST `ContactInput` | Contact (ARES is not called implicitly; use `lookup_ares` first) |
| `create_draft` | the REST `DocumentInput` for `docType` `invoice` \| `proforma` \| `simplified`, `direction` `issued` (others → `invalid_value` on `docType`); defaults as the UI: `issueDate` today, `dueDate` from the contact / company due days, `currency` / `locale` from the contact / company, `vatMode` from the company, default bank account of the currency, `paymentMethod`, `roundingMode` (by payment method) and the header / footer notes (by locale) from the company defaults (4e, [defaults.md](defaults.md)) | Document (draft) |
| `update_draft` | `id` + the full `DocumentInput` (replaces, as `PUT`) | Document; non-draft → `document_locked` |
| `issue_document` | `id` | Document (issued, with `number`, `pdfUrl`); every REST issue error as a tool error (`validation`, `pdf_unavailable`, `cnb_unavailable`, …) |
| `add_payment` | `id`, `date`, `amount`, `note?` | the payment + the document's new `paymentState` (DDPP creation for a proforma as in REST) |
| `mark_sent` | `id`, `sentAt?` | Document |

Not exposed (by decision): cancel, delete, credit / debit notes, settlement, e-mail sending, received documents
create / edit, imports / exports, settings changes.

## Implementation notes

- `src/mcp/`: the server (`rmcp` `ServerHandler` + `#[tool]` router), one file per tool group, sharing the REST
  handlers' service functions (extract them where a handler still holds logic). Errors map through `AppError`.
- Mounted in `app.rs` under the auth layer; the request body limit as for JSON routes.
- OpenAPI lists `POST /api/mcp` as an opaque JSON-RPC endpoint.

## Tests

- Integration tests drive `/api/mcp` over HTTP with raw JSON-RPC (`initialize`, `tools/list`, `tools/call`):
  auth required; `tools/list` names + annotations; each tool's happy path; a full flow (ARES-less contact → draft →
  compute → issue with the mdcast mock → payment → mark sent → get with `pdfUrl` → download the PDF over REST);
  validation errors surface as `isError` with the REST JSON; unknown tool → JSON-RPC error.

## Clarifications (as implemented)

- **SDK / transport.** `rmcp` 3.5.1 (`server`, `macros`, `transport-streamable-http-server`), `schemars` 1.
  `stateful_mode` is called `legacy_session_mode` in rmcp 3 and is `false`; responses are `application/json` (no SSE,
  no keep-alive pings), notifications (e.g. `notifications/initialized`) → 202 with an empty body. The client must
  send `Accept: application/json, text/event-stream` and a `Host` header. rmcp's loopback-only `Host` allowlist
  (DNS-rebinding guard for local servers) is disabled: the instance runs on its own public hosts and the auth
  middleware runs first (401 `{"code":"unauthorized"}` + `WWW-Authenticate: Bearer` before any MCP handling).
  No `Mcp-Session-Id` is ever issued; every request may go to any replica.
- **HTTP-level errors** keep the API shape: rmcp's plain-text rejections are rewritten by a response layer on the
  route (`mcp::json_errors`) — 405 (`GET` / `DELETE`, `Allow: POST` kept) → `{"code":"method_not_allowed"}`, 413
  (body > 2 MiB, axum's JSON default) → `{"code":"too_large"}`, 400 / 406 (missing `Accept`) / 415 →
  `{"code":"bad_request"}`. JSON bodies (JSON-RPC errors) pass through unchanged.
- **Protocol version.** `initialize` echoes the client's version when rmcp knows it (`2024-11-05`, `2025-03-26`,
  `2025-06-18`, `2025-11-25`); an unknown one gets `2025-11-25`. Clients on `2026-07-28` (no `initialize`,
  per-request `_meta`) are served statelessly by rmcp as well. A missing `MCP-Protocol-Version` header means
  `2025-03-26`.
- **Arguments.** Tools take the raw argument object and parse it themselves (`mcp::args`, `serde_path_to_error`);
  the advertised `inputSchema` is still generated from the Rust type. A shape error is a tool error with the REST
  validation body: the first failing field, path in the REST form (`lines.0.unitPrice`), reason `required` (missing)
  or `invalid` (wrong type / format, e.g. a bad UUID or date) — not rmcp's plain-text `failed to deserialize
  parameters`. An argument object that is not an object at all → `fields.arguments`.
- **Decimal inputs take numbers.** Every decimal field (`amount`, `exchangeRate`, line `quantity` / `unitPrice` /
  `discountPct` / `vatRate`) accepts a decimal string or a JSON number; the schema says `"type": ["string",
  "number"]`. A number is taken by its shortest textual form (`0.1` → `"0.1"`, `1234.56` → `"1234.56"`, never via
  f64 arithmetic) and then validated like the string (`1e21`, too many decimal places → `invalid`). The same
  serde helper (`num_text`) sits on the shared REST input types, so the REST bodies (`DocumentInput` lines /
  `exchangeRate`, `POST /api/documents/compute`, `POST …/payments`) accept numbers too; responses stay strings.
- **Results.** `content[0].text` is the pretty-printed JSON, `structuredContent` the same value. `structuredContent`
  must be an object, so `list_catalog_items` returns `{ "items": CatalogItem[] }` (not a bare array). `pdfUrl` is
  `/api/documents/{id}/pdf` unless the document serves its uploaded original
  (`pdf::archive::serves_original`: received or imported, the same rule as `GET …/pdf`) and has none — then
  `null`. `get_document` adds `payments` (the `GET …/payments` shape; both read concurrently); `create_draft` /
  `update_draft` / `issue_document` / `mark_sent` return the document with `pdfUrl` (no `payments`).
  `add_payment` → `{ payment: Payment, paymentState }` (`payment.advanceDocumentId` = the DDPP it issued;
  `paymentState` derived from the document row, no full view load). `sentAt` comes back in UTC.
- **Committed writes never report failure.** After `create_draft`, `update_draft`, `issue_document`, `add_payment`
  or `mark_sent` has committed, a failing follow-up read is logged and the tool still succeeds with what is known:
  `{ "id", "readError": {"code":…} }` (document tools) or `{ payment, "paymentState": null, "readError" }`, so a
  client does not retry a write that happened. `create_contact` has no follow-up read.
- **Errors.** A domain error is a tool result with `isError: true`; its text and `structuredContent` are the REST
  error body (`code`, `fields`, `detail` exactly as the REST route would return, via the shared `AppError::body()`),
  logged like a REST failure; internal / DB causes surface only as `{"code":"internal"}`. An unknown tool is a
  JSON-RPC error (`-32602`, "tool not found").
- **`create_draft`.** Same validation and defaults as `POST /api/documents` (shared `documents::create_draft`).
  **Deviation:** a disallowed `docType` / `direction` gives the REST reason `invalid` (`{"fields":{"docType":
  "invalid"}}`), not `invalid_value` (that code exists only in the CSV import). `imported: true` → `{"fields":
  {"imported":"invalid"}}` (manual import is out of the MCP scope). `direction: "received"` is not dispatched to the
  received create as REST does — it is `docType: invalid`.
- **Draft scope of `update_draft` / `issue_document`.** Only drafts MCP can create: issued direction, `invoice` /
  `proforma` / `simplified`, not imported, no `relatedDocumentId`. Any other draft (credit / debit note, DDPP
  correction, a settlement invoice from `…/settle`, an imported draft) → `{"code":"invalid_state"}`; they are
  edited and issued in the UI. Non-drafts keep the REST errors (`document_locked` for update — received documents
  included —, `invalid_state` for issue).
- **`update_draft`.** Input = `id` + the flattened `DocumentInput` (same level, not nested); `id` and the document
  are parsed apart so field paths inside the document stay exact.
- **`add_payment`** takes an optional `exchangeRate` as `POST …/payments` does: CZK per unit for the DDPP of a
  foreign-currency VAT-payer proforma (default ČNB for the payment date; when ČNB cannot give one →
  `{"fields":{"exchangeRate":"required"}}`, fixable by resending with the rate). Ignored otherwise. The DDPP PDF is
  archived in the background exactly as after the REST call.
- **Paging.** `list_documents` and `list_contacts`: `limit` default 20, at most 100, then the REST normalization
  (search trimmed, `limit` ≥ 1, `offset` bounded) — clamped, not an error. `list_documents.direction` must be
  `issued` / `received` (else `direction: invalid`); REST filters it as given. `status` / `paymentState` outside
  their enums → `invalid`.
- **`get_settings`** reads company, bank accounts, VAT rates and number series concurrently through the settings
  repos and REST DTOs; `vatRates` keeps active rates only.
- **Annotations.** `idempotentHint` is `true` only for `update_draft`: **deviation** — `mark_sent` is `false`,
  since without `sentAt` each call sets a new timestamp. Every tool sets `openWorldHint` explicitly: `true` only
  for `lookup_ares` (public register). Read tools set only `readOnlyHint: true` (+ `openWorldHint`).
- **Shared code.** Extracted from the REST handlers so both use one implementation: `documents::{list_page,
  create_draft, update_draft, compute_totals}`, `payments::add` (incl. the DDPP archive spawn),
  `actions::mark_sent_at` (default now), `ares::handlers::lookup_ico` (IČO normalization + `invalid_ico`),
  `AppError::{log, body}`, `pdf::archive::serves_original`, `NumberSeries::build`.
