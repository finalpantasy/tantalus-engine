use crate::install::ImportError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetInfo {
    pub preload_index: i32,
    pub preload_size: i32,
    pub file_id: i32,
    pub path_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetBundleMetadata {
    pub name: String,
    pub preload_table: Vec<(i32, i64)>,
    pub container: Vec<(String, AssetInfo)>,
    pub main_asset: AssetInfo,
    pub runtime_compatibility: u32,
    pub asset_bundle_name: String,
    pub dependencies: Vec<String>,
    pub is_streamed_scene: bool,
}

impl AssetBundleMetadata {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut cursor = Cursor { bytes, position: 0 };
        let name = cursor.string(source)?;
        let preload_count = cursor.count(source, 100_000, "preload")?;
        let mut preload_table = Vec::with_capacity(preload_count);
        for _ in 0..preload_count {
            preload_table.push((cursor.i32(source)?, cursor.i64(source)?));
        }
        cursor.align(source)?;
        let container_count = cursor.count(source, 100_000, "container")?;
        let mut container = Vec::with_capacity(container_count);
        for _ in 0..container_count {
            let path = cursor.string(source)?;
            let info = cursor.asset_info(source)?;
            container.push((path, info));
        }
        cursor.align(source)?;
        let main_asset = cursor.asset_info(source)?;
        let runtime_compatibility = cursor.u32(source)?;
        let asset_bundle_name = cursor.string(source)?;
        let dependency_count = cursor.count(source, 4_096, "dependency")?;
        let mut dependencies = Vec::with_capacity(dependency_count);
        for _ in 0..dependency_count {
            dependencies.push(cursor.string(source)?);
        }
        let is_streamed_scene = cursor.byte(source)? != 0;
        cursor.align(source)?;
        if cursor.position != bytes.len() {
            return Err(invalid(
                source,
                "AssetBundle metadata has unexpected trailing bytes",
            ));
        }
        Ok(Self {
            name,
            preload_table,
            container,
            main_asset,
            runtime_compatibility,
            asset_bundle_name,
            dependencies,
            is_streamed_scene,
        })
    }

    pub fn atlas_for(&self, field_name: &str) -> Option<&AssetInfo> {
        let path = format!(
            "assets/resources/fieldmaps/{}/atlas.png",
            field_name.to_lowercase()
        );
        self.container
            .iter()
            .find(|(candidate, _)| candidate == &path)
            .map(|(_, info)| info)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn asset_info(&mut self, source: &str) -> Result<AssetInfo, ImportError> {
        Ok(AssetInfo {
            preload_index: self.i32(source)?,
            preload_size: self.i32(source)?,
            file_id: self.i32(source)?,
            path_id: self.i64(source)?,
        })
    }

    fn count(&mut self, source: &str, maximum: usize, kind: &str) -> Result<usize, ImportError> {
        let value = self.i32(source)?;
        let value = usize::try_from(value)
            .map_err(|_| invalid(source, format!("negative {kind} count")))?;
        if value > maximum {
            return Err(invalid(source, format!("{kind} count exceeds {maximum}")));
        }
        Ok(value)
    }

    fn string(&mut self, source: &str) -> Result<String, ImportError> {
        let length = self.i32(source)?;
        let length = usize::try_from(length)
            .map_err(|_| invalid(source, "AssetBundle string has negative length"))?;
        if length > 1024 * 1024 {
            return Err(invalid(source, "AssetBundle string exceeds its limit"));
        }
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| invalid(source, "AssetBundle string range overflow"))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid(source, "AssetBundle string is truncated"))?;
        let value = String::from_utf8(bytes.to_vec()).map_err(|error| {
            invalid(source, format!("AssetBundle string is not UTF-8: {error}"))
        })?;
        self.position = end;
        self.align(source)?;
        Ok(value)
    }

    fn i32(&mut self, source: &str) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.array(source)?))
    }

    fn i64(&mut self, source: &str) -> Result<i64, ImportError> {
        Ok(i64::from_le_bytes(self.array(source)?))
    }

    fn u32(&mut self, source: &str) -> Result<u32, ImportError> {
        Ok(u32::from_le_bytes(self.array(source)?))
    }

    fn byte(&mut self, source: &str) -> Result<u8, ImportError> {
        Ok(self.array::<1>(source)?[0])
    }

    fn array<const N: usize>(&mut self, source: &str) -> Result<[u8; N], ImportError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| invalid(source, "AssetBundle cursor overflow"))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid(source, "AssetBundle metadata is truncated"))?;
        self.position = end;
        Ok(bytes.try_into().expect("fixed AssetBundle field"))
    }

    fn align(&mut self, source: &str) -> Result<(), ImportError> {
        let padding = (4 - self.position % 4) % 4;
        let end = self.position + padding;
        if end > self.bytes.len() {
            return Err(invalid(source, "AssetBundle alignment is truncated"));
        }
        self.position = end;
        Ok(())
    }
}

fn invalid(source: &str, message: impl Into<String>) -> ImportError {
    ImportError::invalid(source, message.into())
}
