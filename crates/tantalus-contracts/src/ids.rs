use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::hash::Hash;
use std::str::FromStr;

const MAX_ID_LENGTH: usize = 320;
const MAX_NAMESPACE_LENGTH: usize = 64;
const MAX_PATH_LENGTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentId(String);

impl ContentId {
    pub fn parse(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::new(IdErrorKind::Empty, text, "identity is empty"));
        }
        if text.len() > MAX_ID_LENGTH {
            return Err(IdError::new(
                IdErrorKind::TooLong,
                text,
                format!("identity exceeds {MAX_ID_LENGTH} bytes"),
            ));
        }

        let Some((namespace, path)) = text.split_once(':') else {
            return Err(IdError::new(
                IdErrorKind::MissingNamespace,
                text,
                "expected `namespace:path`",
            ));
        };
        if namespace.len() > MAX_NAMESPACE_LENGTH {
            return Err(IdError::new(
                IdErrorKind::InvalidNamespace,
                text,
                format!("namespace exceeds {MAX_NAMESPACE_LENGTH} bytes"),
            ));
        }
        let mut namespace_chars = namespace.chars();
        let valid_first = namespace_chars
            .next()
            .is_some_and(|character| character.is_ascii_lowercase());
        if !valid_first
            || !namespace_chars.all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '_' | '-')
            })
        {
            return Err(IdError::new(
                IdErrorKind::InvalidNamespace,
                text,
                "namespace must start with a lowercase ASCII letter and contain only lowercase letters, digits, `_`, or `-`",
            ));
        }

        if path.len() > MAX_PATH_LENGTH {
            return Err(IdError::new(
                IdErrorKind::InvalidPath,
                text,
                format!("path exceeds {MAX_PATH_LENGTH} bytes"),
            ));
        }
        let valid_path = !path.is_empty()
            && path.split('/').all(|segment| {
                let mut characters = segment.chars();
                characters.next().is_some_and(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit()
                }) && characters.all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || matches!(character, '_' | '-')
                })
            });
        if !valid_path {
            return Err(IdError::new(
                IdErrorKind::InvalidPath,
                text,
                "path segments must start with a lowercase ASCII letter or digit and contain only lowercase letters, digits, `_`, or `-`",
            ));
        }

        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn namespace(&self) -> &str {
        let separator = self
            .0
            .find(':')
            .expect("validated content identity has a namespace separator");
        &self.0[..separator]
    }

    pub fn path(&self) -> &str {
        let separator = self
            .0
            .find(':')
            .expect("validated content identity has a namespace separator");
        &self.0[separator + 1..]
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Display for ContentId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ContentId {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Serialize for ContentId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ContentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError {
    pub kind: IdErrorKind,
    pub input: String,
    pub detail: String,
}

impl IdError {
    fn new(kind: IdErrorKind, input: &str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            input: input.to_owned(),
            detail: detail.into(),
        }
    }
}

impl Display for IdError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid content identity `{}`: {}",
            self.input, self.detail
        )
    }
}

impl Error for IdError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdErrorKind {
    Empty,
    MissingNamespace,
    InvalidNamespace,
    InvalidPath,
    TooLong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ItemInstanceId(u64);

impl ItemInstanceId {
    pub fn from_u64(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl Display for ItemInstanceId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "item:{}", self.0)
    }
}

impl<'de> Deserialize<'de> for ItemInstanceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        Self::from_u64(value)
            .ok_or_else(|| serde::de::Error::custom("item instance ID must be nonzero"))
    }
}
