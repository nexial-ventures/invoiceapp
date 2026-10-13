# InvoiceApp

InvoiceApp ([invoiceapp.cz](https://invoiceapp.cz)) — invoice management in spaces (one space = one own company, users with roles): issued and received invoices, Czech VAT, ISDOC,
CSV / XLSX import, CSV and Pohoda XML export for the accountant, an MCP endpoint
for AI clients,
PDF rendering through [mdcast](https://github.com/xmiksay/mdcast) with a
per-space design. Rust (Axum + SeaORM/Postgres) with an embedded Vue 3 SPA.

Status: phases 1–4 of [docs/plan.md](docs/plan.md) are implemented (core documents, interchange, accounting
exports + MCP, multi-tenant spaces with roles, TOTP, Sign in with Google and per-space designs); scope and what is
out of scope are in the plan, the layout in [docs/architecture.md](docs/architecture.md).

## Quick start

```sh
cp .env.example .env            # set INVOICEAPP__PUBLIC_URL=http://localhost:5173 (the Vite origin), INVOICEAPP__SECRET_KEY (openssl rand -base64 32), registration + SMTP
# local Postgres on localhost:5432 — one-time role/db setup in .env.example
make frontend-install
make dev-server                 # backend on :3000
make dev-frontend               # Vite on :5173, proxies /api and keeps the Host header
```

Open `http://localhost:5173` (the base host) and `http://<slug>.localhost:5173` (a space); `*.localhost` resolves to
loopback without `/etc/hosts`. `INVOICEAPP__PUBLIC_URL` must be the origin the browser uses: e-mail and space links point there and the CSRF
check takes its scheme. Without Vite: `make frontend-build`, `INVOICEAPP__PUBLIC_URL=http://localhost:3000`, `make dev-server`
and open `http://localhost:3000` / `http://<slug>.localhost:3000`.

Bootstrap = registration: with `INVOICEAPP__REGISTRATION=true` (needs SMTP) register on the base host
(`http://localhost:5173`), confirm the e-mail, create a space (`firma`) and log in on its host
(`http://firma.localhost:5173`). Disable registration afterwards if you want (this also disables creating further spaces, including for you). Hosts, roles and tokens:
[docs/api/spaces.md](docs/api/spaces.md), [docs/api/auth.md](docs/api/auth.md).

## Make targets

| Target | Description |
|---|---|
| `make build` | Build the SPA, then the release binary with the SPA embedded |
| `make run` / `make dev-server` | Run the server (`invoiceapp serve`) |
| `make dev-frontend` | Vite dev server |
| `make frontend-install` / `make frontend-build` | `npm ci` / production SPA build |
| `make fmt` / `make fmt-check` / `make clippy` | Rust formatting and lints |
| `make lint` | `lint-backend` + `lint-frontend` (eslint + vue-tsc) + `api-types-check` (generated API types up to date) |
| `make frontend-api-types` | Regenerate `frontend/src/api/schema.d.ts` from the backend's OpenAPI (`invoiceapp openapi`) after an API change |
| `make test-unit` / `make test-integration` | Rust unit / integration tests (integration needs Postgres, the S3 test bucket via `TEST_S3_*` in `.env`, and `xmllint` from `libxml2-utils`) |
| `make test` | `test-backend` + `test-frontend` |
| `make migrate` / `make migrate-status` | Apply / show migrations |
| `make docker-build` | Build the Docker image (`IMAGE=...` to override the tag) |
| `make clean` | Remove build artifacts |

## MCP (AI clients)

`POST /api/mcp` is a stateless [Model Context Protocol](https://modelcontextprotocol.io) server (Streamable HTTP)
of a space, authenticated by a personal API token of that space (API tokens page; the token's role limits the
tools — read tools need `accountant`, write tools `member`): read documents, contacts, ARES, catalog and settings, create contacts and drafts,
issue, record payments, mark sent. Tools and rules: [docs/api/mcp.md](docs/api/mcp.md). PDFs are not sent over
MCP — results carry `pdfUrl` for a download with the same token.

```sh
# Claude Code (the token works only on its space's host)
claude mcp add --transport http invoice https://firma.invoiceapp.cz/api/mcp \
  --header "Authorization: Bearer $INVOICE_TOKEN"
```

Other clients: an "HTTP" / "Streamable HTTP" server with that URL and the `Authorization` header.

## Configuration

See [.env.example](.env.example) and the table in [docs/architecture.md](docs/architecture.md#configuration).

Files (PDF archive, uploaded originals, design overrides) live in a local directory (`INVOICEAPP__STORAGE_KIND=fs`,
default, `INVOICEAPP__STORAGE_DIR`) or an S3-compatible bucket (`INVOICEAPP__STORAGE_KIND=s3`, `INVOICEAPP__S3__*`).

Sending documents by e-mail needs an SMTP server (`INVOICEAPP__SMTP__HOST`, `PORT`, `TLS`, `USERNAME`, `PASSWORD`,
`FROM`); without `INVOICEAPP__SMTP__HOST` the app runs with e-mail disabled.

```sh
invoiceapp storage migrate --from-dir ./data --design-dir ./my-design   # fs archive (+ old design dir) → configured storage
invoiceapp storage sweep --dry-run                                    # unreferenced document objects older than 1 h (guarded; --force)
invoiceapp design push ./my-design --space firma   # override design files of a space (invoice.typ, fonts/, logo.svg|png, signature.png)
invoiceapp design ls --space firma                 # effective design: custom vs built-in
invoiceapp design pull ./backup --space firma      # download the overrides
invoiceapp design rm logo.png --space firma        # back to the built-in file
```

## Upgrading to the InvoiceApp name

The project was renamed from `invoice` to **InvoiceApp** (#53). An existing deployment has to change:

- **Environment prefix** `INVOICE__*` → `INVOICEAPP__*` (e.g. `INVOICE__DATABASE_URL` → `INVOICEAPP__DATABASE_URL`,
  `INVOICE__SMTP__HOST` → `INVOICEAPP__SMTP__HOST`), and a `RUST_LOG` target `invoice=` → `invoiceapp=`.
- **Binary** `invoice` → `invoiceapp` (`invoiceapp serve|migrate|storage|design|openapi`; the image's entrypoint and
  system user changed with it), and the **image** follows the GitHub repository path (`ghcr.io/<owner>/<repo>`).
- **Start-up guard:** any leftover `INVOICE__*` variable refuses every command that reads the configuration
  (`serve`, `migrate`, `storage`, `design`) with an error naming each variable and its `INVOICEAPP__*` replacement —
  rename them all, nothing is read from the old prefix.

Not renamed: the Postgres database/role names, S3 buckets, storage keys and browser storage keys — no data migration.

## Deployment

The image (`ghcr.io/<owner>/<repo>:<version>` + `latest`, i.e. the GitHub repository's path) is built by
`.github/workflows/docker.yml` on a `v*` tag;
`make docker-build` builds it locally. It runs `invoiceapp serve`, which applies pending migrations at startup and
listens on `:3000`. Required: `INVOICEAPP__DATABASE_URL` (Postgres), `INVOICEAPP__PUBLIC_URL` and `INVOICEAPP__SECRET_KEY`;
PDFs need an mdcast (`INVOICEAPP__MDCAST_URL`, `INVOICEAPP__MDCAST_TOKEN`).

- **DNS and TLS:** the base host of `INVOICEAPP__PUBLIC_URL` (registration, login, "my spaces") and every space at
  `{slug}.{base host}` — a wildcard DNS record and a wildcard certificate for `*.{base host}` plus one for the base
  host itself.
- **Reverse proxy:** pass the original `Host` (the app picks the base host or a space by it; an unknown host is 404)
  and **append** to `X-Forwarded-For` rather than passing the client's header through, then set
  `INVOICEAPP__TRUST_FORWARDED=true` — the auth rate limits take the last hop as the client IP (IPv6 by its /64).
  Behind a proxy this **must** be `true`, and the app port (`:3000`) **must not be reachable directly** (bind it to
  the proxy's network only): otherwise all clients share the proxy's rate-limit buckets, or a direct client forges
  the header. With `false` the app logs a warning the first time a request carries `X-Forwarded-For`.
- **`INVOICEAPP__PUBLIC_URL`** must be the external `https://…` URL the browser sees, not the proxy's upstream: its scheme +
  the request `Host` form the origin the CSRF check compares `Origin` with (an `http://` value behind a TLS proxy makes every login and
  cookie mutation 403 `csrf`), the Google redirect URI and the links in e-mails and between spaces; session cookies
  are also `Secure` only with `https`.
- **Security headers** are set by the app on every response (CSP `'self'` only, `nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: strict-origin-when-cross-origin`, and HSTS `max-age=31536000; includeSubDomains` when
  `INVOICEAPP__PUBLIC_URL` is https). `includeSubDomains` pins **every** subdomain of the base host to https for a
  year, not only the spaces: give the app a **dedicated base host** (e.g. `invoice.example.com`), not an apex
  domain shared with other services that may still need plain http. Do not add a conflicting CSP in the proxy; [docs/api/README.md](docs/api/README.md#clarifications-as-implemented) lists the policy.
- **`INVOICEAPP__SECRET_KEY`** (`openssl rand -base64 32`) is permanent: changing or losing it makes every enrolled
  TOTP secret and every recovery code unusable.
- **`INVOICEAPP__REGISTRATION=true`** requires SMTP (`INVOICEAPP__SMTP__*`); the start is refused otherwise. `false` also
  disables creating spaces (403 `space_creation_disabled`): only people invited into existing spaces can enter.
- **mdcast** renders each space's own typst templates: run it **sandboxed** (no file system or network access
  beyond the request, resource limits) — template isolation is mdcast's, not the app's.
- **Render limits:** at most `INVOICEAPP__MDCAST_CONCURRENCY` (default 4) renders run at once per instance — size it to
  what the mdcast behind it handles; a render that waits over 10 s for a slot is 503 `pdf_unavailable` (`busy`).
  Previews (sample, design draft, draft documents) are limited to one in flight and 60 / hour per space with a 20 s
  timeout; issuing is never refused by them. The limits are in memory per instance (several replicas multiply
  them). A timed-out preview may still run to its end in mdcast; previews never take the last slot.
- **E-mail templates** render in a child process of the same binary (`invoiceapp render-template`, hidden) with
  `RLIMIT_DATA` 256 MiB and `RLIMIT_CPU` 5 s on **Linux** (other platforms: only the 5 s wall-clock kill), at most 2
  at once. The container needs nothing extra (no seccomp / capability changes).
- **Orphan sweep:** `serve` deletes document objects no row references (left by a crash between the object write and
  its commit) once a day, the first time 1 hour after the start, only when older than 1 hour and only for spaces
  that exist. It deletes nothing (logs an error) when no row references any object or the orphans exceed
  max(100, 10 %) — check the database, then run `invoiceapp storage sweep --force`. `--dry-run` lists them (needs
  `INVOICEAPP__DATABASE_URL` and the storage settings).
- **Restoring an older database snapshot** while the storage keeps newer objects: the newer archives and originals
  look like orphans to the sweep. Stop the server (or restore within its first hour and fix the storage before the
  sweep runs), restore, then check `invoiceapp storage sweep --dry-run`; below the guard's threshold the sweep **would
  delete** them, so copy them aside or restore the storage to the same point first.
- **Sign in with Google** (optional): the only redirect URI to register is
  `{INVOICEAPP__PUBLIC_URL}/api/auth/google/callback`.
- **PgBouncer:** in transaction mode set `INVOICEAPP__DB_PGBOUNCER=true` (otherwise every connection fails with
  `unsupported startup parameter in options`); the pool then runs without its 30 s statement / idle-in-transaction
  timeouts.
- **Storage:** the image's `/data` volume (`INVOICEAPP__STORAGE_KIND=fs`, default) or an S3-compatible bucket
  (`INVOICEAPP__STORAGE_KIND=s3`, `INVOICEAPP__S3__*`).
- **Space limits:** every space has a hidden storage quota (default 100 MB: originals, archived PDFs, design files;
  issuing is never blocked) and outbound e-mail limits (document, test and invitation e-mails: 100 / day and 20 / h
  per space, the same per sending user within the space; over → 429 `rate_limited`). There is no API or admin
  UI for them — change a space's row in the database ([docs/api/spaces.md](docs/api/spaces.md#limits)):
  ```sql
  UPDATE space_limits SET storage_quota_bytes = 500 * 1024 * 1024,
         email_space_per_day = 500, email_user_per_day = 500, updated_at = now()
  WHERE space_id = (SELECT id FROM spaces WHERE slug = 'firma');
  ```
- **Public registration safeguards:** a user may create and own at most `INVOICEAPP__MAX_SPACES_PER_USER` (default 3)
  spaces; phishing-looking and service slugs (`login`, `paypal`, `bank`, `faktura`, …) are reserved; a document
  e-mail has at most 10 recipients. `INVOICEAPP__EMAIL_DISABLED=true` is the kill switch for abuse: every space e-mail
  → 503 `email_disabled`, account e-mails (verify, reset, invitations) are skipped and logged.
