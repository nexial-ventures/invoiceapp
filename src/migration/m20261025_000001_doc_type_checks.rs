//! `doc_type` CHECKs matching the typed enum (`document::doc_type::DocType`):
//! `documents` loses the legacy `'received'` value (received documents are
//! `direction = 'received'` with a real type since 1f-a), `number_series`
//! gets a CHECK on the fourteen series keys. A row outside them aborts the
//! migration with its values named instead of failing a constraint later.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const UP: &str = r#"
DO $$
DECLARE
    bad text;
BEGIN
    SELECT string_agg(DISTINCT doc_type, ', ') INTO bad FROM documents
    WHERE doc_type NOT IN ('invoice', 'credit_note', 'debit_note', 'proforma', 'advance_tax_doc',
                           'advance_credit_note', 'simplified');
    IF bad IS NOT NULL THEN
        RAISE EXCEPTION 'documents.doc_type holds values outside the document types: %. Fix those rows before upgrading.', bad;
    END IF;
    SELECT string_agg(DISTINCT doc_type, ', ') INTO bad FROM number_series
    WHERE doc_type NOT IN ('invoice', 'credit_note', 'debit_note', 'proforma', 'advance_tax_doc',
                           'advance_credit_note', 'simplified', 'received', 'received_credit_note',
                           'received_debit_note', 'received_proforma', 'received_advance_tax_doc',
                           'received_advance_credit_note', 'received_simplified');
    IF bad IS NOT NULL THEN
        RAISE EXCEPTION 'number_series.doc_type holds values outside the series keys: %. Fix those rows before upgrading.', bad;
    END IF;
END
$$;

ALTER TABLE documents DROP CONSTRAINT documents_doc_type_check;
ALTER TABLE documents ADD CONSTRAINT documents_doc_type_check
    CHECK (doc_type IN ('invoice', 'credit_note', 'debit_note', 'proforma', 'advance_tax_doc',
                        'advance_credit_note', 'simplified'));
ALTER TABLE number_series ADD CONSTRAINT number_series_doc_type_check
    CHECK (doc_type IN ('invoice', 'credit_note', 'debit_note', 'proforma', 'advance_tax_doc',
                        'advance_credit_note', 'simplified', 'received', 'received_credit_note',
                        'received_debit_note', 'received_proforma', 'received_advance_tax_doc',
                        'received_advance_credit_note', 'received_simplified'));
"#;

const DOWN: &str = r#"
ALTER TABLE number_series DROP CONSTRAINT number_series_doc_type_check;
ALTER TABLE documents DROP CONSTRAINT documents_doc_type_check;
ALTER TABLE documents ADD CONSTRAINT documents_doc_type_check
    CHECK (doc_type IN ('invoice', 'credit_note', 'debit_note', 'proforma', 'advance_tax_doc',
                        'advance_credit_note', 'simplified', 'received'));
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(UP).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(DOWN).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::UP;
    use crate::document::doc_type::DocType;

    /// The quoted values of every `IN (…)` list after `marker` in `UP`.
    fn listed(marker: &str) -> Vec<String> {
        let at = UP.find(marker).expect("marker");
        let rest = &UP[at..];
        let list = &rest[rest.find("IN (").expect("IN") + 4..];
        let list = &list[..list.find(')').expect("close")];
        list.split(',')
            .map(|v| v.trim().trim_matches('\'').to_string())
            .collect()
    }

    #[test]
    fn checks_list_exactly_the_enum_values() {
        let all: Vec<String> = DocType::ALL.iter().map(|d| d.as_str().into()).collect();
        let docs: Vec<String> = DocType::ALL
            .iter()
            .filter(|d| d.is_document_type())
            .map(|d| d.as_str().into())
            .collect();
        assert_eq!(listed("ADD CONSTRAINT documents_doc_type_check"), docs);
        assert_eq!(listed("ADD CONSTRAINT number_series_doc_type_check"), all);
        assert_eq!(listed("INTO bad FROM documents"), docs);
        assert_eq!(listed("INTO bad FROM number_series"), all);
    }
}
