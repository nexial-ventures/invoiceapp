//! Every foreign key has an index leading with its columns: a delete of the
//! referenced row (cascade / SET NULL) must not scan the referencing table.

mod common;

use common::TestDb;
use sea_orm::{ConnectionTrait, Statement};

/// FKs of the current schema without an index whose leading columns are
/// exactly the FK's columns (in any order; partial indexes count).
const UNINDEXED: &str = r#"
SELECT c.conrelid::regclass::text || ' (' ||
       (SELECT string_agg(a.attname, ', ' ORDER BY a.attname)
          FROM pg_attribute a
         WHERE a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)) || ')' AS fk
  FROM pg_constraint c
  JOIN pg_namespace n ON n.oid = c.connamespace
 WHERE c.contype = 'f'
   AND n.nspname = current_schema()
   AND NOT EXISTS (
       SELECT 1 FROM pg_index i
        WHERE i.indrelid = c.conrelid
          AND (SELECT array_agg(k ORDER BY k)
                 FROM unnest((i.indkey::int2[])[0:cardinality(c.conkey) - 1]) k)
            = (SELECT array_agg(k ORDER BY k) FROM unnest(c.conkey) k))
 ORDER BY 1
"#;

async fn unindexed(db: &TestDb) -> Vec<String> {
    db.conn
        .query_all(Statement::from_string(
            db.conn.get_database_backend(),
            UNINDEXED,
        ))
        .await
        .expect("catalog query")
        .iter()
        .map(|r| r.try_get::<String>("", "fk").expect("fk"))
        .collect()
}

#[tokio::test]
async fn every_foreign_key_is_indexed() {
    let db = TestDb::new().await;
    let missing = unindexed(&db).await;
    assert!(
        missing.is_empty(),
        "foreign keys without an index: {missing:?}"
    );
    // The check itself sees a missing one.
    db.conn
        .execute_unprepared("DROP INDEX api_tokens_user_idx")
        .await
        .expect("drop");
    assert_eq!(unindexed(&db).await, ["api_tokens (user_id)"]);
}

#[tokio::test]
async fn received_duplicate_lookup_has_an_index() {
    let db = TestDb::new().await;
    let row = db
        .conn
        .query_one(Statement::from_string(
            db.conn.get_database_backend(),
            "SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() \
             AND indexname = 'documents_received_supplier_number_idx'",
        ))
        .await
        .expect("query")
        .and_then(|r| r.try_get::<String>("", "indexdef").ok())
        .expect("index exists");
    assert!(
        row.contains("(space_id, supplier_number)") && row.contains("'received'"),
        "{row}"
    );
}
