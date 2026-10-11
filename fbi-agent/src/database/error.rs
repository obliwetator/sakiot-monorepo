pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("recording is excluded in this voice channel")]
    RecordingExcluded,
    #[error("the user opted out of recording in this guild")]
    RecordingOptedOut,
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
