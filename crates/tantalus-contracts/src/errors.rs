use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    InvalidProperties,
    CapacityExhausted,
    Conflict,
    UnknownContent,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineError {
    pub category: ErrorCategory,
    pub context: String,
    pub owner: String,
    pub recoverable: bool,
}

impl EngineError {
    pub fn new(
        category: ErrorCategory,
        context: impl Into<String>,
        owner: impl Into<String>,
        recoverable: bool,
    ) -> Self {
        Self {
            category,
            context: context.into(),
            owner: owner.into(),
            recoverable,
        }
    }
}

impl Display for EngineError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.owner, self.context)
    }
}

impl Error for EngineError {}
