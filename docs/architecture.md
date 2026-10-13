# Architecture

One crate, one binary (`invoiceapp`), one Postgres database, one embedded SPA. This file is the map: what each module
is, its key types, and where its contract lives. Behaviour, routes, roles, limits and error codes are specified in
[`api/`](api/README.md) (one file per area; each ends with "Clarifications (as implemented)").

```
invoiceapp serve            → migrations up → Axum server (API + SPA) on INVOICEAPP__BIND
invoiceapp migrate up|status|down [-n N]
invoiceapp storage migrate --from-dir <dir> [--design-dir <dir>]   → copy an fs archive (+ old design dir) into the configured storage
invoiceapp storage sweep [--dry-run] [--force]                       → delete unreferenced document objects older than 1 h (needs the database; guarded)
invoiceapp design push <dir> | pull <dir> | ls | rm <path>… --space <slug>  → design overrides of a space (`spaces/{id}/design/…`)
invoiceapp openapi          → print the OpenAPI document (no config needed; `make frontend-api-types`)
```

`storage` needs only the storage settings (with `INVOICEAPP__DATABASE_URL` it also re-records every space's design
storage usage afterwards); `design` also needs `INVOICEAPP__DATABASE_URL` (slug → space id). There is no CLI user /
space creation: bootstrap = registration.

## Backend (`src/`)

Domain modules follow a simplified nexial/infra pattern: `src/<module>/{mod.rs (router), entity/, repo/, handlers/}`
(handler files hold their DTOs + validation; repos take validated DTOs). Routers are mounted in `app.rs` and
registered in `openapi.rs`; they share `AppState`, `error.rs` and `validation.rs` instead of per-module config /
error files. Every repo takes the `SpaceId` of the request's role extractor ([api/spaces.md](api/spaces.md)).

### Core

| File | What it is | Contract |
|---|---|---|
| `main.rs` | clap CLI (`storage …` / `design …` in `main_storage.rs`; the hidden `render-template` child of `email::sandbox`, dispatched before any runtime or logger), tracing init, storage reachability probe at start (logged only), the hourly purge (`cleanup::run`), the daily orphan sweep (`storage::sweep`, first run 1 h after start), serves with `ConnectInfo` (client IP), graceful shutdown on SIGINT / SIGTERM | this file |
| `lib.rs` | module root; lets integration tests build the router | — |
| `app.rs` | `AppState` (db, ARES, ČNB, `PdfRoot`, `Option<Mailer>`, `PublicUrl`, switches, `SecretKey`, rate limiter, `TemplateSandbox`, `Option<Google>`); `router(state)`: health / OpenAPI on any host, the rest classified by host, then public auth, account, base-host, invitation-accept and space-host route groups | [api/spaces.md](api/spaces.md), [api/auth.md](api/auth.md) |
| `config.rs` | `config` crate, prefix `INVOICEAPP`, separator `__`; `DbConfig` (migrate, design) and the full serve config; validation refuses the start (see [Configuration](#configuration)); tests in `config_tests.rs`, removed/renamed settings in `config_legacy_tests.rs` | [Configuration](#configuration) |
| `db.rs` | `advisory_xact_lock(key)`, `allow_idle` (lifts the idle-in-transaction timeout for one transaction), `pool_options` (size, acquire timeout, server-side timeouts) | [api/README.md](api/README.md#clarifications-as-implemented) |
| `cleanup.rs` | hourly purge of expired rows (sessions, tokens, pending logins, Google flows, invitations, old `email_sends`) | [api/README.md](api/README.md#clarifications-as-implemented) |
| `error.rs` | `AppError` → `{"code"}` (+ `fields` on 422, + `detail` where a contract allows); `log()` / `body()` shared by REST and MCP; DB conversions in `error_convert.rs`, `FieldErrors` in `error_fields.rs`; internal causes logged only | [api/README.md](api/README.md#errors) and each part |
| `extract.rs` | `ApiJson` / `ApiQuery` / `ApiPath`: rejections in the error format (400 `bad_request`, bad path → 404); `optional_json` | — |
| `download.rs` | `attachment(content_type, filename, body)` (`Content-Disposition: attachment`, `no-store`) | [api/README.md](api/README.md#exports) |
| `validation.rs`, `num_parse.rs`, `num_text.rs` | pure validators (IČO, DIČ, IBAN, account number, codes, lengths); decimal text (`decimal`, `parse_rate`); decimals accepted as JSON number or string | [api/README.md](api/README.md) |
| `paging.rs` | `ListQuery` (`q` / `limit` / `offset` → `normalized()`), `like_pattern` (escaped ILIKE) | [api/README.md](api/README.md#list-shape) |
| `time.rs` | `today()` / `current_year()` in Europe/Prague | [api/documents.md](api/documents.md) |
| `currency.rs` | `LOCAL_CURRENCY` (`CZK`, the accounting currency) + `is_local` | — |
| `secret.rs` | `Secret<T>` (redacted `Debug`), `SecretKey` (`INVOICEAPP__SECRET_KEY`) | [api/mfa.md](api/mfa.md) |
| `health.rs`, `openapi.rs`, `spa.rs` | `GET /api/health` (DB ping); utoipa document (`cookie` / `bearer` schemes); rust-embed of `frontend/dist` (client routes → `index.html`, `/assets/*` immutable, 503 if not built) | — |
| `security_headers.rs`, `compression.rs` | outermost middleware: CSP, HSTS (https only) and friends on every response; brotli / gzip of text bodies, `Identity(..)` for fresh secrets | [api/README.md](api/README.md#clarifications-as-implemented) |
| `layering_tests.rs` | source-text checks: `document::handlers` used only by `document/mod.rs` and `openapi.rs`; `document/wire` names no repo / entity / DB types | this file |
| `migration/` | sea-orm-migration `Migrator`, raw-SQL migrations, append-only (`mod.rs` explains how to add one); each file's header says what it does and when it refuses (data pre-checks, irreversible `down`) | `src/migration/` |

### Storage, auth, spaces

| Module | What it is | Contract |
|---|---|---|
| `storage/` | `Storage` over `object_store` (fs or S3): atomic `put` (sha256), `get_stream`, `delete`, `list`, `delete_prefix`, server-side `copy`, `scoped(prefix)`; `config.rs`, `local.rs` (walk a dir), `transfer.rs` (CLI copy / pull), `sweep.rs` (orphan sweep: `spaces/` only, refuses on a suspicious ratio unless `--force`); errors → 503 `storage_unavailable` | [api/pdf.md](api/pdf.md), [api/design.md](api/design.md) |
| `auth/host.rs` | `PublicUrl`, `classify` → base \| `Slug` \| unknown; middleware `classify`, `space_only`, `base_only`; `GET /api/context` | [api/spaces.md](api/spaces.md) |
| `auth/ctx.rs`, `auth/resolve.rs` | `authenticate` (Bearer on its space's host, or the `invoice_session` cookie on its own host) → `Authed`; CSRF `check_origin`; extractors `Read` / `Write` / `Manage` / `Own` → `Scope`; one joined lookup query, throttled `last_seen_at` | [api/auth.md](api/auth.md), [api/spaces.md](api/spaces.md) |
| `auth/crypto.rs`, `session.rs`, `users.rs` | tokens and digests, argon2id behind a global semaphore (503 `busy`), `inv_{prefix}_{secret}` API tokens; session cookies and lifetimes; users and single-use `user_tokens` | [api/auth.md](api/auth.md) |
| `auth/handlers/`, `auth/tokens/`, `auth/mail.rs` | login / logout / me, register / verify, password change / reset; `/api/tokens`; cs / en account e-mails sent in a spawned task | [api/auth.md](api/auth.md), [api/spaces.md](api/spaces.md) |
| `auth/ratelimit/` | in-memory limiter; `limits.rs` holds every limit (login delay, per-IP, TOTP, mail, invitations, Google); `client_ip.rs` (`ConnectInfo` or last `X-Forwarded-For` hop) | [api/auth.md](api/auth.md) |
| `auth/mfa/` | TOTP: `totp.rs` (pure RFC 6238), `seal.rs` (AES-GCM secrets, HMAC recovery codes), `repo.rs`, `verify.rs` (`check` inside the caller's transaction, step-up), `handlers.rs`, `login.rs` (`POST /api/auth/login/mfa`) | [api/mfa.md](api/mfa.md) |
| `auth/google/` | Sign in with Google: `config.rs`, `provider.rs` (`Google`: discovery, code exchange + PKCE), `jwks.rs`, `idtoken.rs` (pure check), `flow.rs` (flows / handoffs), `account.rs` (resolve / link), `handlers.rs`, `finish.rs` | [api/google.md](api/google.md) |
| `space/` | `SpaceId` (only from `Scope` / a row; `storage_prefix()` = `spaces/{id}`), `Role` (accountant < member < admin < owner), `slug.rs`, `seed.rs` (per-space seeds), `limits.rs` (quota, e-mail limits), `repo.rs`, `handlers.rs` (`/api/spaces`, `/api/space`) | [api/spaces.md](api/spaces.md) |
| `members/` | `rules.rs` (pure role rules), `repo.rs` (members, under the space row lock), `invites.rs`, `join.rs` (accept core shared by the accept route, Google finish and the login code step), `accept.rs`, handlers | [api/members.md](api/members.md) |

### Business

| Module | What it is | Contract |
|---|---|---|
| `settings/` | company profile + document defaults, bank accounts, VAT rates, number series (`pattern.rs` incl. `{C}` and the inverse matcher, `repo/numbering.rs` = allocation / preview / manual-number matching, global or per-client), categories, custom field definitions | [api/settings-contacts.md](api/settings-contacts.md), [api/numbering.md](api/numbering.md), [api/defaults.md](api/defaults.md), [api/received-import.md](api/received-import.md) |
| `contact/`, `ares/`, `cnb/` | address book (+ `numberCode`, per-customer counters `repo/number_counters.rs`); `AresClient` → `AresSubject`; `CnbClient` + `denni_kurz.txt` parser behind the `exchange_rates` cache | [api/settings-contacts.md](api/settings-contacts.md), [api/documents.md](api/documents.md) |
| `catalog/` | catalog items and groups (ordered members, one VAT rate) | [api/advances-credit-catalog.md](api/advances-credit-catalog.md) |
| `document/` (pure) | `line.rs` (enums as string `ActiveEnum`s), `doc_type.rs` (`DocType` = series key), `compute.rs`, `rounding.rs`, `advance.rs`, `ddpp.rs`, `credit.rs`, `correction.rs`, `defaults.rs`, `received.rs`, `snapshot.rs`, `model.rs`, `meta.rs` | [api/documents.md](api/documents.md), [api/doc-types.md](api/doc-types.md), [api/defaults.md](api/defaults.md) |
| `document/` (I/O) | `service.rs` = operations shared by REST and MCP; `wire/` = DTOs + request validation (no DB); `repo/` = DB (insert, issue, lifecycle incl. unlock `issued → draft` (admin+, number kept), payments, DDPP, settle, credit, corrections, query); `handlers/` = axum only | [api/advances-credit-catalog.md](api/advances-credit-catalog.md), [api/received-import.md](api/received-import.md) |
| `pdf/` | `PdfRoot` (in `AppState`) → `space(id)` = `PdfService` (scoped storage, `render`); `limits.rs` (`RenderLimits`: instance-wide mdcast slots, per-space preview limits); `client.rs` (mdcast errors), `design.rs` + `design_cache.rs` (embedded ∪ stored design, cached), pure `format` / `labels` / `reference` / `payload` / `amounts` / `spayd`; `source.rs`, `archive.rs`, `preview.rs`, `handlers.rs` | [api/pdf.md](api/pdf.md) |
| `pdf/editor/` | per-space design editor: draft, activation / restore / reset, versions, usage under the space quota, `design_drafts` row lock | [api/design.md](api/design.md) |
| `import/` | bulk import pipeline shared by ISDOC and CSV / XLSX: `model.rs` (`Plan`), `lookup.rs` / `batch.rs` (per entry / one query per kind over the batch), `check.rs`, `category.rs`, `store.rs` (one entry per transaction), `form.rs`, `wire.rs` | [api/isdoc.md](api/isdoc.md), [api/csv.md](api/csv.md) |
| `isdoc/` | ISDOC 6.0.2: pure `parse` / `plan` / `upload` (zip limits) / `export_xml`; `analyze.rs`; handlers `import.rs`, `export.rs`, `export_bulk.rs` | [api/isdoc.md](api/isdoc.md) |
| `csvio/` | CSV / XLSX interchange: pure `format` / `read` / `columns` / `cell` / `amounts` / `row` / `write` / `export_row`; `analyze.rs`, `export_csv.rs`, handlers | [api/csv.md](api/csv.md) |
| `export/` | what the CSV and accounting exports share: `load.rs` (`Loaded` chunks → `Source`), `source.rs` (CZK recap / total / amount conversions), `period.rs` (the tax-date period check and filter, also used by `overview`); `handlers.rs` (the only module using both `csvio` and `accounting`) | [api/csv.md](api/csv.md), [api/accounting.md](api/accounting.md) |
| `overview/` | `GET /api/overview`: pure `period.rs` (chart months), `aggregate.rs` (totals, VAT per rate, months, categories), `open.rs` (receivables / payables); `repo.rs` (column-only reads in one snapshot), `dto.rs` (`Overview*` schemas), `handlers.rs` | [api/overview.md](api/overview.md) |
| `accounting/` | accounting settings (`settings.rs`) and the Pohoda (`pohoda*.rs`) and Money S3 (`money*.rs`) XML writers over the shared `doc.rs`; `export.rs` (`Format`), `repo.rs`, `handlers.rs` | [api/accounting.md](api/accounting.md), [api/money.md](api/money.md) |
| `email/` | `config.rs` (`SmtpConfig`), `sender.rs` (`Mailer`, lettre + rustls), `templates.rs` (MiniJinja), `sandbox.rs` + `template_render.rs` (rendering in a resource-limited child process), `context.rs`, `input.rs`, `limits.rs` (outbound limits), `log.rs`, `handlers/` (settings, document send) | [api/email.md](api/email.md), [api/spaces.md](api/spaces.md#limits) |
| `mcp/` | `rmcp` Streamable HTTP, stateless; `read.rs` / `write.rs` tool routers calling the REST service functions; `args.rs`, `params.rs`, `result.rs` | [api/mcp.md](api/mcp.md) |

### Invariants

- DB-enforced as well as in the repos: one company row per space, at most one default bank account per currency and
  one active default VAT rate (partial unique indexes; repos promote a replacement), unique VAT rate value, unique
  contact IČO when not null, unique contact `number_code` (case-insensitive), unique `(doc_type, number)` per direction, composite `(space_id, x_id)` FKs between
  space tables ([api/spaces.md](api/spaces.md#data-scoping)).
- Multi-row writes run in `db.transaction(|txn| …)`. `numbering::allocate` locks the counter row of
  `number_series_counters` (global) or `contact_number_counters` (a customer's client sequence), takes the next free
  number after it (held numbers of the series loaded once) and sets the counter to it; it must run inside the
  transaction that stores the numbered document. An unlocked draft keeps its number unless renumbered; `document/repo/numbering.rs` decides keep / manual /
  allocate and `previousNumber` ([api/numbering.md](api/numbering.md)).
- Totals are recomputed by `compute.rs` on every save and at issue and stored; reads return stored values. Money
  arithmetic is checked; an overflow is a 422.
  Received recaps and DDPP recaps are stored as entered / computed, never recomputed.
- Every state-changing write locks the document row before checking its status. Lock order is always owning document
  → referenced documents (invoice → DDPP / proforma, proforma → DDPP, correction → its original).
- No transaction, row lock or pooled connection is held across an mdcast render, SMTP or a document storage call;
  the design editor's lock transaction is the documented exception (`db::allow_idle`). Exports build the file in a
  read-only snapshot, commit, then send. The issue / archive protocol: [api/pdf.md](api/pdf.md),
  [api/documents.md](api/documents.md).
- Every foreign key has an index (`tests/fk_indexes.rs`; a partial index counts only with the predicate
  `<one of the FK columns> IS NOT NULL`). Every route has an access decision (`tests/spaces_route_roles.rs`).

## Routes

Every route, its role and its wire format is in [`api/`](api/README.md). Summary of the access model:

- `/api/health`, `/api/openapi.json`: any host, public. `/api/context`, login, password reset, Google navigations:
  any known host, public. Register / verify and `/api/spaces`: base host. Everything else: a space host.
- `space` routes need the session cookie or an API token of that space; the minimum role comes from the handler's
  extractor: GET → `accountant`, mutations (and read-only POSTs such as compute and previews) → `member`, Settings
  mutations and `PUT /api/space` → `admin`, space delete → `owner` (403 `forbidden`). MCP tools: read `accountant`,
  write `member`.
- On the base host every space route is 404; an unknown host is 404 for every `/api` route; other `/api/*` → 404
  `not_found`; anything else is the SPA. Unauthenticated → 401 `unauthorized` + `WWW-Authenticate: Bearer`; a
  cookie mutation without the own `Origin` → 403 `csrf`.

## Configuration

| Env var | Required | Default | Notes |
|---|---|---|---|
| `INVOICEAPP__DATABASE_URL` | yes | — | Postgres URL |
| `INVOICEAPP__DB_MAX_CONNECTIONS` | no | `20` | serve pool size (≥ 1); acquire timeout 5 s; every pooled connection runs with `statement_timeout` and `idle_in_transaction_session_timeout` 30 s (migrations use their own connection without them) |
| `INVOICEAPP__DB_PGBOUNCER` | no | `false` | `true` behind a transaction-pooling PgBouncer: no startup `options`, so the pool runs **without** the two 30 s timeouts (PgBouncer refuses untracked options and silently drops tracked ones Postgres does not report) |
| `INVOICEAPP__PUBLIC_URL` | serve | — | `http(s)://host[:port]`: base host (registration, login, "my spaces"); spaces at `{slug}.{host}` |
| `INVOICEAPP__SECRET_KEY` | serve | — | 32 bytes base64 (`openssl rand -base64 32`): AES-256-GCM of TOTP secrets, HMAC of recovery codes ([api/mfa.md](api/mfa.md)); missing / wrong length → start refused; changing it breaks every enrolled TOTP |
| `INVOICEAPP__EMAIL_DISABLED` | no | `false` | kill switch: every e-mail sent on behalf of a space (document, test) → 503 `email_disabled`; account e-mails (verify, reset, invitations) skipped and logged |
| `INVOICEAPP__MAX_SPACES_PER_USER` | no | `3` | spaces a user may create and own (≥ 1; invited memberships do not count) → `POST /api/spaces` 403 `space_limit` |
| `INVOICEAPP__REGISTRATION` | no | `false` | public registration on the base host and creating spaces (`false` → `POST /api/spaces` 403 `space_creation_disabled`); `true` needs SMTP (start refused otherwise) |
| `INVOICEAPP__TRUST_FORWARDED` | no | `false` | client IP for the auth rate limits from the **last** `X-Forwarded-For` hop (the proxy must append, not pass the client's header through); must be `true` behind a proxy, with the app port not reachable directly (`false` + an `X-Forwarded-For` request → one warning) |
| `INVOICEAPP__GOOGLE__CLIENT_ID`, `INVOICEAPP__GOOGLE__CLIENT_SECRET` | no | — | Sign in with Google: both or neither (one only → start refused); redirect URI `{INVOICEAPP__PUBLIC_URL}/api/auth/google/callback` |
| `INVOICEAPP__GOOGLE__ISSUER` | no | `https://accounts.google.com` | OpenID discovery base (https; http only for a loopback host, e.g. a test mock) |
| ~~`INVOICEAPP__API_TOKEN`~~ | — | — | removed in 4a (ignored) |
| `INVOICEAPP__BIND` | no | `0.0.0.0:3000` | listen address |
| `INVOICEAPP__ARES_URL` | no | `https://ares.gov.cz/ekonomicke-subjekty-v-be/rest` | ARES REST root; tests point it at a local mock |
| `INVOICEAPP__CNB_URL` | no | `https://www.cnb.cz/cs/financni-trhy/devizovy-trh/kurzy-devizoveho-trhu/kurzy-devizoveho-trhu/denni_kurz.txt` | ČNB daily rates (`?date=DD.MM.YYYY` appended); tests point it at a local mock |
| `INVOICEAPP__MDCAST_URL` | no | `https://mdcast.nexial.cz` | mdcast root (empty → default); tests point it at a local mock |
| `INVOICEAPP__MDCAST_TOKEN` | no | — | Bearer for mdcast |
| `INVOICEAPP__MDCAST_CONCURRENCY` | no | `4` | concurrent mdcast renders of the instance (≥ 1); a render waits ≤ 10 s for a slot, then 503 `pdf_unavailable` `busy` |
| `INVOICEAPP__DESIGN_CACHE_MB` | no | `128` | in-memory cache of downloaded design files, all spaces together (≥ 1 MiB; least recently used evicted first, a file larger than the whole cache is not cached) |
| `INVOICEAPP__STORAGE_KIND` | no | `fs` | `fs` or `s3`; every key is under `spaces/{space id}/` (archives, originals, e-mail templates, design overrides / draft / versions — layout in [api/pdf.md](api/pdf.md), [api/design.md](api/design.md)); the DB stores the space-relative key |
| `INVOICEAPP__STORAGE_DIR` | no | `./data` | `fs` root, created at start; image: `/data` volume |
| `INVOICEAPP__S3__ENDPOINT` | no | — (AWS) | `s3`: `http(s)://` endpoint (Garage, MinIO, R2…); without path-style the bucket is prefixed to its host (`https://s3.example` → `https://{bucket}.s3.example`, unchanged when the host already starts with `{bucket}.`) |
| `INVOICEAPP__S3__BUCKET` | `s3` | — | bucket |
| `INVOICEAPP__S3__REGION` | no | `us-east-1` | Garage: `garage` |
| `INVOICEAPP__S3__ACCESS_KEY_ID`, `INVOICEAPP__S3__SECRET_ACCESS_KEY` | `s3` | — | credentials (never logged) |
| `INVOICEAPP__S3__PATH_STYLE` | no | `false` | `true` for path-style requests (Garage, MinIO) |
| ~~`INVOICEAPP__DESIGN_DIR`~~ | — | — | removed; a non-empty value refuses the start and points to `invoiceapp storage migrate --design-dir` / `invoiceapp design push` (a silent fallback to the default design would end up in immutable archives) |
| ~~`INVOICE__*`~~ | — | — | the prefix before the InvoiceApp rename (#53): any such variable refuses every command that reads the configuration, naming each one and its `INVOICEAPP__*` replacement (`INVOICE__DESIGN_DIR` as removed) |
| `INVOICEAPP__SMTP__HOST` | no | — | SMTP server; unset → e-mail not configured (sending → 503 `smtp_not_configured`) |
| `INVOICEAPP__SMTP__PORT` | no | by TLS mode | `starttls` 587, `tls` 465, `none` 25 |
| `INVOICEAPP__SMTP__TLS` | no | `starttls` | `starttls` \| `tls` (implicit) \| `none` (plain; tests / local relay) |
| `INVOICEAPP__SMTP__USERNAME`, `INVOICEAPP__SMTP__PASSWORD` | no | — | both or neither (password never logged); with `TLS=none` only to a loopback host (`localhost`, `127.0.0.0/8`, `::1`) |
| `INVOICEAPP__SMTP__FROM` | with HOST | — | `addr@x` or `Name <addr@x>`; its domain is the `Message-ID` domain |
| `RUST_LOG` | no | `invoiceapp=info,tower_http=info,sea_orm_migration=info` | |
| `TEST_DATABASE_URL` | tests | `…@localhost:5432/invoice_test` | integration tests (they also need `xmllint` from `libxml2-utils` for the vendored ISDOC, Pohoda and Money S3 schemas in `tests/fixtures/{isdoc,pohoda,money}/schema/`) |
| `TEST_S3_ENDPOINT`, `TEST_S3_BUCKET`, `TEST_S3_REGION`, `TEST_S3_ACCESS_KEY_ID`, `TEST_S3_SECRET_ACCESS_KEY` | tests | — | S3 test bucket (path-style) for `tests/storage.rs` (every case on fs and S3) and the app-on-S3 tests; random prefix per test, deleted after; fail, not skip, without them. Makefile: from `.env`; CI: secrets of the same names |
| `TEST_SECRET_KEY` | no | fixed test key | `INVOICEAPP__SECRET_KEY` of the test apps (`tests/common/mod.rs`) |

Empty values mean the default for the optional settings ([api/pdf.md](api/pdf.md) lists the storage / mdcast ones).

## Tests (`tests/`)

Each integration test file is its own binary over `tests/common/`. `TestDb::new()` creates a throwaway schema with
every migration and one seeded space (`acme`, `TEST_HOST`, owner token `TEST_TOKEN`); requests without `Host` go to
it. Routers come from `TestApp::builder(&db.conn)` (`tests/common/builder.rs`): defaults are a fresh mock mdcast, the
binary's shared fs storage, no SMTP / Google, registration off; options `.ares(url)`, `.cnb(url)`,
`.mdcast(&PdfEnv)` / `.mdcast_url(url)` / `.storage(s)`, `.smtp(&config)`, `.google(issuer)` + `.jwks_refresh(d)`,
`.registration(b)`, `.trust_forwarded(b)`, `.state(|s| …)` → `.build()` or `.build_with_pdf()` (+ `PdfRoot`). The
older helpers (`router*`, `PdfEnv::router*`, `EmailEnv::router` / `unconfigured`, `auth::mail_app`, `google::app*`)
are thin wrappers over it. Mocks: mdcast (`mdcast.rs`), SMTP (`smtp.rs`), OpenID provider (`oidc.rs`).

## Frontend (`frontend/`)

Vue 3 + TS + Vite + Tailwind v4 + Pinia + vue-router + vue-i18n (cs default, en).

Feature code lives in `src/features/<feature>/`: `api.ts` (calls through `request()`), `types.ts` (wire types —
decimals as strings, nullable as `| null`; responses alias the generated schema via `src/api/wire.ts`, request bodies
are hand-written), Pinia store(s), `views/` (routed) and `components/`. Form state is all-strings drafts with pure
`toDraft` / `validate*` / `to<Wire>` helpers, unit-tested outside components. Client validation is light; the
server's 422 is the source of truth and its `fields` are shown inline (422 `lines.N.field` / `vatRecap.N.*` keys are
routed to the row).

### Shell and shared code

| Path | Role |
|---|---|
| `src/api/client.ts` | fetch wrapper: same-origin cookie (the browser adds `Origin` for the CSRF check), `ApiError{status, code, fields, detail}` (status 0 = network); 401 → session cleared + `/login?return=` unless `quiet401` (the session probe, the login itself); 403 `email_unverified` → `/verify` (handlers set in `main.ts`) |
| `src/api/client.ts` (bodies) | `request()` for JSON; a `FormData` body goes out without a Content-Type so the browser sets the multipart boundary; `requestBlob()` → `{blob, filename}` (`filename*` wins) |
| `src/api/schema.d.ts` | **generated**: `make frontend-api-types` runs `invoiceapp openapi` → `openapi-typescript`; committed; `make lint` (`api-types-check`) regenerates it into `target/` and fails on a diff |
| `src/api/wire.ts`, `src/api/types.ts` | `Wire<"Name">` (every field present, serde writes `null`), `Narrow<T, {…}>` (backend drift fails the type-check), `allOf<T>()`; shared wire types and document enums (from the schema) |
| `src/lib/money.ts` | exact BigInt cents — never `Number(x) * 100`; credit notes are stored positive and shown negated (`negate` / `signed`) |
| `src/lib/pdf.ts` | `openPdf` opens the tab synchronously **before** the fetch (a `window.open` after a slow render loses the user activation and is popup-blocked), then points it at the blob; blocked → current tab; failure → closes the blank tab; object URLs revoked after 5 min. `downloadPdf` clicks an `<a download>` |
| `src/lib/loadOnce.ts` | a store's `ensure()`: concurrent callers share the in-flight promise; a failure rejects every waiter and the next call retries |
| `src/lib/` (rest) | `formErrors.ts` (422 / error code → i18n key, `errorDetailOf`), `ico.ts`, `format.ts` (display only), `bytes.ts`, `storage.ts` (guarded `localStorage`), `returnPath.ts` (same-app paths only), `navigate.ts` (`leaveTo` another host) |
| `src/composables/`, `src/components/` | `useFormSubmit` (validate → request → inline 422), `useAction`, `usePdf` (busy, error, render `detail`); `form/` (`FormField`, `PartyFields` + ARES merge), `UsageBar`, `ErrorDetail` (typst diagnostics) |
| `src/boot.ts` | `loadContext()` before mount (`GET /api/context`); unknown host (404; base URL guessed by dropping the first label) or backend down → `BootFailureView` |
| `src/stores/` | `session.ts` (host context, user, `can(action)`, `login` → `"mfa"` when the code step follows), `toast.ts` (one message, 4 s), `health.ts` (version in the footer) |
| `src/router/` | routes chosen by the boot context: `authRoutes` (every host: login, forgot, account), `baseRoutes` (hub, register only while registration is on, verify, reset), `spaceRoutes`; `authGuard` (`meta.public`, `guestOnly`, `can` = minimum role) |
| `src/locales/{cs,en}/<namespace>.json` | one file per namespace, precompiled by `@intlify/unplugin-vue-i18n` (runtime-only vue-i18n), merged per locale in `src/i18n/messages/`; `main.ts` awaits `loadInitialLocale()` before mount, the other locale loads on the first switch; parity test: same files and keys, no empty message; no literal `{`, `@` or `\|` |
| `src/test-utils.ts`, `src/style.css` | fetch stubs (`mockFetchRoutes` by `"METHOD /path"`); Tailwind + shared classes (`.input`, `.btn*`, `.card`, `.table`, `.badge`, `.alert-error`) |

### Features (non-obvious rules)

- **settings**: CRUD stores reload the list after each mutation (the server owns the default flags); categories in use
  (409) → "deactivate instead"; custom field key / type immutable after create; `numberPattern.ts` mirrors `Pattern`
  (`{C}` only in a client pattern, previewed with the code `ACME`); issued series also edit their client pattern
  (`ClientPatternForm`, "" → null); `CountersEditor` (counters per year, any value ≥ 0) is shared with the contact's
  client sequences; the design store reloads per visit. Accounting tab: the draft lists issued / received × the six exported types in a
  fixed order, so row index = the `N` of the 422 keys `<program>.codes.N.<field>`; `""` → null; limits come from the
  GET / PUT response; one Save sends both programs.
- **documents** (issued, every type): every action stores the server's answer, payments reload the document;
  `lines.ts` remaps subtotal refs (1-based) on move / remove; `catalogLines.ts`: a group → member lines + a subtotal
  with the group's collapse, an item of another currency gets an empty price; `useCompute.ts` debounces 300 ms and ignores stale answers;
  `useIndicativeRate.ts` is shown, never saved (the manual `exchangeRate` is cleared on a currency change); issue maps
  `pdf_unavailable` / `pdf_render_failed` / `storage_unavailable` to "not issued" + detail; simplified documents over
  10 000 CZK only warn. Manual import: `?imported=1`, own number, only imports send `relatedDocumentId`;
  `RelatedDocumentPicker` offers issued targets only, first 50. Lists show an "Importovaný" badge and a "bez PDF" hint.
  Unlock (admin+): `UnlockPanel` asks a required reason and warns about sent / paid / referenced documents;
  `UnlockedBanner` shows when / who / why on the draft and, as a note, after re-issue. Number of a native draft
  (`DocumentNumberField`, [numbering.md](api/numbering.md)): empty = allocated at issue, with the expected number from
  `useNumberPreview.ts` (debounced on doc type / customer / issue date, failures show no preview); typed = a manual
  number; an unlocked draft starts with its kept number. `number` is sent only once touched (`numberTouched`):
  omitted keeps it, `null` ("Assign new number from sequence") clears it. `previousNumber` shows on the detail.
- **contacts**: the form always sends `numberCode` (`PUT` replaces every field); the edit view lists the customer's
  client sequences (`ContactNumberCounters`, `useContactNumberingStore`), counters editable by admins.
- **received** (no draft): VAT suggested as round2(base × rate) until typed; `receivedDate` follows tax point ?? issue
  date and `payable` follows the total until edited; `vatDeductible` resets on a VAT-mode change. A received document
  opened under `/invoices/…` redirects to `/received/…` and vice versa. `OriginalPdfPanel` (received + imported):
  PDF ≤ 20 MB checked client-side, 404 `pdf_missing` → "PDF nenahráno".
- **isdoc / imports / csvImport**: files ≤ 50 MiB checked client-side; `ok` entries preselected; batch `categoryId` /
  `vatDeductible` only sent while a received entry is selected; ISDOC lines of a received document are read-only (a
  received PUT sends no lines). Unknown error / warning codes fall back to showing the code.
- **overview** (`/`, route `home`, first nav item): the period lives in the URL query (`period.ts`: `fromQuery` /
  `toQuery`, presets normalized, anything unusable → the current month); `chart.ts` lays out the hand-rolled SVG bars
  (shared zero baseline, rounded data ends); the VAT block is hidden when `vatPayer` is false; a slower answer for a
  period already left is dropped.
- **csvExport**: `period.ts` `previousMonth()` (January → December of the previous year); Pohoda XML is
  Windows-1250 and saved as raw bytes; 422 `filter` `too_many`, `documents: unexportable` (+ detail), `from: empty`.
- **email**: the template editor stays mounted (`v-show`) so edits survive tab switches; switching the send dialog's
  locale re-fetches subject + body (after a confirm when edited), recipients stay; "Bcc to me" = the company e-mail;
  an empty test address → the company e-mail. `recipients.ts` splits on `,` `;` newline, keeps `Name <addr>`
  as one chip, dedupes case-insensitively.
- **metadata**: unsaved edits survive reloads of the same document; values of fields without an applicable
  definition are sent back unchanged; decimal comma → dot.

Dev: Vite proxies `/api` → `http://localhost:3000` and keeps the `Host` header (`changeOrigin: false`), because the
backend picks the base app or a space by host. Run the server with `INVOICEAPP__PUBLIC_URL=http://localhost:5173`, open
`http://localhost:5173` (base host) and `http://<slug>.localhost:5173` (a space); `*.localhost` resolves to loopback
without `/etc/hosts`.

## Build, CI, image

- `Dockerfile`: node:22 builds the SPA → `rust:1.97.0-bookworm` builds the release binary with the SPA and `design/`
  embedded (`--locked`) → `debian:bookworm-slim`, non-root uid 10001, `INVOICEAPP__STORAGE_DIR=/data` (`VOLUME /data`,
  owned by 10001; unused with `INVOICEAPP__STORAGE_KIND=s3`), `EXPOSE 3000`, `invoiceapp serve`.
- `.github/workflows/ci.yml`: PR + push to `master`; backend job (Postgres service, `TEST_S3_*` secrets,
  `libxml2-utils`, `make lint-backend test-backend`, toolchain pin check, `make frontend-install api-types-check`),
  frontend job (`make frontend-install lint-frontend test-frontend frontend-build`).
- `.github/workflows/docker.yml`: `v*` tag → `ghcr.io/${{ github.repository }}:<version>` + `latest`.
- Deployment manifests are out of scope for now.
