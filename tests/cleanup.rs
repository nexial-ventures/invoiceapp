//! The hourly purge of expired rows (`invoice::cleanup`).

mod common;

use common::TestDb;
use invoice::cleanup::{Purged, purge};
use sea_orm::{ConnectionTrait, Statement};

/// One expired and one live row per table (sessions: absolute and idle
/// expiry), `$1` = the user, `$2` = the space.
const ROWS: &str = r#"
INSERT INTO sessions (id, token_hash, user_id, space_id, created_at, last_seen_at, expires_at) VALUES
  (gen_random_uuid(), 's-absolute', $1, $2, now() - interval '91 days', now() - interval '1 hour', now() - interval '1 day'),
  (gen_random_uuid(), 's-idle', $1, $2, now() - interval '20 days', now() - interval '15 days', now() + interval '70 days'),
  (gen_random_uuid(), 's-live', $1, $2, now(), now(), now() + interval '90 days');
INSERT INTO user_tokens (id, user_id, kind, token_hash, expires_at) VALUES
  (gen_random_uuid(), $1, 'reset', 'u-old', now() - interval '1 minute'),
  (gen_random_uuid(), $1, 'verify', 'u-live', now() + interval '1 day');
INSERT INTO mfa_logins (id, token_hash, user_id, space_id, expires_at) VALUES
  (gen_random_uuid(), 'm-old', $1, $2, now() - interval '1 minute'),
  (gen_random_uuid(), 'm-live', $1, $2, now() + interval '5 minutes');
INSERT INTO google_flows (id, state_hash, space_id, purpose, pkce_verifier, nonce, return_path,
                          browser_cookie, browser_hash, expires_at) VALUES
  (gen_random_uuid(), 'f-old', $2, 'login', 'v', 'n', '/', 'c', 'h', now() - interval '1 minute'),
  (gen_random_uuid(), 'f-live', $2, 'login', 'v', 'n', '/', 'c', 'h', now() + interval '10 minutes');
INSERT INTO google_handoffs (id, token_hash, space_id, purpose, return_path, google_sub, google_email,
                             browser_cookie, browser_hash, expires_at) VALUES
  (gen_random_uuid(), 'h-old', $2, 'login', '/', 'sub', 'a@example.com', 'c', 'h', now() - interval '1 second'),
  (gen_random_uuid(), 'h-live', $2, 'login', '/', 'sub', 'a@example.com', 'c', 'h', now() + interval '1 minute');
INSERT INTO space_invites (id, space_id, email, role, token_hash, invited_by, expires_at) VALUES
  (gen_random_uuid(), $2, 'old@example.com', 'member', 'i-old', $1, now() - interval '1 minute'),
  (gen_random_uuid(), $2, 'new@example.com', 'member', 'i-live', $1, now() + interval '7 days');
INSERT INTO email_sends (space_id, user_id, kind, created_at) VALUES
  ($2, $1, 'e-old', now() - interval '2 days 1 minute'),
  ($2, $1, 'e-live', now() - interval '47 hours');
"#;

async fn count(db: &TestDb, table: &str, like: &str) -> i64 {
    let hash = match table {
        "google_flows" => "state_hash",
        // The marker of an `email_sends` row (no token there).
        "email_sends" => "kind",
        _ => "token_hash",
    };
    db.conn
        .query_one(Statement::from_string(
            db.conn.get_database_backend(),
            format!("SELECT count(*) AS n FROM {table} WHERE {hash} LIKE '{like}'"),
        ))
        .await
        .expect("count")
        .and_then(|r| r.try_get::<i64>("", "n").ok())
        .expect("n")
}

#[tokio::test]
async fn purge_deletes_only_expired_rows() {
    let db = TestDb::new().await;
    for stmt in ROWS.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        db.conn
            .execute(Statement::from_sql_and_values(
                db.conn.get_database_backend(),
                stmt,
                [db.user_id.into(), db.space.uuid().into()],
            ))
            .await
            .unwrap_or_else(|e| panic!("{stmt}: {e}"));
    }

    let purged = purge(&db.conn).await.expect("purge");
    assert_eq!(
        purged,
        Purged {
            sessions: 2,
            user_tokens: 1,
            mfa_logins: 1,
            google_flows: 1,
            google_handoffs: 1,
            space_invites: 1,
            email_sends: 1,
        }
    );
    for (table, prefix) in [
        ("sessions", "s-"),
        ("user_tokens", "u-"),
        ("mfa_logins", "m-"),
        ("google_flows", "f-"),
        ("google_handoffs", "h-"),
        ("space_invites", "i-"),
        ("email_sends", "e-"),
    ] {
        assert_eq!(
            count(&db, table, &format!("{prefix}live")).await,
            1,
            "{table} live row kept"
        );
        assert_eq!(
            count(&db, table, &format!("{prefix}%")).await,
            1,
            "{table} expired rows gone"
        );
    }
    assert_eq!(
        purge(&db.conn).await.expect("again"),
        Purged::default(),
        "nothing left"
    );
}
