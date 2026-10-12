# TOTP two-factor authentication (4c)

Optional TOTP second factor per **user** (global, like the password), and a per-**space** policy that requires it
(#5). Accounts and sessions: [auth.md](auth.md); spaces and roles: [spaces.md](spaces.md); members and invitations:
[members.md](members.md).

## Config
- `INVOICE__SECRET_KEY` (**required**): 32 bytes, base64. Encrypts TOTP secrets at rest (AES-256-GCM, random
  96-bit nonce per value) and keys the HMAC of recovery codes. Missing / wrong length → startup fails. Losing or
  changing it makes every enrolled TOTP unusable (users recover with a recovery code, then enrol again). No
  rotation in 4c.

## TOTP parameters
RFC 6238: SHA-1, 6 digits, 30 s step, 20-byte random secret (base32 in the URI). Accepted window: the current step
±1. **Replay protection:** the user's last accepted step is stored; a code for a step ≤ that one is refused.
`otpauth://totp/Invoice%20({base host}):{email}?secret=…&issuer=Invoice%20({base host})&digits=6&period=30`.

## Recovery codes
10 codes `xxxxx-xxxxx` (base32 lowercase, 50 bits), shown **once** at enable / regenerate; stored as
HMAC-SHA256(secret key, normalized code) — normalized = lowercase, without `-` and spaces. Each works once wherever
a TOTP code is accepted (login, step-up). Regenerating replaces all.

## Enrolment (`/api/account/mfa`, session only — a token gets 403; on the base host or any space host)
- `GET /api/account/mfa` → `{ enabled, recoveryCodesLeft, requiredBy: [ { slug, name } ] }` (`requiredBy` = the
  user's spaces with the policy on).
- `POST /api/account/mfa/setup { password }` → `{ secret, otpauthUri }`; stores the secret as *pending* (encrypted,
  10 min); a new setup replaces it. Already enabled → 409 `{"code":"mfa_enabled"}`. Wrong password → 422
  `password: invalid` (login bucket). An **unverified** user → 403 `email_unverified` (setup and enable; checked
  right after the session rule): a second factor is set up only on a proven address. Verifying an address later
  drops any TOTP, pending setup and recovery codes set up before ([auth.md](auth.md)).
- `POST /api/account/mfa/enable { code }` → `{ recoveryCodes: [10] }`; the pending secret becomes active. No / expired
  pending → 422 `code: expired`; wrong code → 422 `code: invalid`.
- `POST /api/account/mfa/disable { password, code }` → 204. Removes the secret and recovery codes. **Allowed even
  when a space requires MFA**: the user's sessions on those spaces' hosts are deleted and their tokens in those
  spaces stop working at once (401) — the UI lists `requiredBy` before confirming.
- `POST /api/account/mfa/recovery-codes { password, code }` → `{ recoveryCodes }`.
- Failed passwords count in the user's login bucket (progressive delay after 3 failures → 429 while it runs), failed
  codes in the user's **code bucket** (5 / 15 min, hard → 429), [auth.md](auth.md).

## Login
- `POST /api/auth/login { email, password }`:
  - user without TOTP → as before (204 + session), except on a space with the policy → **403
    `{"code":"mfa_required"}`** (only after a correct password; nothing is created).
  - user with TOTP → **200 `{ "mfa": "required" }`** + a pending cookie `invoice_mfa` (host-only, HttpOnly,
    SameSite=Lax, Secure as the session cookie; 32 random bytes, stored sha256 with user + host, 5 min, single use).
    No session yet.
- `POST /api/auth/login/mfa { code }` (`code` = TOTP or a recovery code) → 204 + session cookie, pending cookie
  cleared. No / expired pending → 401 `invalid_credentials`. Wrong code → 401 `{"code":"mfa_invalid"}`; failures
  count in the code bucket per e-mail and the login bucket per IP; the pending login dies after 5 failures.
- The `me` response gains `user.mfaEnabled`.

## Space policy
- `GET /api/space` gains `requireMfa: boolean`. `PUT /api/space { name?, requireMfa? }`: `requireMfa` is **owner
  only** (an admin sending it → 403), and turning it on requires the owner's own TOTP (422 `requireMfa:
  mfa_not_enabled`).
- Effect: **from the next login**. Existing sessions and API tokens of members without TOTP keep working until they
  end / are revoked; a new login on that host without TOTP → 403 `mfa_required` (above). A user who later
  **disables** TOTP loses sessions and tokens in such spaces at once (above).
- `GET /api/members` items gain `mfaEnabled` (admin+), so the owner sees who would be locked out.

## Invitations into a space with the policy
- `GET /api/invites/accept` gains `requireMfa`.
- Existing account without TOTP → `POST /api/invites/accept` → 403 `mfa_required`; the invitation stays valid. The
  user enables TOTP on the base host (`/account`) and opens the link again.
- New account → the account is created (verified, with the given name and password) but **no membership and no
  session**; 403 `{"code":"mfa_required","detail":"account_created"}`; the invitation stays valid. The user logs
  in on the base host, enables TOTP, then opens the link again (now an existing account).
- Existing account with TOTP → `code` is required in the accept body (step-up, below).

## Step-up (re-confirm with a code)
For a user with TOTP these also require `code` (TOTP or recovery) next to the password they already take:
`POST /api/account/password` (`currentPassword` + `code`), `DELETE /api/space` (`slug`, `password`, `code`),
`POST /api/invites/accept` (existing account), `POST /api/account/mfa/disable`, `/recovery-codes`.
`POST /api/tokens` takes `code` too (no password) for a user with TOTP. Missing → 422 `code: required`, wrong → 422
`code: invalid` (code bucket). Users without TOTP: unchanged.

## UI
- Login: after the password, a code step ("Kód z aplikace" + "Použít záchranný kód"); `mfa_required` → message
  with a link to `{baseUrl}/account`.
- Account → "Dvoufázové ověření": off → enable (password → QR rendered in the browser from `otpauthUri` + the
  manual key → code → the 10 recovery codes with copy / download as .txt and an "uložil(a) jsem je" confirmation);
  on → codes left, regenerate, disable (lists `requiredBy`).
- Code fields on the step-up forms (password change, space delete, token create, invite accept) when
  `user.mfaEnabled`.
- Settings → Space (owner): "Vyžadovat dvoufázové ověření" toggle; disabled with a hint when the owner has no TOTP;
  before enabling, lists members without TOTP (from `mfaEnabled`) who will be refused at their next login.
- Settings → Členové: an "MFA" column.
- Invite page: `requireMfa` hint; the two `mfa_required` outcomes explained with a link to the base host.

## Tests
- Unit: RFC 6238 test vectors (SHA-1), window ±1, replay, recovery code normalisation / HMAC, AES-GCM round trip and
  tamper detection, secret key parsing.
- Integration: enrol (setup → enable, expired pending, wrong code), login with TOTP / recovery code / wrong code /
  replayed code / expired pending / rate limit, disable (incl. loss of sessions + tokens in requiring spaces),
  regenerate, space policy (owner only, owner needs TOTP, next login refused, existing sessions keep working), invite
  accept outcomes, step-up on every listed route.

## Clarifications (as implemented)
- **Secret key**: `INVOICE__SECRET_KEY` = standard base64 (padded or not, surrounding whitespace trimmed) of exactly
  32 bytes; missing, empty, not base64 or another length → the start is refused with a message naming the variable.
  Two subkeys are derived from it (HMAC-SHA256 over fixed labels): one for AES-256-GCM, one for the recovery-code
  HMAC — "keyed by the secret key" in that sense. The sealed value is `nonce (12) ‖ ciphertext ‖ tag` (bytea) with
  the **user id as associated data** (a sealed secret copied to another user does not decrypt). A secret that does
  not decrypt (key changed, damaged row) is logged at `error` per attempt and a TOTP code is answered as **wrong
  without a reservation in the code bucket** (no 500, no lockout); recovery codes need no decryption and keep
  working **as long as the key is the same**. **Contract correction:** the recovery-code HMAC is keyed by
  `INVOICE__SECRET_KEY` too, so *changing* the key also invalidates every recovery code — the Config note "users
  recover with a recovery code" does not hold for a changed key; such users need TOTP cleared in the database
  (`UPDATE users SET totp_secret = NULL …`, `DELETE FROM recovery_codes …`). Treat the key as permanent.
- **Storage** (migration `m20261018_000001_mfa`): `users.totp_secret` (`NOT NULL` = TOTP on), `totp_pending` +
  `totp_pending_expires_at` (setup), `totp_last_step` (replay guard); `recovery_codes` (user, HMAC hex, unique per
  user; a used code is deleted); `mfa_logins` (sha256 of the `invoice_mfa` cookie, user, space — `NULL` = base host —,
  user agent, failures, expiry); `spaces.require_mfa` (default `false`). The per-request auth query loads only
  `totp_secret IS NOT NULL` (→ `mfaEnabled`); the secret, pending setup and last step are read on demand where a
  code is checked or enrolment runs.
- **Codes**: the input is normalized (lowercase, `-` and whitespace removed). 6 digits → TOTP, 10 base32 characters
  → recovery code, anything else → wrong. TOTP accepts steps now−1 … now+1, compared in constant time; the accepted
  step is stored with a conditional update (`totp_last_step < step`), so a code of a step ≤ the stored one — or the
  same code sent twice in parallel — is refused. A recovery code is spent by a single `DELETE`. `enable` stores the
  step of its code, so that code cannot be reused for a login.
- **Status / setup / enable / disable / regenerate**: all session only (a token → 403 `forbidden`), any known host.
  `setup` checks `mfa_enabled` (409) before the password; the secret is 20 bytes, `secret` = base32 without
  padding (32 chars); `otpauthUri` uses the base host **without the port** and percent-encodes the e-mail (`+` →
  `%2B`, `@` kept). `enable`: TOTP already on → 409 `mfa_enabled`; no / expired pending → 422 `code: expired`; empty
  `code` → `code: required`; a setup replaced or expired between the read and the write → `code: expired`.
  `disable` of a user without TOTP checks only the password and answers 204 — **nothing is deleted** (no session,
  token or pending login). `recovery-codes` of a user without TOTP → 409 `{"code":"mfa_disabled"}`. Regenerating
  replaces the codes in one transaction (one multi-row insert).
- **Disable** deletes, in one transaction, the secret, the pending setup, all recovery codes, the user's pending
  logins, the user's sessions on hosts of spaces with `require_mfa` (incl. the current one when called there) and
  the user's API tokens in those spaces. Other sessions (base host, spaces without the policy) stay.
- **Step-up order** (`password change`, `DELETE /api/space`, `POST /api/tokens`, `PUT /api/space` with
  `requireMfa`, `disable`, `recovery-codes`): the other checks first (field validation, password); `code: required`
  is reported together with them; the code is checked — and spent — **only when nothing else failed**, wrong →
  `code: invalid`. A wrong password with any code → only `password` / `currentPassword: invalid`. **A `code` sent by
  a user without TOTP is ignored everywhere** (no 422). Code failures count in the user's code bucket (e-mail,
  5 / 15 min, hard); the invite accept and the login code step also in the login IP bucket (20 / 15 min).
- **No code is spent by a failing request.** Invite accept and the login code step verify (and spend) the code
  **inside the operation's transaction, after its own checks**: accept = space lock → consume the invitation →
  inviter re-check (`may_grant`) → code → user / membership / session; a wrong code rolls everything back (the
  invitation stays usable, 422 `code: invalid`); a failed inviter re-check commits only the consumed invitation and
  never looks at the code. On the other step-up routes the code is the last check; only DB failures can follow it.
- **Password events end pending logins**: a password change, a reset confirm and `sessions/revoke-others` delete
  the user's `mfa_logins` rows together with the sessions (`session::delete_others`).
- **Login**: the `mfa_required` refusal and the `{ "mfa": "required" }` answer both come after a correct password
  and the usual checks (disabled, verified + member on a space host); the login reservation is refunded then. A user
  **with** TOTP gets the code step on every host, also where the policy is off. The pending login purges expired
  rows on creation (and the hourly expired-row purge does too). `POST /api/auth/login/mfa` is **one transaction that first locks the pending row** (`FOR
  UPDATE`), so attempts on one pending login run in turn: it re-checks the user (disabled, verified, still a member
  → 401 `invalid_credentials`), checks the code, then either counts the failure on the locked row and commits it
  (the 5th failure deletes the row; later requests → 401 `invalid_credentials`; 8 parallel wrong codes → exactly
  5 × `mfa_invalid` + 3 × `invalid_credentials`) or deletes the row and creates the session with the user agent of
  the first step (single use: a parallel request with another code → 401 `invalid_credentials`, its code unspent).
  Wrong codes also count in the code and IP buckets (429 when used up). The response sets the session cookie and a
  `Set-Cookie` clearing `invoice_mfa` (`Max-Age=0`). A pending cookie of another host → 401 `invalid_credentials`.
- **Space policy**: `PUT /api/space` takes `{ name?, requireMfa?, code? }` (all optional; an empty body → 200
  unchanged). `requireMfa` present (on **or off**): effective role below owner → 403 `forbidden`, a token → 403
  `forbidden` (session only) — both before any validation; the owner without TOTP → 422 `requireMfa:
  mfa_not_enabled` (reported with `name` errors); with TOTP a step-up `code` is required (`code: required |
  invalid`, code bucket). Changing only `name` is unchanged (admin+, token or session, no code). `requireMfa` is
  part of every `SpaceInfo` (also `GET /api/spaces` and `POST /api/spaces`).
- **Where the policy is checked**: login, invitation accept and **`POST /api/tokens`** (minting a credential counts
  as a new login: a caller without TOTP in a space with the policy → 403 `mfa_required`, checked first, also from
  an existing session or token). Existing sessions and tokens are never re-checked per request.
- **Invitations** (`POST /api/invites/accept`, space with the policy): rate limit → token → existing account:
  password (401 `invalid_credentials`) → no TOTP: a sent `code` is ignored, 403 `mfa_required` (invitation kept) →
  TOTP: missing `code` → 422 `code: required`, then the transaction above (wrong → 422 `code: invalid`, code
  bucket + login IP bucket). New account: `displayName` / `password` validation (422) → the user is created **verified**
  with that name and password in its own transaction (no membership, no session, invitation kept) → 403
  `{"code":"mfa_required","detail":"account_created"}`; an address registered meanwhile → 409 `email_taken`. Without
  the policy an existing account with TOTP still needs the `code` (step-up). `GET /api/invites/accept` returns
  `requireMfa`.
- `GET /api/account/mfa` `requiredBy` = the user's spaces with the policy, sorted by name (case-insensitive), then
  slug. `GET /api/members` items carry `mfaEnabled` (TOTP on), `GET /api/auth/me` carries `user.mfaEnabled`.
- New error codes: 403 `mfa_required` (`detail: "account_created"` only for the new-account accept), 401
  `mfa_invalid`, 409 `mfa_enabled`, 409 `mfa_disabled`. Nothing secret (TOTP secret, codes, recovery codes, `invoice_mfa` cookie, the
  secret key) is logged; the secret key is `[REDACTED]` in `Debug`.
