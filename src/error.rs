//! Error types for the collation kernel.

/// A result returned by the collation kernel.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while storing or reading a case.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite rejected an operation.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// A requested record does not exist.
    #[error("{kind} `{id}` was not found")]
    NotFound {
        /// The type of record being requested.
        kind: &'static str,
        /// The application identifier.
        id: String,
    },
    /// Fixture data failed an invariant.
    #[error("invalid fixture: {0}")]
    InvalidFixture(String),
    /// A review decision was malformed or lacked a required justification.
    #[error("invalid review decision: {0}")]
    InvalidReview(String),
    /// A review decision asked for a state change the workflow does not allow.
    #[error("cannot move {target} `{id}` from `{from}` to `{to}`: {reason}")]
    InvalidTransition {
        /// The record type being reviewed.
        target: &'static str,
        /// The application identifier.
        id: String,
        /// State the record currently holds.
        from: String,
        /// State the reviewer asked for.
        to: String,
        /// Why the workflow refuses the move.
        reason: &'static str,
    },
    /// A verification cited a locator other than the record's own original.
    #[error("verification of `{id}` cited `{cited}` but its original locator is `{actual}`")]
    LocatorMismatch {
        /// The application identifier.
        id: String,
        /// Locator the reviewer claimed to have opened.
        cited: String,
        /// Locator the record actually points at.
        actual: String,
    },
    /// JSON output could not be produced.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
