//! Error types for the collation kernel.

/// A result returned by the collation kernel.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while storing or reading a case.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `SQLite` rejected an operation.
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
    /// An authored proposition or relationship was malformed.
    #[error("invalid authoring: {0}")]
    InvalidAuthoring(String),
    /// An interpretation, source profile, or content grouping was malformed.
    #[error("invalid interpretation: {0}")]
    InvalidInterpretation(String),
    /// A search query was empty or could not be parsed as full-text syntax.
    #[error("invalid search: {0}")]
    InvalidSearch(String),
    /// A keyframe embedding could not be stored against a derived still.
    #[error("invalid keyframe index: {0}")]
    InvalidIndex(String),
    /// A persistent intake request, result, state transition or source path was invalid.
    #[error("invalid intake: {0}")]
    InvalidIntake(String),
    /// Reading or verifying a local original/artifact failed.
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    /// A proposition was mapped again to an element it is already filed under.
    #[error(
        "proposition `{proposition}` is already assessed `{assessment}` on element `{element}`"
    )]
    ElementAlreadyMapped {
        /// The element being mapped.
        element: String,
        /// The proposition already filed under it.
        proposition: String,
        /// The direction it is already filed under.
        assessment: String,
    },
    /// An export would have carried a factual line with no original to open.
    #[error(
        "evidence from `{source_name}` bearing on proposition `{proposition}` has no original \
         locator; an export must resolve every factual line to something a reader can open"
    )]
    UnlocatedExport {
        /// The proposition the line bears on.
        proposition: String,
        /// The source the line came from. Not named `source`: `thiserror`
        /// reserves that field for a wrapped error.
        source_name: String,
    },
    /// A superseded version was revised instead of the current one.
    #[error("{kind} `{id}` has already been superseded{}; revise the current version instead",
            .by.as_ref().map(|id| format!(" by `{id}`")).unwrap_or_default())]
    Superseded {
        /// The type of record being revised.
        kind: &'static str,
        /// The version the caller tried to revise.
        id: String,
        /// The version that replaced it.
        by: Option<String>,
    },
    /// A record was written twice under the same identity.
    #[error("{kind} `{id}` already exists")]
    AlreadyExists {
        /// The type of record being written.
        kind: &'static str,
        /// The application identifier or the claim being repeated.
        id: String,
    },
    /// A write tried to attach a record that belongs to a different case.
    #[error("{kind} `{id}` belongs to another case; cases do not share records")]
    WrongCase {
        /// The type of record being attached.
        kind: &'static str,
        /// The application identifier that belongs elsewhere.
        id: String,
    },
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
    /// A generated factual sentence was not backed by an exact locator.
    #[error("unsupported factual sentence: {0}")]
    UnsupportedSentence(String),
}
