//! The hourly purge of expired rows: sessions past their idle or absolute
//! expiry, e-mail / link tokens, pending TOTP logins, Google flows and
//! handoffs, invitations, and the e-mail limit log past [`KEEP`]. Every
//! reader already ignores an expired row; this only keeps the tables (and
//! their indexes) from growing.

use std::time::Duration;

use anyhow::Context as _;
use chrono::Utc;
use sea_orm::sea_query::Condition;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::auth::entity::{mfa_login, session, user_token};
use crate::auth::google::entity::{google_flow, google_handoff};
use crate::auth::session::IDLE;
use crate::email::entity::email_send;
use crate::email::limits::KEEP;
use crate::members::entity::space_invite;

pub const INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Rows deleted by one [`purge`], per table.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    pub sessions: u64,
    pub user_tokens: u64,
    pub mfa_logins: u64,
    pub google_flows: u64,
    pub google_handoffs: u64,
    pub space_invites: u64,
    pub email_sends: u64,
}

async fn delete<E: EntityTrait>(db: &DatabaseConnection, cond: Condition) -> anyhow::Result<u64> {
    let table = E::default().table_name().to_string();
    Ok(E::delete_many()
        .filter(cond)
        .exec(db)
        .await
        .with_context(|| format!("purge expired {table}"))?
        .rows_affected)
}

/// Delete every expired row once.
pub async fn purge(db: &DatabaseConnection) -> anyhow::Result<Purged> {
    let now = Utc::now();
    Ok(Purged {
        sessions: delete::<session::Entity>(
            db,
            Condition::any()
                .add(session::Column::ExpiresAt.lte(now))
                .add(session::Column::LastSeenAt.lte(now - IDLE)),
        )
        .await?,
        user_tokens: delete::<user_token::Entity>(
            db,
            Condition::all().add(user_token::Column::ExpiresAt.lte(now)),
        )
        .await?,
        mfa_logins: delete::<mfa_login::Entity>(
            db,
            Condition::all().add(mfa_login::Column::ExpiresAt.lte(now)),
        )
        .await?,
        google_flows: delete::<google_flow::Entity>(
            db,
            Condition::all().add(google_flow::Column::ExpiresAt.lte(now)),
        )
        .await?,
        google_handoffs: delete::<google_handoff::Entity>(
            db,
            Condition::all().add(google_handoff::Column::ExpiresAt.lte(now)),
        )
        .await?,
        space_invites: delete::<space_invite::Entity>(
            db,
            Condition::all().add(space_invite::Column::ExpiresAt.lte(now)),
        )
        .await?,
        email_sends: delete::<email_send::Entity>(
            db,
            Condition::all().add(email_send::Column::CreatedAt.lte(now - KEEP)),
        )
        .await?,
    })
}

/// Purge every [`INTERVAL`], the first time right away; a failed round is
/// logged and retried at the next tick.
pub async fn run(db: DatabaseConnection) {
    let mut tick = tokio::time::interval(INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match purge(&db).await {
            Ok(p) => tracing::debug!(
                sessions = p.sessions,
                user_tokens = p.user_tokens,
                mfa_logins = p.mfa_logins,
                google_flows = p.google_flows,
                google_handoffs = p.google_handoffs,
                space_invites = p.space_invites,
                email_sends = p.email_sends,
                "expired rows purged"
            ),
            Err(e) => tracing::warn!(error = %format!("{e:#}"), "expired row purge failed"),
        }
    }
}
