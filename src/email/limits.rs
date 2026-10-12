//! Outbound e-mail limits (`docs/api/spaces.md`, "Limits"): every e-mail
//! sent on behalf of a space (document e-mails, the settings test e-mail,
//! invitations) is counted in `email_sends` per space and per sending user
//! within that space, over a rolling hour and day; the bounds are the
//! space's hidden `space_limits` row. Account e-mails (verify, reset) are
//! per-address limited elsewhere and not counted here.
//!
//! [`reserve`] checks and records in one transaction under the space's lock
//! (the user's count is a subset of the space's rows), so parallel sends
//! cannot both pass on the last free slot. A reservation is not given back
//! when the SMTP exchange fails: the attempt reached the server.

use chrono::{DateTime, TimeDelta, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, Set, Statement, TransactionTrait};
use uuid::Uuid;

use super::entity::email_send;
use crate::db::advisory_xact_lock;
use crate::error::AppError;
use crate::space::SpaceId;
use crate::space::limits::{EmailLimits, limits};

/// Rows older than this are purged (`cleanup`); longer than the day window.
pub const KEEP: TimeDelta = TimeDelta::days(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Document,
    Test,
    Invite,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Test => "test",
            Self::Invite => "invite",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Space,
    User,
}

impl Scope {
    /// Rows of the window, `$1` = the space, `$2` = the user.
    fn filter(self) -> &'static str {
        match self {
            Self::Space => "space_id = $1",
            Self::User => "space_id = $1 AND user_id = $2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    scope: Scope,
    length: TimeDelta,
    limit: u32,
    used: u64,
}

fn hour() -> TimeDelta {
    TimeDelta::hours(1)
}

fn day() -> TimeDelta {
    TimeDelta::days(1)
}

/// The four windows with their use; [`Counts`] in the order of the query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counts {
    space_hour: u64,
    space_day: u64,
    user_hour: u64,
    user_day: u64,
}

fn windows(l: EmailLimits, c: Counts) -> [Window; 4] {
    let w = |scope, length, limit, used| Window {
        scope,
        length,
        limit,
        used,
    };
    [
        w(Scope::Space, hour(), l.space_per_hour, c.space_hour),
        w(Scope::Space, day(), l.space_per_day, c.space_day),
        w(Scope::User, hour(), l.user_per_hour, c.user_hour),
        w(Scope::User, day(), l.user_per_day, c.user_day),
    ]
}

/// The windows with no room for one more e-mail.
fn full(ws: &[Window]) -> impl Iterator<Item = &Window> {
    ws.iter().filter(|w| w.used >= u64::from(w.limit))
}

/// Whole seconds (≥ 1) until `freeing` — the send whose expiry makes room —
/// leaves the window; the whole window when there is none (limit 0).
fn retry_secs(freeing: Option<DateTime<Utc>>, length: TimeDelta, now: DateTime<Utc>) -> u64 {
    let wait = freeing.map_or(length, |at| at + length - now);
    let ms = u64::try_from(wait.num_milliseconds()).unwrap_or(0);
    ms.div_ceil(1000).max(1)
}

fn int(row: &sea_orm::QueryResult, col: &str) -> Result<u64, AppError> {
    Ok(u64::try_from(row.try_get::<i64>("", col)?).unwrap_or(0))
}

async fn counts(
    txn: &impl ConnectionTrait,
    space: SpaceId,
    user: Uuid,
    now: DateTime<Utc>,
) -> Result<Counts, AppError> {
    let row = txn
        .query_one(Statement::from_sql_and_values(
            txn.get_database_backend(),
            "SELECT count(*) FILTER (WHERE space_id = $1 AND created_at > $3) AS space_hour, \
                    count(*) FILTER (WHERE space_id = $1) AS space_day, \
                    count(*) FILTER (WHERE user_id = $2 AND created_at > $3) AS user_hour, \
                    count(*) FILTER (WHERE user_id = $2) AS user_day \
             FROM email_sends WHERE space_id = $1 AND created_at > $4",
            [
                space.into(),
                user.into(),
                (now - hour()).into(),
                (now - day()).into(),
            ],
        ))
        .await?;
    let Some(r) = row else {
        return Ok(Counts::default());
    };
    Ok(Counts {
        space_hour: int(&r, "space_hour")?,
        space_day: int(&r, "space_day")?,
        user_hour: int(&r, "user_hour")?,
        user_day: int(&r, "user_day")?,
    })
}

/// `Retry-After` of a full window: when its `used − limit + 1`-th oldest
/// send leaves it, one slot is free.
async fn retry_after(
    txn: &impl ConnectionTrait,
    space: SpaceId,
    user: Uuid,
    w: &Window,
    now: DateTime<Utc>,
) -> Result<u64, AppError> {
    let offset = i64::try_from(w.used - u64::from(w.limit)).unwrap_or(i64::MAX);
    let row = txn
        .query_one(Statement::from_sql_and_values(
            txn.get_database_backend(),
            format!(
                "SELECT created_at FROM email_sends WHERE {} AND created_at > $3 \
                 ORDER BY created_at OFFSET $4 LIMIT 1",
                w.scope.filter()
            ),
            [
                space.into(),
                user.into(),
                (now - w.length).into(),
                offset.into(),
            ],
        ))
        .await?;
    let freeing = match row {
        Some(r) => Some(r.try_get::<DateTime<Utc>>("", "created_at")?),
        None => None,
    };
    Ok(retry_secs(freeing, w.length, now))
}

/// Count one e-mail of `user` on behalf of `space` (the id of its row, for
/// [`release`]), or refuse it with 429 `rate_limited` + `Retry-After`
/// (nothing recorded) when the space, or the user in it, has used up an
/// hour or a day.
pub async fn reserve(
    db: &DatabaseConnection,
    space: SpaceId,
    user: Uuid,
    kind: Kind,
) -> Result<i64, AppError> {
    let txn = db.begin().await?;
    advisory_xact_lock(&txn, format!("email-space:{space}")).await?;
    let now = Utc::now();
    let ws = windows(
        limits(&txn, space).await?.email,
        counts(&txn, space, user, now).await?,
    );
    let mut wait = 0;
    for w in full(&ws) {
        wait = wait.max(retry_after(&txn, space, user, w, now).await?);
    }
    if wait > 0 {
        return Err(AppError::RateLimited(wait));
    }
    let id = email_send::Entity::insert(email_send::ActiveModel {
        space_id: Set(Some(space.uuid())),
        user_id: Set(user),
        kind: Set(kind.as_str().to_string()),
        created_at: Set(now.into()),
        ..Default::default()
    })
    .exec(&txn)
    .await?
    .last_insert_id;
    txn.commit().await?;
    Ok(id)
}

/// Give back a [`reserve`]d send that was not attempted after all (the
/// request was refused after the reservation).
pub async fn release(db: &DatabaseConnection, id: i64) -> Result<(), AppError> {
    email_send::Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(space_hour: u64, space_day: u64, user_hour: u64, user_day: u64) -> Counts {
        Counts {
            space_hour,
            space_day,
            user_hour,
            user_day,
        }
    }

    fn full_of(c: Counts) -> Vec<(Scope, i64)> {
        full(&windows(EmailLimits::default(), c))
            .map(|w| (w.scope, w.length.num_hours()))
            .collect()
    }

    #[test]
    fn a_window_is_full_at_its_limit() {
        assert!(full_of(counts(19, 99, 19, 99)).is_empty());
        assert_eq!(full_of(counts(20, 20, 0, 0)), [(Scope::Space, 1)]);
        assert_eq!(full_of(counts(5, 100, 5, 5)), [(Scope::Space, 24)]);
        assert_eq!(full_of(counts(0, 0, 20, 20)), [(Scope::User, 1)]);
        assert_eq!(
            full_of(counts(20, 100, 20, 100)),
            [
                (Scope::Space, 1),
                (Scope::Space, 24),
                (Scope::User, 1),
                (Scope::User, 24)
            ]
        );
    }

    #[test]
    fn a_zero_limit_is_always_full() {
        let l = EmailLimits {
            space_per_hour: 0,
            ..EmailLimits::default()
        };
        assert_eq!(full(&windows(l, Counts::default())).count(), 1);
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        let now = Utc::now();
        let sent = now - TimeDelta::minutes(59) - TimeDelta::milliseconds(500);
        assert_eq!(retry_secs(Some(sent), hour(), now), 60);
        assert_eq!(retry_secs(Some(now - hour()), hour(), now), 1, "at least 1");
        assert_eq!(retry_secs(None, day(), now), 86_400);
    }

    #[test]
    fn kinds() {
        assert_eq!(Kind::Document.as_str(), "document");
        assert_eq!(Kind::Test.as_str(), "test");
        assert_eq!(Kind::Invite.as_str(), "invite");
        assert!(KEEP > day());
    }
}
