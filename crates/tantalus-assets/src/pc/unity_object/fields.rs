//! Typed field access over decoded type-tree values.

use super::type_tree::Value;
use crate::install::ImportError;
use serde::{Deserialize, Serialize};

/// Unity object reference. `file_id` 0 means the same serialized file;
/// `path_id` 0 is a null reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PPtr {
    pub file_id: i32,
    pub path_id: i64,
}

impl PPtr {
    pub fn is_null(&self) -> bool {
        self.path_id == 0
    }

    pub fn is_local(&self) -> bool {
        self.file_id == 0 && self.path_id != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub center: [f32; 3],
    pub extent: [f32; 3],
}

pub(crate) struct Fields<'v, 't> {
    pub value: &'v Value<'t>,
    pub source: &'v str,
    pub context: &'static str,
}

impl<'v, 't> Fields<'v, 't> {
    pub fn new(value: &'v Value<'t>, source: &'v str, context: &'static str) -> Self {
        Self {
            value,
            source,
            context,
        }
    }

    pub fn error(&self, message: impl std::fmt::Display) -> ImportError {
        ImportError::invalid(self.source, format!("{}: {message}", self.context))
    }

    pub fn get(&self, name: &str) -> Result<&'v Value<'t>, ImportError> {
        self.value
            .field(name)
            .ok_or_else(|| self.error(format!("missing field `{name}`")))
    }

    pub fn sub(&self, name: &str) -> Result<Fields<'v, 't>, ImportError> {
        Ok(Fields::new(self.get(name)?, self.source, self.context))
    }

    pub fn u64(&self, name: &str) -> Result<u64, ImportError> {
        self.get(name)?
            .as_u64()
            .ok_or_else(|| self.error(format!("`{name}` is not an unsigned integer")))
    }

    pub fn u32(&self, name: &str) -> Result<u32, ImportError> {
        u32::try_from(self.u64(name)?).map_err(|_| self.error(format!("`{name}` exceeds 32 bits")))
    }

    pub fn i64(&self, name: &str) -> Result<i64, ImportError> {
        self.get(name)?
            .as_i64()
            .ok_or_else(|| self.error(format!("`{name}` is not an integer")))
    }

    pub fn i32(&self, name: &str) -> Result<i32, ImportError> {
        i32::try_from(self.i64(name)?).map_err(|_| self.error(format!("`{name}` exceeds 32 bits")))
    }

    pub fn f32(&self, name: &str) -> Result<f32, ImportError> {
        self.get(name)?
            .as_f32()
            .ok_or_else(|| self.error(format!("`{name}` is not a float")))
    }

    pub fn bool(&self, name: &str) -> Result<bool, ImportError> {
        self.get(name)?
            .as_bool()
            .ok_or_else(|| self.error(format!("`{name}` is not a bool")))
    }

    pub fn string(&self, name: &str) -> Result<String, ImportError> {
        self.get(name)?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| self.error(format!("`{name}` is not a string")))
    }

    pub fn bytes(&self, name: &str) -> Result<&'v [u8], ImportError> {
        self.get(name)?
            .as_bytes()
            .ok_or_else(|| self.error(format!("`{name}` is not a byte array")))
    }

    pub fn u32s(&self, name: &str) -> Result<Vec<u32>, ImportError> {
        match self.get(name)? {
            Value::U32s(values) => Ok(values.clone()),
            Value::Array(values) if values.is_empty() => Ok(Vec::new()),
            _ => Err(self.error(format!("`{name}` is not a u32 array"))),
        }
    }

    pub fn f32s(&self, name: &str) -> Result<Vec<f32>, ImportError> {
        match self.get(name)? {
            Value::F32s(values) => Ok(values.clone()),
            _ => Err(self.error(format!("`{name}` is not a float array"))),
        }
    }

    pub fn array(&self, name: &str) -> Result<&'v [Value<'t>], ImportError> {
        self.get(name)?
            .as_array()
            .ok_or_else(|| self.error(format!("`{name}` is not a struct array")))
    }

    pub fn len(&self, name: &str) -> Result<usize, ImportError> {
        self.get(name)?
            .array_len()
            .ok_or_else(|| self.error(format!("`{name}` is not an array")))
    }

    pub fn pptr(&self, name: &str) -> Result<PPtr, ImportError> {
        let sub = self.sub(name)?;
        Ok(PPtr {
            file_id: sub.i32("m_FileID")?,
            path_id: sub.i64("m_PathID")?,
        })
    }

    pub fn pptrs(&self, name: &str) -> Result<Vec<PPtr>, ImportError> {
        self.array(name)?
            .iter()
            .map(|value| {
                let sub = Fields::new(value, self.source, self.context);
                Ok(PPtr {
                    file_id: sub.i32("m_FileID")?,
                    path_id: sub.i64("m_PathID")?,
                })
            })
            .collect()
    }

    pub fn floats<const N: usize>(
        &self,
        name: &str,
        keys: [&str; N],
    ) -> Result<[f32; N], ImportError> {
        let sub = self.sub(name)?;
        let mut out = [0.0; N];
        for (slot, key) in out.iter_mut().zip(keys) {
            *slot = sub.f32(key)?;
        }
        Ok(out)
    }

    pub fn vec3(&self, name: &str) -> Result<[f32; 3], ImportError> {
        self.floats(name, ["x", "y", "z"])
    }

    pub fn quat(&self, name: &str) -> Result<[f32; 4], ImportError> {
        self.floats(name, ["x", "y", "z", "w"])
    }

    pub fn aabb(&self, name: &str) -> Result<Aabb, ImportError> {
        let sub = self.sub(name)?;
        Ok(Aabb {
            center: sub.vec3("m_Center")?,
            extent: sub.vec3("m_Extent")?,
        })
    }
}
