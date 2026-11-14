use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveErrorCategory {
    Io,
    InvalidData,
    Corrupt,
    UnsupportedVersion,
    MissingRequiredModule,
    MigrationFailed,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveError {
    pub category: SaveErrorCategory,
    pub operation: String,
    pub path: Option<PathBuf>,
    pub context: String,
    pub recoverable: bool,
}

impl SaveError {
    pub fn new(
        category: SaveErrorCategory,
        operation: impl Into<String>,
        path: Option<PathBuf>,
        context: impl Into<String>,
        recoverable: bool,
    ) -> Self {
        Self {
            category,
            operation: operation.into(),
            path,
            context: context.into(),
            recoverable,
        }
    }

    pub(crate) fn io(operation: &str, path: &Path, error: std::io::Error) -> Self {
        Self::new(
            SaveErrorCategory::Io,
            operation,
            Some(path.to_path_buf()),
            error.to_string(),
            true,
        )
    }

    pub(crate) fn invalid(context: impl Into<String>) -> Self {
        Self::new(
            SaveErrorCategory::InvalidData,
            "validate",
            None,
            context,
            true,
        )
    }
}

impl Display for SaveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "save {}: {}", self.operation, self.context)?;
        if let Some(path) = &self.path {
            write!(formatter, " ({})", path.display())?;
        }
        Ok(())
    }
}

impl Error for SaveError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFailurePoint {
    BeforeGenerationPublish,
    BeforeLatestPublish,
    AfterLatestPublish,
}
