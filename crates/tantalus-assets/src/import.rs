use crate::cache::{CacheStore, CancellationToken, sha256_bytes};
use crate::canonical::{
    AssetCodec, AssetRequest, CONVERTER_VERSION, ImportPlan, PACK_SCHEMA_VERSION, PackAsset,
    PackManifest,
};
use crate::install::ImportError;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub trait AssetSource {
    fn read(&self, request: &AssetRequest, max_bytes: u64) -> Result<Vec<u8>, ImportError>;
}

#[derive(Debug, Clone)]
pub struct DirectoryAssetSource {
    root: PathBuf,
}

impl DirectoryAssetSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl AssetSource for DirectoryAssetSource {
    fn read(&self, request: &AssetRequest, max_bytes: u64) -> Result<Vec<u8>, ImportError> {
        let relative = safe_relative_path(&request.source_path)?;
        let canonical_root = fs::canonicalize(&self.root)
            .map_err(|source| ImportError::io("resolve source root", &self.root, source))?;
        let path = canonical_root.join(relative);
        let canonical_path = fs::canonicalize(&path)
            .map_err(|source| ImportError::io("resolve source resource", &path, source))?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(ImportError::invalid(
                &request.source_path,
                "resource escapes the selected source root",
            ));
        }
        let metadata = fs::metadata(&canonical_path).map_err(|source| {
            ImportError::io("inspect source resource", &canonical_path, source)
        })?;
        if !metadata.is_file() {
            return Err(ImportError::invalid(
                &request.source_path,
                "source resource is not a regular file",
            ));
        }
        if metadata.len() > max_bytes {
            return Err(ImportError::invalid(
                &request.source_path,
                format!("resource exceeds the {max_bytes}-byte file limit"),
            ));
        }
        let bytes = fs::read(&canonical_path)
            .map_err(|source| ImportError::io("read source resource", &canonical_path, source))?;
        if bytes.len() as u64 > max_bytes {
            return Err(ImportError::invalid(
                &request.source_path,
                "resource grew beyond the file limit while being read",
            ));
        }
        Ok(bytes)
    }
}

pub fn import_subset(
    plan: &ImportPlan,
    source: &dyn AssetSource,
    cache: &Path,
    cancel: &CancellationToken,
) -> Result<PackManifest, ImportError> {
    validate_plan(plan)?;
    let mut requests = plan.requests.clone();
    requests.sort_by(|left, right| left.id.cmp(&right.id));

    let store = CacheStore::new(cache);
    let mut assets = Vec::with_capacity(requests.len());
    let mut identities = BTreeSet::new();
    let mut total_bytes = 0_u64;

    for request in requests {
        if cancel.is_cancelled() {
            return Err(ImportError::Cancelled {
                cache: cache.to_path_buf(),
            });
        }
        if !identities.insert(request.id.clone()) {
            return Err(ImportError::invalid(
                request.id.as_str(),
                "duplicate content identity in import plan",
            ));
        }
        let bytes = source.read(&request, plan.limits.max_file_bytes)?;
        total_bytes = total_bytes.checked_add(bytes.len() as u64).ok_or_else(|| {
            ImportError::invalid(request.id.as_str(), "total import size overflow")
        })?;
        if total_bytes > plan.limits.max_total_bytes {
            return Err(ImportError::invalid(
                request.id.as_str(),
                format!(
                    "import exceeds the {}-byte total limit",
                    plan.limits.max_total_bytes
                ),
            ));
        }
        if request.codec == AssetCodec::Utf8 && std::str::from_utf8(&bytes).is_err() {
            return Err(ImportError::invalid(
                request.id.as_str(),
                format!("{} is not valid UTF-8", request.source_path),
            ));
        }

        let hash = sha256_bytes(&bytes);
        store.write_object(&hash, &bytes, cancel)?;
        assets.push(PackAsset {
            id: request.id,
            kind: request.kind,
            codec: request.codec,
            source_path: request.source_path,
            size_bytes: bytes.len() as u64,
            sha256: hash.clone(),
            object: format!("objects/{hash}.bin"),
            dependencies: request.dependencies,
        });
    }

    for asset in &assets {
        let mut dependencies = BTreeSet::new();
        for dependency in &asset.dependencies {
            if dependency == &asset.id {
                return Err(ImportError::invalid(
                    asset.id.as_str(),
                    "asset cannot depend on itself",
                ));
            }
            if !dependencies.insert(dependency.clone()) {
                return Err(ImportError::invalid(
                    asset.id.as_str(),
                    format!("duplicate dependency `{dependency}`"),
                ));
            }
            if !identities.contains(dependency) {
                return Err(ImportError::invalid(
                    asset.id.as_str(),
                    format!("dangling dependency `{dependency}`"),
                ));
            }
        }
    }

    #[derive(Serialize)]
    struct PackIdentity<'a> {
        schema_version: u32,
        converter_version: &'a str,
        source_id: &'a str,
        assets: &'a [PackAsset],
    }

    let identity = PackIdentity {
        schema_version: PACK_SCHEMA_VERSION,
        converter_version: CONVERTER_VERSION,
        source_id: &plan.source_id,
        assets: &assets,
    };
    let identity_bytes = serde_json::to_vec(&identity)
        .map_err(|error| ImportError::invalid("pack identity", error))?;
    let manifest = PackManifest {
        schema_version: PACK_SCHEMA_VERSION,
        converter_version: CONVERTER_VERSION.to_owned(),
        source_id: plan.source_id.clone(),
        pack_id: sha256_bytes(&identity_bytes),
        assets,
    };
    store.publish(&manifest, cancel)?;
    store.load(&manifest.pack_id)
}

fn validate_plan(plan: &ImportPlan) -> Result<(), ImportError> {
    if plan.target_schema != PACK_SCHEMA_VERSION {
        return Err(ImportError::unsupported(
            "import plan",
            format!("target schema {}", plan.target_schema),
        ));
    }
    if plan.source_id.is_empty() {
        return Err(ImportError::invalid(
            "source_id",
            "source ID cannot be empty",
        ));
    }
    if plan.requests.is_empty() {
        return Err(ImportError::invalid(
            "requests",
            "import plan must contain at least one asset",
        ));
    }
    if plan.limits.max_assets == 0
        || plan.limits.max_file_bytes == 0
        || plan.limits.max_total_bytes == 0
    {
        return Err(ImportError::invalid(
            "limits",
            "asset and byte limits must be nonzero",
        ));
    }
    if plan.requests.len() > plan.limits.max_assets {
        return Err(ImportError::invalid(
            "requests",
            format!("plan exceeds the {}-asset limit", plan.limits.max_assets),
        ));
    }
    Ok(())
}

fn safe_relative_path(text: &str) -> Result<PathBuf, ImportError> {
    let path = Path::new(text);
    if text.is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ImportError::invalid(
            text,
            "source paths must be nonempty relative paths without parent, root, or prefix components",
        ));
    }
    Ok(path.to_path_buf())
}
