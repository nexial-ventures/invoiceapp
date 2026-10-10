# Sign in with Google (4d)

OpenID Connect login with Google (#6): authorization code + PKCE. Accounts and sessions: [auth.md](auth.md); hosts
and spaces: [spaces.md](spaces.md); invitations: [members.md](members.md); TOTP and step-up: [mfa.md](mfa.md).

## Config
- `INVOICE__GOOGLE__CLIENT_ID`, `INVOICE__GOOGLE__CLIENT_SECRET`: both set → Google enabled; neither → disabled
  (the routes 404, the buttons are hidden); only one → startup fails.
- `INVOICE__GOOGLE__ISSUER` (default `https://accounts.google.com`): discovery base, overridable for tests (a mock
  provider). Discovery and JWKS are fetched lazily and cached (JWKS refreshed on an unknown `kid`, at most once a
  minute).
- **Redirect URI** = `{INVOICE__PUBLIC_URL}/api/auth/google/callback` — the only one to register in Google Cloud
  Console. Google does not allow wildcard redirect URIs, so the callback is always on the **base host**.
- `GET /api/context` gains `google: boolean`.

## Flow
1. `GET /api/auth/google/start?purpose=login|link|invite&return=<path>&invite=<token>` on any known host (base or a
   space) → 302 to Google's authorization endpoint (`scope=openid email profile`, `response_type=code`,
   `code_challenge` S256, `nonce`, `state`, `prompt=select_account`). Stores a **flow row** (state sha256, the
   origin host = space id or null for base, purpose, PKCE verifier, nonce, `return` (a relative path, else `/`),
   invite token hash for `invite`, the session's user for `link`; 10 min, single use). `link` needs a session on that
   host (else 401). Rate limited per IP (20 / 15 min).
2. `GET /api/auth/google/callback?state&code` (base host) → validates the flow row (unknown / expired / used → error
   redirect), exchanges the code (client secret + PKCE verifier), verifies the **ID token**: signature via JWKS
   (RS256), `iss`, `aud` = client id, `exp`, `iat` not in the future (60 s skew), `nonce`, `email_verified` true.
   Then resolves the user (below) and creates a **handoff token** (32 random bytes, sha256 stored, 60 s, single use,
   bound to the flow's origin host, carrying user id, purpose, `return`) → 302 to
   `{origin host}/api/auth/google/finish?token=…`.
3. `GET /api/auth/google/finish?token` (the origin host) → consumes the handoff and completes the purpose; 302 to
   the SPA. The handoff is the only cross-host transfer; password login still never crosses hosts.

Errors at any step → 302 to `{origin host}/login?google_error=<code>` (or `/account?google_error=` for `link`,
`/invite?token=…&google_error=` for `invite`); codes: `cancelled` (user denied), `expired`, `invalid` (state, token,
signature, claims), `unverified_email`, `not_allowed` (no account and registration off and no invitation),
`disabled`, `not_member`, `mfa_required`, `already_linked`, `email_mismatch`, `unavailable` (Google unreachable).
Details are logged, never put in the URL.

## Resolving the user (callback)
1. A user with this Google `sub` → that user.
2. Else a user with the same e-mail (lowercased) → **linked automatically** (`sub` stored; Google's verified e-mail is
   the proof). Their password, TOTP and memberships are unchanged.
3. Else **new account** (verified, no password, display name = the Google `name`, else the e-mail's local part)
   when `INVOICE__REGISTRATION` is true, or when the purpose is `invite` and the invitation's e-mail equals the Google
   e-mail. Otherwise `not_allowed`.
- Disabled user → `disabled`. A `sub` already linked to another user (purpose `link`) → `already_linked`.

## Finishing (origin host)
- **login** on the base host → base session. On a space host → the user must be a member (`not_member`); the space
  policy and TOTP rules of [mfa.md](mfa.md) apply exactly as for a password login: a user **with TOTP** gets the
  pending `invoice_mfa` cookie and is sent to `/login?mfa=1` (the SPA shows the code step); a user without TOTP on a
  requiring space → `mfa_required`. Otherwise a session (cookie) → 302 to `return`.
- **invite**: the Google e-mail must equal the invitation e-mail (`email_mismatch`); then the 4b/4c accept rules
  (inviter re-check, MFA policy: no TOTP on a requiring space → `mfa_required` and the invitation stays; with TOTP →
  the code step, the membership is created only after the code) → membership + session → `return` (`/`).
- **link** (from the account page, session on that host): attaches the Google `sub` to the session's user — any
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
- `DELETE /api/account/google { password, code? }` → 204 unlink; needs a password set (`password: not_set`), so an
  account never loses its last login method.

## UI
- "Přihlásit přes Google" button on the base and space login pages and on the invite page (when `context.google`);
  "Registrovat přes Google" on the register page when registration is on.
- `google_error` codes shown as readable messages; `/login?mfa=1` opens the code step directly.
- Account: Google section (linked / not linked, "Propojit s Googlem", "Odpojit" with password + code); for an account
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
(filled during the 4d implementation)
