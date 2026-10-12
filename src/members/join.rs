//! The core of accepting an invitation, shared by `POST /api/invites/accept`,
//! the Google finish (`purpose=invite`, `docs/api/google.md`) and the login
//! code step of a Google accept that waits for its TOTP code.

use sea_orm::DatabaseTransaction;
use uuid::Uuid;

use super::{invites, repo};
use crate::auth::mfa::verify::{self, Verifier};
use crate::auth::ratelimit::Limit;
use crate::auth::users;
use crate::error::AppError;
use crate::space::{Role, SpaceId};

/// Who accepts.
pub enum Joiner {
    Existing(Uuid),
    /// A new account (invitation e-mail, this name and password hash).
    New {
        name: String,
        hash: String,
    },
}

/// A step-up code to check (and spend) inside the accept.
pub struct Code<'a> {
    pub verifier: &'a Verifier,
    pub code: &'a str,
    pub buckets: &'a [(Limit, &'a str)],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Joined {
    /// The user (created when new) is a verified member now.
    Member(Uuid),
    /// No live invitation behind the token, or its inviter may no longer
    /// grant its role (then the invitation is used up). Nothing else was
    /// written: committing keeps the link dead.
    Refused,
    /// The code was wrong; the caller must roll back (the invitation stays
    /// usable).
    WrongCode,
}

/// Under the space's membership lock (a removal / demotion of the inviter
/// cannot interleave): use up the live invitation `token_hash` → the inviter
/// re-check → the code, last, so a refused accept never spends one → the
/// user (an address registered meanwhile → 409 `conflict`) → marked verified
/// (the link proved the address) → the membership (an existing one keeps its
/// role).
pub async fn join(
    txn: &DatabaseTransaction,
    space: SpaceId,
    token_hash: &str,
    who: Joiner,
    code: Option<Code<'_>>,
) -> Result<Joined, AppError> {
    repo::lock(txn, space).await?;
    let Some(invite) = invites::consume(txn, space, token_hash).await? else {
        return Ok(Joined::Refused);
    };
    let Some(role) = Role::parse(&invite.role) else {
        return Ok(Joined::Refused);
    };
    if !repo::may_grant(txn, space, invite.invited_by, role).await? {
        return Ok(Joined::Refused);
    }
    let mut code_proved = false;
    let user_id = match who {
        Joiner::Existing(id) => {
            if let Some(c) = code {
                if !verify::check(txn, c.verifier, id, c.code, c.buckets).await? {
                    return Ok(Joined::WrongCode);
                }
                code_proved = true;
            }
            id
        }
        Joiner::New { name, hash } => {
            users::create(txn, &invite.email, &name, Some(hash))
                .await?
                .ok_or_else(|| AppError::Conflict("users_email_key".into()))?
                .id
        }
    };
    // A TOTP just proved in this transaction is the user's own: keep it
    // (wiping it would leave a member without TOTP in a requiring space).
    if code_proved {
        users::mark_verified_keeping_mfa(txn, user_id).await?;
    } else {
        users::mark_verified(txn, user_id).await?;
    }
    repo::join(txn, space, user_id, &invite.role).await?;
    Ok(Joined::Member(user_id))
}
