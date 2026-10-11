# invoice

Invoice management in spaces (one space = one own company, users with roles): issued and received invoices, Czech VAT, ISDOC,
CSV / XLSX import, CSV and Pohoda XML export for the accountant, an MCP endpoint
for AI clients,
PDF rendering through [mdcast](https://github.com/xmiksay/mdcast) with a
per-space design. Rust (Axum + SeaORM/Postgres) with an embedded Vue 3 SPA.

Status: phases 1–4 of [docs/plan.md](docs/plan.md) are implemented (core documents, interchange, accounting
exports + MCP, multi-tenant spaces with roles, TOTP, Sign in with Google and per-space designs); scope and what is
out of scope are in the plan, the layout in [docs/architecture.md](docs/architecture.md).

## Quick start

```sh
cp .env.example .env            # set INVOICE__PUBLIC_URL=http://localhost:5173 (the Vite origin), INVOICE__SECRET_KEY (openssl rand -base64 32), registration + SMTP
# local Postgres on localhost:5432 — one-time role/db setup in .env.example
make frontend-install
make dev-server                 # backend on :3000
make dev-frontend               # Vite on :5173, proxies /api and keeps the Host header
```

Open `http://localhost:5173` (the base host) and `http://<slug>.localhost:5173` (a space); `*.localhost` resolves to
loopback without `/etc/hosts`. `INVOICE__PUBLIC_URL` must be the origin the browser uses: e-mail and space links point there and the CSRF
check takes its scheme. Without Vite: `make frontend-build`, `INVOICE__PUBLIC_URL=http://localhost:3000`, `make dev-server`
and open `http://localhost:3000` / `http://<slug>.localhost:3000`.

Bootstrap = registration: with `INVOICE__REGISTRATION=true` (needs SMTP) register on the base host
(`http://localhost:5173`), confirm the e-mail, create a space (`firma`) and log in on its host
(`http://firma.localhost:5173`). Disable registration afterwards if you want (this also disables creating further spaces, including for you). Hosts, roles and tokens:
[docs/api/spaces.md](docs/api/spaces.md), [docs/api/auth.md](docs/api/auth.md).

## Make targets

| Target | Description |
|---|---|
| `make build` | Build the SPA, then the release binary with the SPA embedded |
| `make run` / `make dev-server` | Run the server (`invoice serve`) |
| `make dev-frontend` | Vite dev server |
| `make frontend-install` / `make frontend-build` | `npm ci` / production SPA build |
| `make fmt` / `make fmt-check` / `make clippy` | Rust formatting and lints |
| `make lint` | `lint-backend` + `lint-frontend` (eslint + vue-tsc) |
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

Files (PDF archive, uploaded originals, design overrides) live in a local directory (`INVOICE__STORAGE_KIND=fs`,
default, `INVOICE__STORAGE_DIR`) or an S3-compatible bucket (`INVOICE__STORAGE_KIND=s3`, `INVOICE__S3__*`).

Sending documents by e-mail needs an SMTP server (`INVOICE__SMTP__HOST`, `PORT`, `TLS`, `USERNAME`, `PASSWORD`,
`FROM`); without `INVOICE__SMTP__HOST` the app runs with e-mail disabled.

```sh
invoice storage migrate --from-dir ./data --design-dir ./my-design   # fs archive (+ old design dir) → configured storage
invoice design push ./my-design --space firma   # override design files of a space (invoice.typ, fonts/, logo.svg|png, signature.png)
invoice design ls --space firma                 # effective design: custom vs built-in
invoice design pull ./backup --space firma      # download the overrides
invoice design rm logo.png --space firma        # back to the built-in file
```

## Deployment

The image (`ghcr.io/xmiksay/invoice:<version>` + `latest`) is built by `.github/workflows/docker.yml` on a `v*` tag;
`make docker-build` builds it locally. It runs `invoice serve`, which applies pending migrations at startup and
listens on `:3000`. Required: `INVOICE__DATABASE_URL` (Postgres), `INVOICE__PUBLIC_URL` and `INVOICE__SECRET_KEY`;
PDFs need an mdcast (`INVOICE__MDCAST_URL`, `INVOICE__MDCAST_TOKEN`).

- **DNS and TLS:** the base host of `INVOICE__PUBLIC_URL` (registration, login, "my spaces") and every space at
  `{slug}.{base host}` — a wildcard DNS record and a wildcard certificate for `*.{base host}` plus one for the base
  host itself.
- **Reverse proxy:** pass the original `Host` (the app picks the base host or a space by it; an unknown host is 404)
  and **append** to `X-Forwarded-For` rather than passing the client's header through, then set
  `INVOICE__TRUST_FORWARDED=true` — the auth rate limits take the last hop as the client IP.
- **`INVOICE__PUBLIC_URL`** must be the external `https://…` URL the browser sees, not the proxy's upstream: its scheme +
  the request `Host` form the origin the CSRF check compares `Origin` with (an `http://` value behind a TLS proxy makes every login and
  cookie mutation 403 `csrf`), the Google redirect URI and the links in e-mails and between spaces; session cookies
  are also `Secure` only with `https`.
- **`INVOICE__SECRET_KEY`** (`openssl rand -base64 32`) is permanent: changing or losing it makes every enrolled
  TOTP secret and every recovery code unusable.
- **`INVOICE__REGISTRATION=true`** requires SMTP (`INVOICE__SMTP__*`); the start is refused otherwise. `false` also
  disables creating spaces (403 `space_creation_disabled`): only people invited into existing spaces can enter.
- **Sign in with Google** (optional): the only redirect URI to register is
  `{INVOICE__PUBLIC_URL}/api/auth/google/callback`.
- **Storage:** the image's `/data` volume (`INVOICE__STORAGE_KIND=fs`, default) or an S3-compatible bucket
  (`INVOICE__STORAGE_KIND=s3`, `INVOICE__S3__*`).
