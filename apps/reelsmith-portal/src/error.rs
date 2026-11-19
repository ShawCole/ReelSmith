//! Portal errors. The message is safe to show to a reviewer: no tokens, no filesystem paths.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Invalid,
    NotFound,
    Forbidden,
    Conflict,
    Locked,
    Unsupported,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub message: String,
    pub kind: Kind,
    /// The version's current `seq`, sent with a 409 so the page knows what to re-open.
    pub seq: Option<u64>,
}

impl Error {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self { message: message.into(), kind, seq: None }
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = Some(seq);
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Kind::Invalid, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Kind::NotFound, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(Kind::Forbidden, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(Kind::Conflict, message)
    }

    pub fn locked(message: impl Into<String>) -> Self {
        Self::new(Kind::Locked, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(Kind::Unsupported, message)
    }

    pub fn status(&self) -> u16 {
        match self.kind {
            Kind::Invalid => 400,
            Kind::NotFound => 404,
            Kind::Forbidden => 403,
            Kind::Conflict => 409,
            Kind::Locked => 423,
            Kind::Unsupported => 501,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        // The OS error can contain a path. Keep the kind, drop the path.
        Self::invalid(format!("storage error ({})", value.kind()))
    }
}
