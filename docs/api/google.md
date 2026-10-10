# Sign in with Google (4d)

OpenID Connect login with Google (#6): authorization code + PKCE. Accounts and sessions: [auth.md](auth.md); hosts
and spaces: [spaces.md](spaces.md); invitations: [members.md](members.md); TOTP and step-up: [mfa.md](mfa.md).

## Config
- `INVOICE__GOOGLE__CLIENT_ID`, `INVOICE__GOOGLE__CLIENT_SECRET`: both set → Google enabled; neither → disabled
  (the routes 404, the buttons are hidden); only one → startup fails.
- `INVOICE__GOOGLE__ISSUER` (default `https://accounts.google.com`): discovery base, overridable for tests (a mock
  provider). Discovery and JWKS are fetched lazily and cached (the JWKS for its `Cache-Control: max-age`, default
  1 h, at most 24 h; refetched on an unknown `kid`, at most once a minute; concurrent callers wait for one fetch).
- **Redirect URI** = `{INVOICE__PUBLIC_URL}/api/auth/google/callback` — the only one to register in Google Cloud
  Console. Google does not allow wildcard redirect URIs, so the callback is always on the **base host**.
- `GET /api/context` gains `google: boolean`.

## Flow
0. **link only** (review decision): `POST /api/account/google/link { password, code? }` (session only, a verified
   user — else 403 `email_unverified`; password + TOTP step-up as for unlink: `password: invalid | not_set`,
   `code: required | invalid`) → `200 { url }` = the start URL with a single-use **link grant** (5 min, bound to the
   user); the SPA navigates to `url`. A password-less user is already Google-linked, so `not_set` does not block a
   real use.
1. `GET /api/auth/google/start?purpose=login|link|invite&return=<path>&invite=<token>&grant=<grant>` on any known
   host (base or a space) → 302 to Google's authorization endpoint (`scope=openid email profile`, `response_type=code`,
   `code_challenge` S256, `nonce`, `state`, `prompt=select_account`). Stores a **flow row** (state sha256, the
   origin host = space id or null for base, purpose, PKCE verifier, nonce, `return` (a relative path, else `/`),
   invite token hash for `invite`, the session's user for `link`; 10 min, single use) and sets this flow's
   **browser-binding cookie** `invoice_google_<12 hex>` (one per flow) on the origin host. `link` needs a session on
   that host **and** the grant of step 0 (used up here); otherwise → error redirect `invalid`. Rate limited per IP
   (20 / 15 min).
2. `GET /api/auth/google/callback?state&code` (base host) → validates the flow row (unknown / expired / used → error
   redirect), exchanges the code (client secret + PKCE verifier), verifies the **ID token**: signature via JWKS
   (RS256), `iss`, `aud` = client id, `azp` (required and = client id when `aud` has several values; = client id
   whenever present), `exp`, `iat` not in the future (60 s skew), `nonce`, `email_verified` true. It **writes no
   account**: it creates a **handoff token** (32 random bytes, sha256 stored, 60 s, single use, bound to the flow's
   origin host, carrying the verified claims `sub` / `email` / `name`, purpose, `return`, the `link` user) → 302 to
   `{origin host}/api/auth/google/finish?token=…`.
3. `GET /api/auth/google/finish?token` (the origin host) → consumes the handoff, checks the flow's browser-binding
   cookie, **then** resolves the user (below — the only place accounts are created, linked or claimed) and
   completes the purpose; 302 to the SPA. The handoff is the only cross-host transfer; password login still never
   crosses hosts.

Errors at any step → 302 to `{origin host}/login?google_error=<code>` (or `/account?google_error=` for `link`,
`/invite?token=…&google_error=` for `invite`); codes: `cancelled` (user denied), `expired`, `invalid` (state, token,
signature, claims), `unverified_email`, `not_allowed` (no account and registration off and no invitation),
`disabled`, `not_member`, `mfa_required`, `already_linked`, `email_mismatch`, `unavailable` (Google unreachable).
Details are logged, never put in the URL. Internal failures (database…) in `start`, `callback` and `finish` are
redirects with `unavailable` too (logged), never a JSON 500.

## Resolving the user (finish, after the browser binding)
1. A user with this Google `sub` → that user.
2. Else a user with the same e-mail (lowercased) → **linked automatically** (`sub` stored; Google's verified e-mail is
   the proof). Their password, TOTP and memberships are unchanged.
3. Else **new account** (verified, no password, display name = the Google `name`, else the e-mail's local part)
   when `INVOICE__REGISTRATION` is true, or when the purpose is `invite` and the invitation's e-mail equals the Google
   e-mail. Otherwise `not_allowed`.
- Disabled user → `disabled`. A `sub` already linked to another user (purpose `link`) → `already_linked`.
- Only a **verified** user may link a Google account. A `sub` linked while the address was unproven is removed when
  the address gets verified by any means (verify link, password reset, invitation accept, the owner's Google login
  claiming the account).

## Finishing (origin host)
- **login** on the base host → base session. On a space host → the user must be a member (`not_member`); the space
  policy and TOTP rules of [mfa.md](mfa.md) apply exactly as for a password login: a user **with TOTP** gets the
  pending `invoice_mfa` cookie and is sent to `/login?mfa=1` (the SPA shows the code step); a user without TOTP on a
  requiring space → `mfa_required`. Otherwise a session (cookie) → 302 to `return`.
- **invite**: the Google e-mail must equal the invitation e-mail (`email_mismatch`); then the 4b/4c accept rules
  (inviter re-check, MFA policy: no TOTP on a requiring space → `mfa_required` and the invitation stays; with TOTP →
  the code step, the membership is created only after the code) → membership + session → `return` (`/`).
- **link** (from the account page via step 0, session on that host, verified user): attaches the Google `sub` to the session's user — any
  verified Google e-mail may be linked; `already_linked` if that `sub` belongs to someone else; the user is not
  logged in anew. → `/account?google=linked`.
- Sessions record their login method (`password` | `google`) and time.

## Accounts without a password
- `password_hash` becomes nullable; `GET /api/auth/me` `user` gains `hasPassword` and `googleLinked`.
- Password login for a Google-only account → 401 `invalid_credentials` (as a wrong password).
- **Setting a password:** the existing e-mail reset flow works for any account; and `POST /api/account/password
  { newPassword, code? }` **without** `currentPassword` is accepted only when the user has no password and the
  current session was created by Google less than 10 minutes ago (else 422 `currentPassword: required`); TOTP
  step-up applies as in mfa.md.
- Every action that asks for the user's password (space delete, MFA setup / disable / regenerate, invite accept with
  an existing account, unlink) returns 422 `password: not_set` for a user without a password; the UI explains how to
  set one.
- `POST /api/account/google/link { password, code? }` → `200 { url }` (step 0 above).
- `DELETE /api/account/google { password, code? }` → 204 unlink; needs a password set (`password: not_set`), so an
  account never loses its last login method.

## UI
- "Přihlásit přes Google" button on the base and space login pages and on the invite page (when `context.google`);
  "Registrovat přes Google" on the register page when registration is on.
- `google_error` codes shown as readable messages; `/login?mfa=1` opens the code step directly.
- Account: Google section (linked / not linked, "Propojit s Googlem" with password + code → navigate to the returned
  `url`, "Odpojit" with password + code); for an account
  without a password a "Nastavit heslo" form (no current password, within 10 min of a Google login, else a hint to
  use "Zapomenuté heslo"), and password-requiring forms replaced by that hint.

## Tests
- A mock OIDC provider in `tests/common` (discovery, JWKS with a test RSA key, token endpoint issuing signed ID
  tokens with chosen claims) — never the real Google.
- Unit: ID token validation (signature, wrong key / kid, iss, aud, exp, iat, nonce, email_verified), PKCE, return
  path sanitising.
- Integration: start → callback → finish for base login, space login (member / not member), new account with
  registration on / off, invite (match / mismatch, MFA policy), auto-link by e-mail, explicit link / already
  linked, TOTP user → code step, requiring space without TOTP, state / handoff expired / reused / wrong host,
  Google error and token-endpoint failure, Google-only account: password login refused, set password within /
  after 10 min, `password: not_set` on the listed routes, unlink rules, disabled config → 404.

## Clarifications (as implemented)
- **Config**: id and secret are trimmed; both empty = disabled, exactly one → the start is refused. `ISSUER` must be
  `https://…` (`http://` only for a loopback host — the test mock); a trailing `/` is dropped. The discovery
  document's `issuer` must equal it (else `unavailable`). Discovery is cached 24 h. The JWKS sits behind one async
  lock: on a cold or expired cache the first caller fetches and the others wait for that result; it lives for the
  response's `Cache-Control: max-age` (capped at 24 h; missing / unparsable → 1 h; `no-store` / `no-cache` → refetched
  on next use). An unknown `kid` refetches once when the last fetch is ≥ 60 s old — or takes the newer set another
  caller fetched meanwhile; within the 60 s the token is `invalid`. A failed JWKS fetch is `unavailable`. ID tokens:
  RS256 only, a `kid` is required (a JWK with another `alg` is refused); for the default issuer `iss` may also be
  `accounts.google.com`; `azp` as in the flow; `email_verified` may be a boolean or `"true"`; the e-mail is trimmed +
  lowercased; `sub` must be non-empty. The code exchange posts the client secret in the form body (`client_secret_post`); the
  provider unreachable / HTTP 5xx → `unavailable`, HTTP 4xx (e.g. `invalid_grant`) → `invalid`. Client secret,
  codes, PKCE verifier, ID tokens, state and handoff tokens are never logged.
- **Routes**: `start`, `callback`, `finish` are browser navigations (no `Origin` rule); all three answer 404 JSON
  when Google is off, `callback` also on a space host. Every redirect is `302` with `Cache-Control: no-store` and
  `Referrer-Policy: no-referrer`. Past the 404 and the start's rate limit every failure is a redirect: internal ones
  (database, random source) are logged and answered `unavailable` (an unusable state / flow → on the base host).
- **start**: the per-IP limit (20 / 15 min, every request counts) is checked first and answers **429 JSON**
  `rate_limited` (not a redirect). An unknown `purpose` → `/login?google_error=invalid`. `link`: a session cookie
  valid on this host (a token does not count) and a live `grant` of that session's user, else
  `/account?google_error=invalid` (a grant presented with another user's session is refused and stays usable; a
  grant is used up by a successful start). `invite` needs a space host and a live invitation of that space → else
  `/invite?token=…&google_error=invalid` (`/login?…` when the token is missing or not base64url). A discovery
  failure → `unavailable`.
- **Link grant** (`POST /api/account/google/link`): 404 when Google is off; session only (token → 403 `forbidden`);
  unverified → 403 `email_unverified`; then password and step-up (failures in the login bucket). Stored like the
  verify / reset tokens (`user_tokens`, kind `google_link`, sha256, 5 min, single use). `url` points at the request's
  own host.
- **Browser binding (login-CSRF protection)**: `start` sets a host-only cookie `invoice_google_<first 12 hex of the
  flow id>` (random value, HttpOnly, SameSite=Lax, 10 min) on the origin host — one name per flow, so parallel
  sign-ins in one browser (two tabs / hosts) keep their own; flow and handoff keep the name and the value's sha256,
  and `finish` refuses a handoff without the matching cookie (`invalid`), so a callback URL produced by someone
  else's browser cannot sign the victim in. `finish` clears only its own cookie.
- **Flow**: the state is used up by the callback (`DELETE … RETURNING`). Unknown / used state → the origin is unknown:
  `{base}/login?google_error=invalid`; a flow whose space was deleted likewise. Expired → `expired` on the origin
  host, without a code exchange. Provider `error=access_denied` → `cancelled`, any other `error` or a missing `code`
  → `invalid`. Flows and handoffs expired are purged when a new flow starts.
- **Invitation token in the redirects**: rows store the invitation's sha256 (as `space_invites`) plus the token
  sealed with AES-256-GCM under `INVOICE__SECRET_KEY` (associated data = the space id), only to build
  `/invite?token=…&google_error=`.
- **Handoff**: used up only on its own host (another host → `invalid` without using it up); used → `invalid`;
  expired → `expired`. It carries the verified claims (`sub`, e-mail, name), the invitation hash (invite) and the
  user (link). The callback's checks before it are read-only (link: `already_linked` / unverified user; invite: live
  invitation, `email_mismatch`); `finish` repeats them before it writes.
- **Resolving** (in `finish`): by `sub` (disabled → `disabled`); else by e-mail: disabled → `disabled`; a verified
  account linked to another Google account → `already_linked`, else `sub` attached; an **unverified** account with
  that e-mail is **claimed** (the Google e-mail proves the owner, whoever registered it): its password, TOTP,
  recovery codes, sessions and pending logins are removed and any Google account linked to it while unverified is
  replaced; then it is verified and linked. Every other verification (`users::mark_verified`: verify link, password
  reset, invitation accept) clears a `google_sub` linked while unverified, in the same statement. New accounts only for a `login` on the **base host**
  with registration on, or an `invite` whose e-mail equals the Google e-mail. A `login` on a space host without an
  account → `not_member` (no account is created there). The new display name: `name` without control characters,
  ≤ 100 chars, else the e-mail's local part.
- **login (finish)**: disabled → `disabled`; on a space host unverified or not a member → `not_member`; then the
  shared password-login rule (`second_factor`): TOTP → pending login (`login_method = google`) + `/login?mfa=1`
  (the `return` path is not carried over), no TOTP on a requiring space → `mfa_required`, else a session
  (`login_method = google`) → `return`. An existing session cookie on that host is left alone.
- **New accounts** — one rule (`account::may_create`): for an invitation of exactly this e-mail, or a `login` on the
  base host while registration is on.
- **invite**: `finish` checks — before any account is created or linked — that the invitation is still live
  (`invalid`) and that its e-mail equals the Google e-mail (`email_mismatch`); after resolving, the account's own
  e-mail must equal it too (an account found by `sub` with another address → `email_mismatch`). Then the TOTP rule: no TOTP on a requiring space → `mfa_required` (invitation kept; an account the callback
  created stays — verified, linked, without membership); TOTP (any space) → pending login carrying the invitation
  hash → `/login?mfa=1`, and `POST /api/auth/login/mfa` runs the shared accept core (`members::join`: space lock,
  consume, inviter re-check, code, verified, membership) in a savepoint — a wrong code leaves the invitation usable
  and counts the failure; a gone invitation / failed inviter re-check → 401 `invalid_credentials` and the pending
  login ends; else the same core without a code → membership + session → `return` (default `/`).
- **link**: callback and `finish` refuse a `sub` of another user and a user already linked to a different Google
  account (`already_linked`, unlink first) and an unverified user (`invalid`); `finish` needs the session cookie of
  the same user on that host (else `invalid`), attaches conditionally (a race → `already_linked`), creates no
  session.
- **Passwords**: a user **with** a password keeps the old rule (`currentPassword` empty or wrong → `invalid`).
  Without one: `currentPassword` is ignored; allowed when the current session has `login_method = google` and was
  created < 10 min ago, else `currentPassword: required`; the step-up code applies; other sessions are revoked as
  for any change. `password: not_set` (nothing counted in the login bucket) on `DELETE /api/space`,
  `/api/account/mfa/setup`, `/disable`, `POST /api/invites/accept` (existing account; before the login buckets) and
  `DELETE /api/account/google`; `/recovery-codes` answers 409 first (a password-less account cannot have TOTP —
  setup needs the password).
- **Unlink**: session only (token → 403 `forbidden`), 404 when Google is off; password + step-up code; 204 also when
  nothing was linked; sessions stay.
- **Storage** (migration `m20261019_000001_google`): `users.password_hash` nullable, `users.google_sub` unique;
  `sessions.login_method`; `mfa_logins.login_method`, `invite_hash`; `google_flows` and `google_handoffs` (hashes
  unique, `space_id NULL` = base host, the binding cookie's name + value hash, the handoff's verified claims,
  cascades); `user_tokens.kind` gains `google_link`. `down` deletes password-less users (to restore `NOT NULL`).
- **UI (decided)**: a signed-in user without a password gets a "send me a link to set a password" button that calls
  `POST /api/auth/password-reset` with their own e-mail; the register page uses `purpose=login` (no separate
  register purpose).
- **Tests**: `tests/common/oidc.rs` (mock provider: discovery, JWKS, token endpoint checking the client secret and
  PKCE, ID tokens signed with the fictitious test keys `tests/fixtures/oidc/key_{a,b}.pem`), `tests/common/google.rs`;
  `Google::with_jwks_refresh` shortens the 60 s refetch gate for the rotation test.
