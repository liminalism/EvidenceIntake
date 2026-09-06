//! Error types for the office layer.

/// A result returned by the office layer.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while storing or reading office records.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `SQLite` rejected an operation.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// A requested record does not exist.
    #[error("{kind} `{id}` was not found")]
    NotFound {
        /// What kind of record was looked for.
        kind: &'static str,
        /// The identifier that matched nothing.
        id: String,
    },
    /// A record was written twice under the same identity.
    #[error("{kind} `{id}` already exists")]
    AlreadyExists {
        /// What kind of record was being written.
        kind: &'static str,
        /// The identifier already in use.
        id: String,
    },
    /// A client, matter, appearance, deadline or note was malformed.
    #[error("invalid office record: {0}")]
    InvalidRecord(String),
    /// A date or time was not a real calendar date in the stored format.
    #[error("`{value}` is not a {expected}")]
    InvalidDate {
        /// The text that could not be read.
        value: String,
        /// What the field required, for example `YYYY-MM-DD date`.
        expected: &'static str,
    },
    /// A write tried to join records belonging to different clients.
    #[error("{kind} `{id}` belongs to another client; a setting spans the matters of one person")]
    WrongClient {
        /// What kind of record crossed the boundary.
        kind: &'static str,
        /// The identifier that belongs elsewhere.
        id: String,
    },
    /// A superseded note was revised instead of the current one.
    #[error("note `{id}` has already been superseded{}; revise the current version instead",
            .by.as_ref().map(|id| format!(" by `{id}`")).unwrap_or_default())]
    Superseded {
        /// The note that is no longer current.
        id: String,
        /// The revision that replaced it, when one is recorded.
        by: Option<String>,
    },
    /// A search query was empty or could not be parsed as full-text syntax.
    #[error("invalid search: {0}")]
    InvalidSearch(String),
    /// JSON output could not be produced.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
