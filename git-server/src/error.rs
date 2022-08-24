#![allow(clippy::large_enum_variant)]
use axum::response::{IntoResponse, Response};

/// Errors that may occur when interacting with the radicle git server or git hooks.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// I/O error.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// The content encoding is not supported.
    #[error("content encoding '{0}' not supported")]
    UnsupportedContentEncoding(&'static str),

    /// The service is not available.
    #[error("service '{0}' not available")]
    ServiceUnavailable(&'static str),

    /// HTTP error.
    #[error("HTTP error: {0}")]
    Http(#[from] http::Error),

    /// Git backend error.
    #[error("backend error")]
    Backend,

    /// Project has no default branch.
    #[error("project has no default branch")]
    NoDefaultBranch,

    /// Custom hook failed to spawn.
    #[error("custom hook failed to spawn: {0}")]
    CustomHook(std::io::Error),

    /// Failed certificate verification.
    #[error("failed certification verification")]
    FailedCertificateVerification,

    /// Unauthorized.
    #[error("unauthorized: {0}")]
    Unauthorized(&'static str),

    /// Post-receive hook error.
    #[error("{0}")]
    PostReceive(&'static str),

    /// Signer key mismatch.
    #[error("signer key mismatch: expected {expected}, got {actual}")]
    KeyMismatch { actual: String, expected: String },

    /// Project alias not found.
    #[error("alias does not exist")]
    AliasNotFound,

    /// Id is not valid.
    #[error("id is not valid")]
    InvalidId,

    /// Peer ID is invalid.
    #[error("peer-id is invalid")]
    InvalidPeerId,

    /// Invalid ref pushed.
    #[error("invalid ref pushed: {0}")]
    InvalidRefPushed(String),

    /// Namespace not found.
    #[error("namespace does not exist")]
    NamespaceNotFound,

    /// Reference not found.
    #[error("reference not found")]
    ReferenceNotFound,

    /// Radicle identity not found for project.
    #[error("radicle identity is not found for project")]
    RadicleIdentityNotFound,

    /// Environmental variable error.
    #[error("environmental variable error: {0}")]
    VarError(#[from] std::env::VarError),

    /// Git config parser error.
    #[error("git2 error: {0}")]
    Git2Error(#[from] git2::Error),

    /// Missing certification signer credentials.
    #[error("missing certificate signer credentials: {0}")]
    MissingCertificateSignerCredentials(String),

    /// Missing environmental variable.
    #[cfg(feature = "hooks")]
    #[error("missing environmental config variable: {0}")]
    EnvConfigError(#[from] envconfig::Error),

    /// Failed to parse byte data into string.
    #[error(transparent)]
    Utf8Error(#[from] std::str::Utf8Error),

    /// Librad profile error.
    #[error(transparent)]
    Profile(#[from] librad::profile::Error),

    /// Failed to connect to unix socket.
    #[error("failed to connect to unix socket")]
    UnixSocket,

    /// An error occured with initializing read-only storage.
    #[error(transparent)]
    Init(#[from] librad::git::storage::read::error::Init),

    /// An error occured with radicle identities.
    #[error(transparent)]
    Identities(#[from] librad::git::identities::Error),

    /// An error occured while verifying an identity.
    #[error("error verifying identity: {0}")]
    VerifyIdentity(String),

    /// An error occured with a git storage pool.
    #[error("storage error: {0}")]
    Pool(#[from] librad::git::storage::pool::PoolError),

    /// Stored refs error.
    #[error(transparent)]
    Stored(#[from] librad::git::refs::stored::Error),

    /// Tracking error.
    #[error(transparent)]
    Track(#[from] librad::git::tracking::error::Track),

    /// Tracking error (inner).
    #[error(transparent)]
    PreviousError(#[from] librad::git::tracking::git::refdb::PreviousError<librad::git_ext::Oid>),

    /// HeaderName error.
    #[error(transparent)]
    InvalidHeaderName(#[from] axum::http::header::InvalidHeaderName),

    /// HeaderValue error.
    #[error(transparent)]
    InvalidHeaderValue(#[from] axum::http::header::InvalidHeaderValue),
}

impl Error {
    pub fn status(&self) -> http::StatusCode {
        match self {
            Error::UnsupportedContentEncoding(_) => http::StatusCode::NOT_IMPLEMENTED,
            Error::ServiceUnavailable(_) => http::StatusCode::SERVICE_UNAVAILABLE,
            Error::Unauthorized(_) => http::StatusCode::UNAUTHORIZED,
            Error::KeyMismatch { .. } => http::StatusCode::UNAUTHORIZED,
            Error::AliasNotFound => http::StatusCode::NOT_FOUND,
            _ => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        tracing::error!("{}", self);

        self.status().into_response()
    }
}
