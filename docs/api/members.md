# Members and invitations (4b)

Membership management for a space (#8); spaces, roles and tokens are in [spaces.md](spaces.md), accounts and
sessions in [auth.md](auth.md). All routes are on a **space host**.

## Who may do what
- **admin and owner** manage members and invitations; everyone else gets 403 `forbidden`.
- An **admin** may invite with, and assign, the roles `accountant`, `member`, `admin`; may change or remove members
  whose current role is not `owner`. Only an **owner** may grant `owner` or change / remove an owner.
- The **last owner** cannot be demoted, removed or leave (422 `role: last_owner` / 409 `{"code":"last_owner"}` —
  see routes).
- Any member may **leave** the space (except the last owner).
- Member management needs a **session or a token** with an effective role ≥ admin; the role ceiling of tokens
  applies as everywhere.

## Members
- `GET /api/members` (admin+) → `[ { userId, email, displayName, role, joinedAt, isSelf } ]`, by role (owner,
  admin, member, accountant) then e-mail.
- `PUT /api/members/{userId} { role }` (admin+) → the item. 422 `role: invalid | too_high` (granting above what the
  caller may grant), `role: last_owner` (demoting the last owner). Changing an owner as an admin, or an unknown /
  foreign user → 404 for unknown, 403 `forbidden` for an owner the caller may not touch. Changing one's own role is
  allowed under the same rules (an owner may step down while another owner exists).
- `DELETE /api/members/{userId}` (admin+) → 204. Self → use leave. Removing the last owner → 409 `last_owner`.
- `POST /api/space/leave` (any role, session only) → 204; last owner → 409 `last_owner`. The cookie is cleared.
- **Removal / leave revokes access at once:** the member's sessions on this space host and their API tokens in this
  space are deleted in the same transaction. A **role change** keeps sessions; tokens stay but are capped by the
  new role (4a rule). Documents the member created stay.

## Invitations
Table `space_invites` (id, space_id, email lowercased, role, token sha256, invited_by user, created_at,
expires_at; unique (space_id, email)).

- `GET /api/invites` (admin+) → pending, unexpired invitations `[ { id, email, role, invitedBy: { email,
  displayName }, createdAt, expiresAt } ]`, newest first. Expired rows are not listed (and are purged lazily).
- `POST /api/invites { email, role, locale? }` (admin+) → `201 { …item, emailSent, url? }`:
  - **SMTP configured** → the link `{space url}/invite?token=…` is sent **only by e-mail** (cs / en by `locale`,
    default cs; subject and text name the space and the inviter; a spawned task, failures logged) and the response
    carries `emailSent: true` and **no `url`** — accepting then proves the invitee owns the address (security
    review H1).
  - **No SMTP** (a closed instance) → the invitation is created, the response carries `emailSent: false` and the
    `url`, shown to the inviter once (copy button) to pass on.
  - Token: 32 random bytes base64url, stored sha256, **valid 7 days**, single use.
  - 422: `email: required | invalid | already_member`, `role: required | invalid | too_high`.
  - A pending invitation for the same e-mail is **replaced** (new role, new token, new expiry; the old link stops
    working).
- `POST /api/invites/{id}/resend` (admin+) → `200 { …item, emailSent, url? }`: new token + new 7-day expiry, e-mail
  again; `url` only without SMTP (as above).
- `DELETE /api/invites/{id}` (admin+) → 204 (revoke). Unknown / foreign → 404.

## Accepting (space host, no auth)
- `GET /api/invites/accept?token=…` → `200 { space: { slug, name }, email, role, accountExists: boolean }`;
  invalid / expired / used → 404 `not_found`. (Rate limited per IP like the other token routes.)
- `POST /api/invites/accept { token, password, displayName? }` → **204 + session cookie** on this host:
  - `accountExists` true → `password` must be that account's password (failures count in the login bucket, 401
    `invalid_credentials` / 429); `displayName` ignored. A disabled account → 401 `invalid_credentials`.
  - `accountExists` false → creates the user with that e-mail (**verified** — the link proved it), `displayName`
    required (1–100), `password` 12–200. Works even when `INVOICE__REGISTRATION` is false.
  - Creates the membership with the invited role, deletes the invitation, creates the session — one transaction.
    Already a member meanwhile → the invitation is consumed, the role is not changed, the session is created.
  - Invalid / expired / used token → 422 `token: invalid`.
  - The CSRF Origin rule applies as for the other unauthenticated auth routes.
- The invitation is bound to its e-mail: the accepting account is always the one with that e-mail; there is no
  "accept with another account".

## UI (space host)
- Settings → **Členové** tab (admin+): members table (role select per row within the caller's rights, remove with
  confirm, "(vy)" marker), "Pozvat" form (e-mail, role) → with SMTP "Odkaz byl odeslán e-mailem na {email}" (no
  link, no copy field); without SMTP the link with copy + "e-mail neodeslán", pending invitations table (resend,
  revoke).
- Account page: **"Opustit space"** with confirm (hidden for the last owner, who sees why).
- `/invite?token=…` page (no login): space name + role; existing account → e-mail (read-only) + password →
  "Přijmout pozvánku"; new account → name + password (≥ 12) → "Vytvořit účet a přijmout". Success → the app home.
  Invalid link → a readable message with a link to the base host.

## Tests
- Integration: invite → accept with an existing account and with a new account (registration off), wrong
  password, expired / used / replaced token, already member; role rules (admin vs owner, too_high, last owner on
  demote / remove / leave); removal and leave delete the member's sessions and tokens in that space only (other
  spaces untouched); role change caps tokens; with the mock SMTP the link only in the mail (no `url`), without SMTP
  `emailSent: false` + `url`;
  isolation (another space's invite id / member id → 404).
- Frontend: members tab (rights-based controls), invite flow with link copy, invite accept page both branches,
  leave.

## Clarifications (as implemented)
- **Minimum roles**: `/api/members*` and `/api/invites` (list, create, resend, revoke) need an effective role ≥ admin
  (`Manage` extractor: session, or a token capped by its role); `POST /api/space/leave` any role, **session only** (a
  token → 403 `forbidden`), and as a cookie mutation it needs the request's own `Origin` (403 `csrf`).
- Role rules are one pure function set (`src/members/rules.rs`): below admin → `forbidden`; touching a member (or a
  pending invitation) whose role is `owner` as a non-owner → `forbidden`; a new role above the caller's effective
  role → `too_high`; taking the last owner's `owner` away → `last_owner`.
- `PUT /api/members/{userId}` order: `role: required | invalid` (422) → unknown / foreign user or a non-UUID id (404)
  → 403 `forbidden` → 422 `role: too_high` → 422 `role: last_owner`. Response = the list item (with `isSelf`).
  Member changes lock the space row (`FOR NO KEY UPDATE`, not blocking inserts that reference the space) and
  count the owners under that lock — only owners whose account is **not disabled** count (a disabled owner cannot
  administer the space, so the last enabled owner cannot step down or leave), so two owners demoting each other at once leave exactly one owner; the
  caller's role is re-read under the lock (a caller demoted meanwhile acts with the lower role).
- `DELETE /api/members/{userId}`: the caller's own id → 403 `forbidden` (use leave); then 404, 403, 409
  `last_owner` as above.
- Removal / leave delete, in one transaction, the membership, the user's sessions whose space is this one (the
  base host and other spaces untouched), the user's API tokens in this space and the **pending invitations they
  sent** in this space (a kept link must not let a removed admin back in). A role change deletes the member's
  pending invitations whose role the new role may no longer grant (→ member / accountant: all of them; → admin:
  their owner invitations). Leave answers 204 with a `Set-Cookie` that clears `invoice_session`.
- `GET /api/members`: `joinedAt` = when the membership was created (RFC 3339); disabled users are listed (they are
  still members, but cannot sign in).
- **Invitations**: `email` trimmed + lowercased, same rule as registration (`required | invalid`, ≤ 200 chars);
  `already_member` when the address belongs to a user with a membership in this space; `role` `required | invalid
  | too_high` (above the caller's effective role) — reported together. An admin cannot replace, resend or revoke a
  pending **owner** invitation (403 `forbidden`, the owner-member rule applied to invitations).
- Replace, resend and revoke are each **one conditional statement** (no check-then-write race with a concurrent
  swap to an owner invitation): for a non-owner caller the upsert does not overwrite a live owner invitation, and
  the `UPDATE` / `DELETE` carry `role <> 'owner'`; nothing written → 403 when the invitation is live (an owner
  invitation), else 404. The item is built from the statement's `RETURNING` joined with the inviter.
- Rate limits on invitation e-mails (so the instance SMTP is no relay): every `POST /api/invites` and `resend`
  that passes validation counts in a per-user bucket (20 / h, the caller) and a per-space bucket (50 / h), reserved
  together (a refused request counts in neither) — whether SMTP is configured or not; used up → 429
  `rate_limited` + `Retry-After`.
- Replacing keeps the invitation `id` and sets a new role, inviter, token, `createdAt` and `expiresAt`. `resend`
  takes an optional body `{ locale? }`, keeps the role and `createdAt`, sets a new token, `expiresAt` and the
  inviter (= who resends, named in the e-mail); an expired invitation → 404 (invite again). `DELETE` of an expired
  row that was not purged yet → 204.
- Token: 32 random bytes base64url (43 chars), sha256 (hex) stored (`space_invites.token_hash`, unique);
  `expiresAt` = creation + exactly 7 days (RFC 3339). Expired rows are never served and are purged by
  `GET /api/invites` (that space's rows only) and by the server's hourly expired-row purge (all spaces).
- `POST` / `resend` answer `{ …item, emailSent, url? }`. With SMTP: `url` is **omitted** (the link exists only in
  the e-mail), `emailSent: true` = queued in a background task, a failure is logged at `error` (`false` only when
  the stored address cannot be used as a recipient — logged; resend after fixing it). Without SMTP: `emailSent:
  false`, nothing is attempted, `url` = `{space url}/invite?token=…`. Subject `{inviter} vás zve do {space}` / `{inviter} invites you to {space}` (inviter = the sender's
  display name, space = its name; control characters → spaces), the text names the role (cs: vlastník, správce,
  člen, účetní; en: the role id) and the 7-day validity.
- **Accept**: both routes exist only on a space host (base host → 404); a token of another space is unknown (404 /
  `token: invalid`). `GET` with a missing / empty token → 404; `accountExists` is also `true` for a disabled
  account (the `POST` then answers 401). `space` = `{ slug, name }` of the host's space.
- Rate limit: a per-IP bucket of 20 requests / 15 min shared by `GET` and `POST` accept, every request counts,
  checked first (429 `rate_limited` + `Retry-After`).
- `POST` order: rate limit → token (422 `token: invalid`; argon2 never runs for a dead token) → existing account:
  the login buckets (e-mail per client: progressive delay, e-mail: 100 / 15 min, IP 20 / 15 min, reserved before the check, refunded on success — the
  same buckets as `POST /api/auth/login`), wrong password or disabled → 401 `invalid_credentials` (the invitation
  stays usable); new account: `displayName` (trimmed, `required | too_long`) and `password` (`too_short | too_long`)
  reported together, then hashed. Then one transaction: the invitation is deleted (`DELETE … RETURNING`, so of two
  parallel accepts one wins and the other gets `token: invalid`), the new user is created, the user is marked
  verified (also an existing unverified account — the link proved the address), the membership is inserted
  (`ON CONFLICT DO NOTHING`: an existing membership keeps its role), the session of the request's own
  `invoice_session` cookie (if any — e.g. signed in on this host as another user) is deleted and the new session
  is created for this host.
- The transaction first takes the space's membership lock and, after consuming the invitation, re-checks that the
  **inviter** is still an enabled member allowed to grant the invited role (`can_invite`); if not, the consumed
  invitation is committed as deleted, nothing else is written, and the answer is 422 `token: invalid`. (`GET`
  does not run this check; invitations of removed / demoted inviters are already deleted by the member change.)
  An account registered for the address between the lookup and the commit → 409 `email_taken`, nothing committed.
- The CSRF rule is the one of the other public auth routes: a present, foreign `Origin` → 403 `csrf`.
- **Accepted risks** (decided with the product owner; kept as is):
  - *Account enumeration* and *address squatting* — **only on an instance without SMTP**: there the inviter gets
    the link, so an admin can invite an arbitrary address and read `accountExists` from `GET /api/invites/accept`,
    or accept an invitation for an address nobody has registered yet and so create a **verified** account for it
    with a password they chose (and a later Google sign-in with that address would link to it). With SMTP the link
    goes only to the invitee's mailbox, which closes both. An instance without SMTP cannot have registration on
    (the start is refused), so only admins of existing spaces — people the operator let in — can do this.
  - *No instance-wide e-mail limit*: the buckets above (and the per-IP / per-address ones of [auth.md](auth.md))
    bound one inviter, one space or one client, but many accounts and spaces together can still make the instance
    send a lot of mail. Mitigation: open registration only to vetted users at first, or keep
    `INVOICE__REGISTRATION=false` — it disables registration **and** space creation (403
    `space_creation_disabled`, also for invited accounts), so only people invited by an existing space's admins can
    enter, and only into those spaces.
- New error code: 409 `last_owner` (remove, leave). Migration `m20261017_000001_space_invites` (`space_invites`,
  FKs to `spaces` and `users` (`invited_by`) with `ON DELETE CASCADE`, unique `(space_id, email)` and
  `token_hash`).
- **4c** ([mfa.md](mfa.md)): `GET /api/members` items gain `mfaEnabled`; `GET /api/invites/accept` gains
  `requireMfa`; `POST /api/invites/accept` takes a step-up `code` (existing account with TOTP) and answers 403
  `mfa_required` in a space with the policy for an account without TOTP (a new account is created first:
  `detail: "account_created"`).
