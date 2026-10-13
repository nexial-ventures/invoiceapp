# Users, registration, login and sessions (4a)

Password accounts with server-side sessions; hosts and spaces are in [spaces.md](spaces.md). TOTP (#5, 4c): the
login code step, step-up codes and `user.mfaEnabled` in [mfa.md](mfa.md). Sign in with Google (#6, 4d), accounts
without a password, `user.hasPassword` / `googleLinked`: [google.md](google.md).

## Config
- `INVOICEAPP__REGISTRATION` (`true` / `false`, default `false`): public registration on the base host **and** creating
  spaces (`POST /api/spaces`; off → 403 `space_creation_disabled`, [spaces.md](spaces.md)). `true` requires
  SMTP to be configured (startup fails otherwise) — verification and reset e-mails go through the instance SMTP
  (2a), `From` = the configured sender.
- `INVOICEAPP__TRUST_FORWARDED` (default `false`): take the client IP for rate limits from the **last**
  `X-Forwarded-For` hop (behind a reverse proxy that appends to the header — earlier hops are client-controlled);
  otherwise the socket address. Behind a proxy it **must** be `true` and the app port must not be reachable
  directly (else every client shares the proxy's buckets, or a direct client forges the header). With `false`, the
  first request that carries `X-Forwarded-For` logs one warning (once per process).
- `INVOICEAPP__API_TOKEN` is removed (startup ignores it; `.env.example` and docs drop it).
- **Bootstrap = registration.** There is no CLI user / space creation; the first user registers like anyone else
  (enable registration, register, disable it if wanted — this also disables creating further spaces, including for
  you).

## Users
Table `users` (id, email unique lowercased + trimmed, display_name, password_hash argon2id — `NULL` for an account
created through Google that has not set one (4d) —, email_verified_at, disabled, created_at, google_sub (4d)). Password: 12–200 chars (no other rule). `displayName` 1–100.

## Registration (base host, only when enabled)
- `POST /api/auth/register { email, password, displayName }` → **202** always for a valid body (no account
  enumeration). New e-mail → user created unverified + a verification e-mail with
  `{baseUrl}/verify?token=…`. Existing e-mail → no change; a "you already have an account" e-mail with the reset
  link instead. 422: `email: required | invalid`, `password: too_short | too_long`, `displayName: required |
  too_long`. Disabled → 404.
- `POST /api/auth/verify { token }` → 204 (sets `email_verified_at`); invalid / expired / used → 422
  `token: invalid`. Tokens: 32 random bytes, stored sha256, single use, 24 h.
- `POST /api/auth/verify/resend { email }` → 202 always (rate limited).
- An unverified user can log in to the base host but cannot create a space, change the password, set up TOTP
  (`/api/account/mfa/setup`, `/enable`) or link Google (403 `email_unverified`), and cannot log in to a space host.
  Whoever registered an address they do not own must not leave credentials on the account its owner later proves.
- When an unverified account becomes verified (verify link, password reset, invitation accept, the Google claim),
  everything second-factor set up before is dropped in the same statement: TOTP secret, pending setup, last step,
  recovery codes, pending logins, a Google link (security review M1). Exception: an invitation accept whose step-up
  TOTP code was checked in the same transaction keeps the TOTP and recovery codes (the user just proved them; only
  the Google link goes) — a member of a space requiring TOTP must not lose it by joining.

## Login and sessions (base host and space hosts)
- `POST /api/auth/login { email, password }` → 204 + session cookie. On a space host the user must be a member of
  that space. Wrong e-mail, wrong password, not a member, disabled → the same 401 `{"code":"invalid_credentials"}`.
  Unverified on a space host → the same 401. Constant-time: an unknown e-mail still runs one argon2 verification.
- Cookie `invoice_session`: 32 random bytes base64url, **host-only** (no `Domain`), `HttpOnly`, `SameSite=Lax`,
  `Path=/`, `Secure` when `INVOICEAPP__PUBLIC_URL` is https. The DB stores sha256 of it (table `sessions`: user_id,
  space_id (null on the base host), created_at, last_seen_at, expires_at, user_agent ≤ 200). A session is valid only
  on the host it was created for (the base host or its space).
- Expiry: idle **14 days** (sliding; `last_seen_at` written at most once a minute), absolute **90 days**. Expired sessions,
  e-mail / link tokens, pending TOTP logins, Google flows / handoffs and invitations are deleted by an hourly
  background purge (`src/cleanup.rs`); every reader ignores an expired row before that.
- `POST /api/auth/logout` → 204, deletes the session, clears the cookie.
- `POST /api/auth/sessions/revoke-others` → 204, deletes all other sessions of the user (all hosts), the pending
  TOTP logins and **every API token of the user (all spaces)**.
- `GET /api/auth/me` → `{ user: { id, email, displayName, emailVerified }, space: { slug, name, role } | null }`
  (session or token; 401 without).
- Moving between spaces = a link to the other host; each host needs its own login (no handoff).

## Password
- `POST /api/account/password { currentPassword, newPassword }` (session only, verified user — else 403
  `email_unverified`) → 204; wrong current → 422 `currentPassword: invalid`; revokes the user's other sessions and
  **all their API tokens** (all spaces).
- `POST /api/auth/password-reset { email }` (any known host) → 202 always; a known verified-or-not user gets
  `{baseUrl}/reset?token=…` (1 h, single use, sha256 stored).
- `POST /api/auth/password-reset/confirm { token, password }` → 204; sets the password, marks the e-mail verified
  (the link proved it), revokes **all** sessions, pending logins and **API tokens** (all spaces) — a reset follows a
  suspected compromise, and a token minted by whoever had the password must die with it (security review M2).

## CSRF
Cookie-authenticated `POST` / `PUT` / `PATCH` / `DELETE` must carry an `Origin` equal to the request's own origin
(`INVOICEAPP__PUBLIC_URL` scheme + the request host + port); missing or different → 403 `{"code":"csrf"}`. Bearer
requests and the unauthenticated auth routes (`login`, `register`, `verify`, `password-reset*`) are checked the same
way when an `Origin` is present (a cross-site form post is refused).

## Rate limits (in memory, per instance)
429 `{"code":"rate_limited"}` with `Retry-After`:
- login per e-mail **and client IP** (IPv6 by its /64): a **progressive delay**, not a lock — 3 failures pass
  freely; after that each further attempt from that client must wait 1 s, 2 s, 4 s … (doubling per failure, capped
  at 30 s) since its last failure, counted over a sliding 15 min. An attempt inside the delay → 429 with
  `Retry-After` = the remaining wait; after it the attempt is checked as usual (a correct password signs in). A
  guesser only delays their own network: the owner signing in from elsewhere is not affected;
- login per e-mail from all clients: 100 failures / 15 min (hard) — the backstop against a guess spread over many
  networks. One network cannot reach it (the IP limit stops it at 20), so holding an account at 429 takes failures
  from at least five networks, sustained (accepted risk below);
- login per IP: 20 failures / 15 min (hard);
- TOTP / recovery codes: 5 failures / 15 min per user (hard, the **code bucket**, [mfa.md](mfa.md));
- register, verify/resend, password-reset: 5 / hour per IP and 3 / hour per e-mail.

Rate-limit keys: an IPv4 client by its address, an **IPv6 client by its /64** (one subscriber usually holds a
whole /64 and could rotate addresses through it); an IPv4-mapped IPv6 address counts as the IPv4 one.

Password hashing (argon2id, ~19 MiB and a core each) runs under a global cap: at most 2 × the available cores
(min 2) hashes / verifications at once; a request that cannot start one within ~2 s → **503 `{"code":"busy"}`**
(nothing consumed — a reset token stays valid, a login / own-password / invite-accept attempt is refunded from the
login buckets). An `X-Forwarded-For` hop with a port (`203.0.113.7:51234`, `[2001:db8::1]:443`) is read as its
address; an unparsable last hop falls back to the socket address and logs one warning.

Nothing secret (password, cookie, token, reset / verify token) is ever logged.

## UI
- **Base host:** `/register` (when enabled), `/verify`, `/login`, `/forgot`, `/reset`, `/` = "Moje spaces" (list
  with links to each host, "Nový space" form with slug preview `{slug}.{base host}`), `/account`.
- **Space host:** `/login` (e-mail + password, link "Zapomenuté heslo"), then the existing app. The token-paste login
  screen is removed; the SPA uses the cookie (`credentials: "same-origin"`), never a stored token.
- 401 anywhere → `/login` with a return path; 403 `email_unverified` → a "verify your e-mail" screen with resend.
- E-mails (verify, already-registered, reset) in cs or en by the browser locale sent with the request
  (`locale` field, default cs); plain text templates in code.

## Tests
- Register → verify → login → create space → login on its host; disabled registration → 404; enumeration-safe 202.
- Login failures (wrong password, unknown e-mail, not a member, unverified, disabled) all 401 alike; the progressive delay per e-mail and client (growth, cap, success after the wait, the owner from another network and other e-mails unaffected), the per-e-mail ceiling and the hard per-IP limit → 429.
- Session expiry (idle + absolute, by adjusting the stored timestamps), logout, revoke-others, cookie of the base
  host refused on a space host and vice versa.
- CSRF: cookie POST without / with a foreign Origin → 403; Bearer unaffected.
- Password change and reset (sessions revoked, token single use / expired).
- E-mails through the mock SMTP; the verify / reset tokens read from the captured mail.

## Clarifications (as implemented)
- Config: `INVOICEAPP__PUBLIC_URL` = `http(s)://host[:port]` with an optional trailing `/` (no path, user info or IP
  literal; host lowercased). `INVOICEAPP__REGISTRATION` / `INVOICEAPP__TRUST_FORWARDED` accept `true` / `false` / `1` /
  `0`, empty = `false`, anything else refuses the start. A leftover `INVOICEAPP__API_TOKEN` is ignored.
- E-mails are lowercased + trimmed everywhere (register, login, resend, reset). An e-mail over 200 chars →
  `email: invalid`. Passwords are never trimmed; length counts characters.
- Register: validation (422) runs before the rate limit, so invalid bodies do not count. A known e-mail gets the
  "already registered" mail with a 1 h reset link (`{baseUrl}/reset?token=…`) — not when that account is disabled
  (no mail, like a reset request). Both paths hash the password. Account e-mails (verify, already registered,
  reset) are sent in a background task: every branch answers 202 at once, so the timing does not show whether an
  account exists; a failed send is logged at `error`. On a space host → 404.
- `verify` and `verify/resend` are base-host routes that also work while registration is off (pending users can
  finish). Resend mails only unverified, non-disabled users. Every e-mail body accepts `locale` (`cs` default,
  `en`); the links always point to the base host.
- Login: one attempt is **reserved** in the e-mail-per-client, the e-mail and the IP bucket at once (check + record under one lock,
  before the argon2 verification), so a parallel burst cannot pass the limit (12 parallel wrong logins → 3 × 401,
  9 × 429: the fourth would have to wait 1 s after the third); a successful login gives its reservation back, so
  only failures count — and only its own: the failures before it stay (a correct password does not reset the delay,
  which would otherwise let anyone who knows the password wipe the failed TOTP codes after it). One argon2 verification always
  runs (a dummy hash for unknown or empty e-mails).
- Sessions: the cookie has `Max-Age` = 90 days; the DB keeps sha256 (hex) of the cookie value and `user_agent`
  cut to 200 characters. Expired sessions are deleted when presented. A session row stores the space of its host
  (`NULL` = base host) and is refused on any other host (401).
- `logout` with a token → 204 (nothing to delete, the cookie is cleared anyway). `sessions/revoke-others` and
  `account/password` are **session only** (a token → 403 `forbidden`); revoke-others keeps the current session.
- Password change reports `currentPassword: invalid` and `newPassword: too_short | too_long` together; it keeps the
  current session and deletes the user's others. Reset confirm validates the password before consuming the token
  (a too-short password does not burn it), then consumes the token (single-use update, row-locked) **before**
  hashing — argon2 runs only for a live token; request and confirm work on any known host; disabled users get no
  mail. Confirm is limited to 20 requests / 15 min per IP (every request counts).
- The own-password checks (`currentPassword` of a password change, `password` of `DELETE /api/space`, MFA setup /
  disable / regenerate, Google unlink) count a failure in the user's login buckets (the progressive delay of the
  user's e-mail from the requesting client, shared with login, and the per-e-mail ceiling — not the IP limit);
  inside the delay or over the ceiling → 429. TOTP / recovery codes (login step, step-up, enable)
  count in the separate, hard code bucket (5 / 15 min per user, plus the IP bucket where the login has one):
  reaching a code check takes the password or a session, so a stranger cannot lock it, and a 6-digit code must not
  get the far higher rate the delay allows (~2 / min).
- Verify / reset tokens of one kind are refused by the other route (`token: invalid`). Several live tokens may
  exist (each resend / request issues a new one); each is single use.
- CSRF: `Origin` is compared case-insensitively with `{scheme of INVOICEAPP__PUBLIC_URL}://{request host incl. port}`;
  `Origin: null` is foreign. GET / HEAD / OPTIONS are never checked. The public auth routes (`login`, `register`,
  `verify*`, `password-reset*`) and Bearer requests are checked only when `Origin` is sent.
- Rate limits: `register`, `verify/resend` and `password-reset` share one per-IP bucket (5 / h) and one per-e-mail
  bucket (3 / h); every request counts there (not only failures). `Retry-After` is in whole seconds (≥ 1). Without
  `INVOICEAPP__TRUST_FORWARDED` the socket address is the IP; with it the **last** `X-Forwarded-For` hop (the one the
  trusted proxy appended — earlier hops are client-controlled; the proxy must append, not pass through).
  `Retry-After` is rounded **up** (a client that waits it is not refused again). The limiter is in memory (single
  instance) and bounded without dropping state that still counts: keys (cut to 320 bytes) are swept in insertion
  order a few per request, removing only **idle** keys (no event left in their window or delay); above 100 000
  keys every idle key is dropped at once, and only if all are still active the oldest are evicted (last resort,
  one warning per process).
- Each authenticated request resolves the cookie / token, its user and the membership in one joined query (plus
  the host's slug lookup); `last_seen_at` / `last_used_at` are written at most once a minute.
- Request logs record method + path only (never the query string: the SPA's `/verify?token=…` / `/reset?token=…`
  pages); no password, cookie, token or verify / reset token is logged anywhere.
- New error codes: 401 `invalid_credentials`, 403 `forbidden`, `email_unverified`, `csrf`, 429 `rate_limited`,
  503 `busy` (password hashing saturated). Later: 403 `space_limit` (space cap), 503 `email_disabled` (kill switch,
  [email.md](email.md)).
- `INVOICEAPP__EMAIL_DISABLED=true`: verify, "already registered" and reset e-mails are skipped and logged at `warn`
  (the routes still answer 202, nothing else changes).
- `GET /api/auth/check` (the old token check) is removed; the SPA uses `GET /api/auth/me`.
- **4d** ([google.md](google.md)): `password_hash` is nullable. A password login of an account without a password
  runs the dummy verification and answers 401 `invalid_credentials` (as a wrong password). Sessions record
  `login_method` (`password` | `google`; `created_at` is the login time). `POST /api/account/password` without
  `currentPassword` sets a first password within 10 min of a Google login (else `currentPassword: required`); every
  own-password check (password change aside) answers 422 `password: not_set` for an account without one, and
  counts nothing in the login bucket. `GET /api/context` gains `google`; `me.user` gains `hasPassword`,
  `googleLinked`.
- **Accepted risks** (decided with the product owner):
  - *Login delay and ceiling* (#40): the delay is per e-mail and client, so a guesser only slows themselves
    (~2 attempts / min per e-mail and network once delayed, and 20 / 15 min per IP). Failures from five or more
    networks, sustained, can keep an e-mail over its 100 / 15 min ceiling, so the owner's logins and own-password
    checks answer 429 until the window frees up (existing sessions keep working); without the ceiling such an
    attacker could guess without bound.
  - *No instance-wide e-mail total*: register, verify/resend and password-reset are limited per IP and per
    address; every e-mail sent on behalf of a space (documents, test, invitations) is limited per space and per
    user in the database (100 / day, 20 / h each, [spaces.md](spaces.md#limits)), and a user owns at most
    `INVOICEAPP__MAX_SPACES_PER_USER` (3) spaces they created. Many clients and accounts together can still make the
    instance send more mail than any one of these bounds. Mitigations: `INVOICEAPP__EMAIL_DISABLED=true` (kill switch:
    space e-mails → 503 `email_disabled`, account e-mails skipped and logged — registration then cannot be
    completed), open registration only to vetted users at first, or keep `INVOICEAPP__REGISTRATION=false`.
  - *PDF template isolation is mdcast's*: a space's design (typst templates uploaded by its admins,
    [design.md](design.md)) is rendered by the mdcast service; the app does not sandbox it. The operator must run
    a sandboxed mdcast (no file system or network access beyond the request, resource limits) — otherwise a
    template could read or reach what the mdcast process can.
