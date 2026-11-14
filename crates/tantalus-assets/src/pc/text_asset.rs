use crate::install::ImportError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextAsset {
    pub name: String,
    pub script: Vec<u8>,
    pub path_name: String,
}

impl TextAsset {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut cursor = Cursor { bytes, position: 0 };
        let name = cursor.string(source, "name")?;
        let script = cursor.bytes(source, "script")?;
        let path_name = cursor.string(source, "path name")?;
        if cursor.position != bytes.len() {
            return Err(ImportError::invalid(
                source,
                format!(
                    "TextAsset has {} unexpected trailing bytes",
                    bytes.len() - cursor.position
                ),
            ));
        }
        Ok(Self {
            name,
            script,
            path_name,
        })
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn string(&mut self, source: &str, field: &str) -> Result<String, ImportError> {
        String::from_utf8(self.bytes(source, field)?).map_err(|error| {
            ImportError::invalid(source, format!("TextAsset {field} is not UTF-8: {error}"))
        })
    }

    fn bytes(&mut self, source: &str, field: &str) -> Result<Vec<u8>, ImportError> {
        let length = self.i32(source)?;
        let length = usize::try_from(length).map_err(|_| {
            ImportError::invalid(source, format!("TextAsset {field} has negative length"))
        })?;
        if length > 64 * 1024 * 1024 {
            return Err(ImportError::invalid(
                source,
                format!("TextAsset {field} exceeds the 64 MiB limit"),
            ));
        }
        let end = self.position.checked_add(length).ok_or_else(|| {
            ImportError::invalid(source, format!("TextAsset {field} length overflow"))
        })?;
        let value = self.bytes.get(self.position..end).ok_or_else(|| {
            ImportError::invalid(source, format!("TextAsset {field} is truncated"))
        })?;
        self.position = end;
        self.align(source)?;
        Ok(value.to_vec())
    }

    fn i32(&mut self, source: &str) -> Result<i32, ImportError> {
        let end = self.position + 4;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| ImportError::invalid(source, "TextAsset length prefix is truncated"))?;
        self.position = end;
        Ok(i32::from_le_bytes(
            bytes.try_into().expect("four-byte length"),
        ))
    }

    fn align(&mut self, source: &str) -> Result<(), ImportError> {
        let padding = (4 - self.position % 4) % 4;
        let end = self.position + padding;
        if end > self.bytes.len() {
            return Err(ImportError::invalid(
                source,
                "TextAsset alignment is truncated",
            ));
        }
        self.position = end;
        Ok(())
    }
}
