use sqlx::postgres::PgQueryResult;

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("unexpected row count for {operation}: expected {expected}, got {actual}")]
    UnexpectedRows {
        operation: &'static str,
        expected: u64,
        actual: u64,
    },
}

pub fn expect_rows(result: PgQueryResult, expected: u64, operation: &'static str) -> DbResult<u64> {
    let actual = result.rows_affected();
    if actual == expected {
        Ok(actual)
    } else {
        Err(DbError::UnexpectedRows {
            operation,
            expected,
            actual,
        })
    }
}
