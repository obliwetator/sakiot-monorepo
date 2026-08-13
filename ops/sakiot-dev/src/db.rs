//! Local PostgreSQL boundary.
//!
//! Migrations, seed data, fixture COPY, and pruning all go through SQLx. The
//! trait keeps orchestration tests independent from a live database while the
//! production implementation uses a single connection for each import batch.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::Connection;
use sqlx::postgres::{PgConnection, PgPoolCopyExt, PgPoolOptions, Postgres};
use sqlx::{AssertSqlSafe, PgPool, raw_sql};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../sakiot-db/migrations");

#[derive(Debug, Clone)]
pub struct CopySection {
    pub statement: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct SqlBatch {
    pub setup: String,
    pub copies: Vec<CopySection>,
    pub finish: String,
}

impl SqlBatch {
    pub fn render_psql(&self) -> Vec<u8> {
        let mut output = String::new();
        output.push_str("BEGIN;\n");
        output.push_str(&self.setup);
        for section in &self.copies {
            output.push_str(&section.statement);
            output.push_str(";\n");
            output.push_str(&String::from_utf8_lossy(&section.data));
            if !section.data.ends_with(b"\n") {
                output.push('\n');
            }
            output.push_str("\\.\n");
        }
        output.push_str(&self.finish);
        output.push_str("COMMIT;\n");
        output.into_bytes()
    }
}

#[async_trait]
pub trait LocalDatabase: Send + Sync {
    async fn migrate(&self) -> Result<()>;
    async fn seed(&self, dev_account_id: i64) -> Result<()>;
    async fn copy_out(&self, query: &str) -> Result<Vec<u8>>;
    async fn table_columns(&self, table: &str) -> Result<Vec<String>>;
    async fn run_batch(&self, batch: &SqlBatch) -> Result<()>;
    async fn execute(&self, sql: &str) -> Result<()>;
    async fn scalar_text(&self, query: &str) -> Result<String>;
}

#[async_trait]
pub trait DatabaseFactory: Send + Sync {
    async fn connect(&self, url: &str) -> Result<Arc<dyn LocalDatabase>>;
}

pub struct RealDatabaseFactory;

#[async_trait]
impl DatabaseFactory for RealDatabaseFactory {
    async fn connect(&self, url: &str) -> Result<Arc<dyn LocalDatabase>> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .with_context(|| format!("could not connect to local PostgreSQL at {url}"))?;
        Ok(Arc::new(SqlxDatabase {
            pool,
            url: url.to_string(),
        }))
    }
}

struct SqlxDatabase {
    pool: PgPool,
    url: String,
}

#[async_trait]
impl LocalDatabase for SqlxDatabase {
    async fn migrate(&self) -> Result<()> {
        MIGRATOR
            .run(&self.pool)
            .await
            .context("embedded local migrations failed")?;
        Ok(())
    }

    async fn seed(&self, dev_account_id: i64) -> Result<()> {
        if dev_account_id < 0 {
            bail!("DEV_ACCOUNT_ID must be non-negative")
        }
        let sql = include_str!("../../../scripts/dev-seed.sql")
            .replace(":dev_id", &dev_account_id.to_string());
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("could not begin seed transaction")?;
        raw_sql(AssertSqlSafe(sql))
            .execute(&mut *transaction)
            .await
            .context("local development seed failed")?;
        transaction
            .commit()
            .await
            .context("could not commit development seed")?;
        Ok(())
    }

    async fn copy_out(&self, query: &str) -> Result<Vec<u8>> {
        let statement = format!("COPY ({query}) TO STDOUT WITH (FORMAT text)");
        let mut stream = self
            .pool
            .copy_out_raw(&statement)
            .await
            .with_context(|| format!("local COPY query failed: {query}"))?;
        let mut output = Vec::new();
        while let Some(chunk) = stream.try_next().await? {
            output.extend_from_slice(&chunk);
        }
        Ok(output)
    }

    async fn table_columns(&self, table: &str) -> Result<Vec<String>> {
        validate_identifier(table)?;
        sqlx::query_scalar::<Postgres, String>(
            "SELECT column_name
               FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = $1
              ORDER BY ordinal_position",
        )
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("could not inspect columns for {table}"))
    }

    async fn run_batch(&self, batch: &SqlBatch) -> Result<()> {
        let mut connection = PgConnection::connect(&self.url)
            .await
            .context("could not open the local fixture import connection")?;
        raw_sql("BEGIN")
            .execute(&mut connection)
            .await
            .context("could not begin local fixture transaction")?;

        let result = async {
            raw_sql(AssertSqlSafe(batch.setup.clone()))
                .execute(&mut connection)
                .await
                .context("fixture import setup failed")?;
            for section in &batch.copies {
                let mut copy = connection
                    .copy_in_raw(&section.statement)
                    .await
                    .with_context(|| format!("fixture COPY failed: {}", section.statement))?;
                copy.send(section.data.as_slice())
                    .await
                    .context("could not stream fixture COPY data")?;
                copy.finish()
                    .await
                    .context("could not finish fixture COPY")?;
            }
            raw_sql(AssertSqlSafe(batch.finish.clone()))
                .execute(&mut connection)
                .await
                .context("fixture import transaction failed")?;
            raw_sql("COMMIT")
                .execute(&mut connection)
                .await
                .context("could not commit fixture import")?;
            Result::<(), anyhow::Error>::Ok(())
        }
        .await;

        if result.is_err() {
            let _ = raw_sql("ROLLBACK").execute(&mut connection).await;
        }
        result
    }

    async fn execute(&self, sql: &str) -> Result<()> {
        raw_sql(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .with_context(|| format!("local SQL failed: {sql}"))?;
        Ok(())
    }

    async fn scalar_text(&self, query: &str) -> Result<String> {
        sqlx::query_scalar::<Postgres, String>(AssertSqlSafe(query.to_owned()))
            .fetch_one(&self.pool)
            .await
            .with_context(|| format!("local scalar query failed: {query}"))
    }
}

fn validate_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("unsafe SQL identifier: {value}")
    }
    Ok(())
}

pub fn quote_identifier(value: &str) -> Result<String> {
    validate_identifier(value)?;
    Ok(format!("\"{value}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psql_batch_keeps_copy_data_and_finish_ordered() {
        let batch = SqlBatch {
            setup: String::new(),
            copies: vec![CopySection {
                statement: "COPY temp_table FROM STDIN".into(),
                data: b"one\ttwo\n".to_vec(),
            }],
            finish: "COMMIT;\n".into(),
        };
        let rendered = String::from_utf8(batch.render_psql()).unwrap();
        assert!(rendered.starts_with("BEGIN;\nCOPY temp_table FROM STDIN;\none\ttwo\n\\.\n"));
        assert!(rendered.ends_with("COMMIT;\n"));
    }

    #[test]
    fn identifiers_are_restricted_to_schema_names() {
        assert!(quote_identifier("audio_files").is_ok());
        assert!(quote_identifier("audio_files;drop").is_err());
    }
}
