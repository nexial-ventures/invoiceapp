# Users, registration, login and sessions (4a)

Password accounts with server-side sessions; hosts and spaces are in [spaces.md](spaces.md). TOTP (#5, 4c): the
login code step, step-up codes and `user.mfaEnabled` in [mfa.md](mfa.md). Sign in with Google (#6, 4d), accounts
without a password, `user.hasPassword` / `googleLinked`: [google.md](google.md).

## Config
- `INVOICE__REGISTRATION` (`true` / `false`, default `false`): public registration on the base host. `true` requires
  SMTP to be configured (startup fails otherwise) — verification and reset e-mails go through the instance SMTP
  (2a), `From` = the configured sender.
- `INVOICE__TRUST_FORWARDED` (default `false`): take the client IP for rate limits from the first
  `X-Forwarded-For` hop (behind the reverse proxy); otherwise the socket address.
- `INVOICE__API_TOKEN` is removed (startup ignores it; `.env.example` and docs drop it).
- **Bootstrap = registration.** There is no CLI user / space creation; the first user registers like anyone else
  (enable registration, register, disable it if wanted).

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
- An unverified user can log in to the base host but cannot create a space (403 `email_unverified`) and cannot log
  in to a space host.

## Login and sessions (base host and space hosts)
- `POST /api/auth/login { email, password }` → 204 + session cookie. On a space host the user must be a member of
  that space. Wrong e-mail, wrong password, not a member, disabled → the same 401 `{"code":"invalid_credentials"}`.
  Unverified on a space host → the same 401. Constant-time: an unknown e-mail still runs one argon2 verification.
- Cookie `invoice_session`: 32 random bytes base64url, **host-only** (no `Domain`), `HttpOnly`, `SameSite=Lax`,
  `Path=/`, `Secure` when `INVOICE__PUBLIC_URL` is https. The DB stores sha256 of it (table `sessions`: user_id,
  space_id (null on the base host), created_at, last_seen_at, expires_at, user_agent ≤ 200). A session is valid only
  on the host it was created for (the base host or its space).
- Expiry: idle **14 days** (sliding; `last_seen_at` written at most once a minute), absolute **90 days**.
- `POST /api/auth/logout` → 204, deletes the session, clears the cookie.
- `POST /api/auth/sessions/revoke-others` → 204, deletes all other sessions of the user (all hosts).
- `GET /api/auth/me` → `{ user: { id, email, displayName, emailVerified }, space: { slug, name, role } | null }`
  (session or token; 401 without).
- Moving between spaces = a link to the other host; each host needs its own login (no handoff).

## Password
- `POST /api/account/password { currentPassword, newPassword }` (session only) → 204; wrong current → 422
  `currentPassword: invalid`; revokes the user's other sessions.
- `POST /api/auth/password-reset { email }` (any known host) → 202 always; a known verified-or-not user gets
  `{baseUrl}/reset?token=…` (1 h, single use, sha256 stored).
- `POST /api/auth/password-reset/confirm { token, password }` → 204; sets the password, marks the e-mail verified
  (the link proved it), revokes **all** sessions. API tokens stay valid.

## CSRF
Cookie-authenticated `POST` / `PUT` / `PATCH` / `DELETE` must carry an `Origin` equal to the request's own origin
(`INVOICE__PUBLIC_URL` scheme + the request host + port); missing or different → 403 `{"code":"csrf"}`. Bearer
requests and the unauthenticated auth routes (`login`, `register`, `verify`, `password-reset*`) are checked the same
way when an `Origin` is present (a cross-site form post is refused).

## Rate limits (in memory, per instance)
429 `{"code":"rate_limited"}` with `Retry-After`:
- login: 5 failures / 15 min per e-mail, 20 / 15 min per IP;
- register, verify/resend, password-reset: 5 / hour per IP and 3 / hour per e-mail.

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
- Login failures (wrong password, unknown e-mail, not a member, unverified, disabled) all 401 alike; rate limit 429.
- Session expiry (idle + absolute, by adjusting the stored timestamps), logout, revoke-others, cookie of the base
  host refused on a space host and vice versa.
- CSRF: cookie POST without / with a foreign Origin → 403; Bearer unaffected.
- Password change and reset (sessions revoked, token single use / expired).
- E-mails through the mock SMTP; the verify / reset tokens read from the captured mail.

## Clarifications (as implemented)
- Config: `INVOICE__PUBLIC_URL` = `http(s)://host[:port]` with an optional trailing `/` (no path, user info or IP
  literal; host lowercased). `INVOICE__REGISTRATION` / `INVOICE__TRUST_FORWARDED` accept `true` / `false` / `1` /
  `0`, empty = `false`, anything else refuses the start. A leftover `INVOICE__API_TOKEN` is ignored.
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
- Login: one attempt is **reserved** in the e-mail and the IP bucket at once (check + record under one lock,
  before the argon2 verification), so a parallel burst cannot pass the limit (12 parallel wrong logins → 5 × 401,
  7 × 429); a successful login gives its reservation back, so only failures count. One argon2 verification always
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
- The own-password checks (`currentPassword` of a password change, `password` of `DELETE /api/space`) count a
  failure in the user's login bucket (5 / 15 min, shared with login, keyed by the user's e-mail); used up → 429.
- Verify / reset tokens of one kind are refused by the other route (`token: invalid`). Several live tokens may
  exist (each resend / request issues a new one); each is single use.
- CSRF: `Origin` is compared case-insensitively with `{scheme of INVOICE__PUBLIC_URL}://{request host incl. port}`;
  `Origin: null` is foreign. GET / HEAD / OPTIONS are never checked. The public auth routes (`login`, `register`,
  `verify*`, `password-reset*`) and Bearer requests are checked only when `Origin` is sent.
- Rate limits: `register`, `verify/resend` and `password-reset` share one per-IP bucket (5 / h) and one per-e-mail
  bucket (3 / h); every request counts there (not only failures). `Retry-After` is in whole seconds (≥ 1). Without
  `INVOICE__TRUST_FORWARDED` the socket address is the IP; with it the **last** `X-Forwarded-For` hop (the one the
  trusted proxy appended — earlier hops are client-controlled; the proxy must append, not pass through). The
  limiter is bounded: keys (cut to 320 bytes) are swept in insertion order a few per request once their newest
  event is older than an hour, and above 100 000 keys the oldest-inserted are dropped.
- Each authenticated request resolves the cookie / token, its user and the membership in one joined query (plus
  the host's slug lookup); `last_seen_at` / `last_used_at` are written at most once a minute.
- Request logs record method + path only (never the query string: the SPA's `/verify?token=…` / `/reset?token=…`
  pages); no password, cookie, token or verify / reset token is logged anywhere.
- New error codes: 401 `invalid_credentials`, 403 `forbidden`, `email_unverified`, `csrf`, 429 `rate_limited`.
- `GET /api/auth/check` (the old token check) is removed; the SPA uses `GET /api/auth/me`.
- **4d** ([google.md](google.md)): `password_hash` is nullable. A password login of an account without a password
  runs the dummy verification and answers 401 `invalid_credentials` (as a wrong password). Sessions record
  `login_method` (`password` | `google`; `created_at` is the login time). `POST /api/account/password` without
  `currentPassword` sets a first password within 10 min of a Google login (else `currentPassword: required`); every
  own-password check (password change aside) answers 422 `password: not_set` for an account without one, and
  counts nothing in the login bucket. `GET /api/context` gains `google`; `me.user` gains `hasPassword`,
  `googleLinked`.
