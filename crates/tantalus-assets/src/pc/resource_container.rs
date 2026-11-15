//! Unity `ResourceManager` (class 147) container: the `Resources.Load` path
//! table.
//!
//! The PC build keeps it in `FF9_Data/mainData` (format 15, Unity 5.2). Its
//! payload starts with `m_Container`, a `vector<pair<string, PPtr<Object>>>`:
//! an `i32` count, then for each entry a length-prefixed UTF-8 path padded to
//! 4 bytes and a `PPtr` (`i32` file id, `i64` path id). File id 0 is the file
//! holding the manager; `n > 0` is its `n`-th external reference. Memoria
//! loads field text as `EmbeddedAsset/Text/<lang>/Field/<zone>.mes`
//! (`FF9TextTool`, `AssetManager.LoadString`) through this table, which is how
//! the same `2.mes` name in several languages is told apart.

use serde::{Deserialize, Serialize};

use crate::install::ImportError;

/// `ResourceManager` class id.
pub const RESOURCE_MANAGER_CLASS_ID: u16 = 147;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceEntry {
    /// Lower-case resource path without extension handling, as stored.
    pub path: String,
    pub file_id: i32,
    pub path_id: i64,
}

/// Parses `m_Container` from a `ResourceManager` payload (little-endian).
pub fn parse_resource_container(
    bytes: &[u8],
    source: &str,
) -> Result<Vec<ResourceEntry>, ImportError> {
    let mut at = 0usize;
    let take = |at: &mut usize, len: usize| -> Result<&[u8], ImportError> {
        let end = at
            .checked_add(len)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| {
                ImportError::invalid(source, "ResourceManager container is truncated")
            })?;
        let slice = &bytes[*at..end];
        *at = end;
        Ok(slice)
    };
    let count = i32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4"));
    if !(0..=1_000_000).contains(&count) {
        return Err(ImportError::invalid(
            source,
            format!("implausible ResourceManager entry count {count}"),
        ));
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len = i32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4"));
        let len = usize::try_from(len)
            .map_err(|_| ImportError::invalid(source, "negative resource path length"))?;
        let path = std::str::from_utf8(take(&mut at, len)?)
            .map_err(|_| ImportError::invalid(source, "resource path is not UTF-8"))?
            .to_owned();
        take(&mut at, (4 - len % 4) % 4)?;
        let file_id = i32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4"));
        let path_id = i64::from_le_bytes(take(&mut at, 8)?.try_into().expect("8"));
        entries.push(ResourceEntry {
            path,
            file_id,
            path_id,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(out: &mut Vec<u8>, path: &str, file_id: i32, path_id: i64) {
        out.extend((path.len() as i32).to_le_bytes());
        out.extend(path.as_bytes());
        out.resize(out.len().next_multiple_of(4), 0);
        out.extend(file_id.to_le_bytes());
        out.extend(path_id.to_le_bytes());
    }

    #[test]
    fn parses_aligned_paths_and_pointers() {
        let mut bytes = 2i32.to_le_bytes().to_vec();
        entry(&mut bytes, "embeddedasset/text/us/field/2", 1, 4242);
        entry(&mut bytes, "abcd", 0, -7);
        bytes.extend([9, 9, 9, 9]); // trailing m_DependentAssets, ignored
        let entries = parse_resource_container(&bytes, "test").unwrap();
        assert_eq!(
            entries,
            vec![
                ResourceEntry {
                    path: "embeddedasset/text/us/field/2".into(),
                    file_id: 1,
                    path_id: 4242
                },
                ResourceEntry {
                    path: "abcd".into(),
                    file_id: 0,
                    path_id: -7
                },
            ]
        );
    }

    #[test]
    fn truncated_or_implausible_payloads_fail() {
        assert!(parse_resource_container(&[1, 0], "test").is_err());
        assert!(parse_resource_container(&(-1i32).to_le_bytes(), "test").is_err());
        let mut bytes = 1i32.to_le_bytes().to_vec();
        bytes.extend(10i32.to_le_bytes());
        bytes.extend(b"short");
        assert!(parse_resource_container(&bytes, "test").is_err());
    }
}
