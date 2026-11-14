use crate::canonical::PackManifest;
use crate::install::ImportError;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn check(&self, cache: &Path) -> Result<(), ImportError> {
        if self.is_cancelled() {
            Err(ImportError::Cancelled {
                cache: cache.to_path_buf(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStore {
    root: PathBuf,
}

impl CacheStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn write_object(
        &self,
        hash: &str,
        bytes: &[u8],
        cancel: &CancellationToken,
    ) -> Result<PathBuf, ImportError> {
        validate_hash(hash, "object hash")?;
        cancel.check(&self.root)?;
        let objects = self.root.join("objects");
        fs::create_dir_all(&objects)
            .map_err(|source| ImportError::io("create object cache", &objects, source))?;
        let path = objects.join(format!("{hash}.bin"));
        if path.exists() {
            let existing = fs::read(&path)
                .map_err(|source| ImportError::io("read cached object", &path, source))?;
            if sha256_bytes(&existing) != hash {
                return Err(ImportError::invalid(
                    path.display(),
                    "existing content-addressed object hash does not match its filename",
                ));
            }
            return Ok(path);
        }

        let temporary = tempfile::Builder::new()
            .prefix("object-")
            .tempfile_in(&objects)
            .map_err(|source| ImportError::io("create temporary object", &objects, source))?;
        temporary
            .as_file()
            .write_all(bytes)
            .map_err(|source| ImportError::io("write object", temporary.path(), source))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| ImportError::io("flush object", temporary.path(), source))?;
        cancel.check(&self.root)?;
        match temporary.persist(&path) {
            Ok(_) => Ok(path),
            Err(error) if path.exists() => {
                drop(error);
                let existing = fs::read(&path)
                    .map_err(|source| ImportError::io("read raced object", &path, source))?;
                if sha256_bytes(&existing) == hash {
                    Ok(path)
                } else {
                    Err(ImportError::invalid(
                        path.display(),
                        "raced object has unexpected content",
                    ))
                }
            }
            Err(error) => Err(ImportError::io("publish object", &path, error.error)),
        }
    }

    pub fn publish(
        &self,
        manifest: &PackManifest,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, ImportError> {
        validate_hash(&manifest.pack_id, "pack ID")?;
        cancel.check(&self.root)?;
        let revisions = self.root.join("revisions");
        let staging = self.root.join("staging");
        fs::create_dir_all(&revisions)
            .map_err(|source| ImportError::io("create revision cache", &revisions, source))?;
        fs::create_dir_all(&staging)
            .map_err(|source| ImportError::io("create staging cache", &staging, source))?;
        let destination = revisions.join(&manifest.pack_id);
        let encoded = serde_json::to_vec_pretty(manifest)
            .map_err(|error| ImportError::invalid(&manifest.pack_id, error))?;

        if destination.exists() {
            return self.verify_existing_revision(&destination, &encoded);
        }

        let temporary = tempfile::Builder::new()
            .prefix("revision-")
            .tempdir_in(&staging)
            .map_err(|source| ImportError::io("create staged revision", &staging, source))?;
        let manifest_path = temporary.path().join("manifest.json");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest_path)
            .map_err(|source| ImportError::io("create pack manifest", &manifest_path, source))?;
        file.write_all(&encoded)
            .map_err(|source| ImportError::io("write pack manifest", &manifest_path, source))?;
        file.sync_all()
            .map_err(|source| ImportError::io("flush pack manifest", &manifest_path, source))?;
        drop(file);
        cancel.check(&self.root)?;

        let staged_path = temporary.keep();
        if let Err(source) = fs::rename(&staged_path, &destination) {
            let cleanup_result = fs::remove_dir_all(&staged_path);
            if destination.exists() {
                self.verify_existing_revision(&destination, &encoded)?;
                return Ok(destination);
            }
            if let Err(cleanup_error) = cleanup_result {
                return Err(ImportError::invalid(
                    staged_path.display(),
                    format!(
                        "revision publication failed ({source}); staging cleanup also failed ({cleanup_error})"
                    ),
                ));
            }
            return Err(ImportError::io(
                "publish pack revision",
                &destination,
                source,
            ));
        }
        Ok(destination)
    }

    pub fn load(&self, pack_id: &str) -> Result<PackManifest, ImportError> {
        validate_hash(pack_id, "pack ID")?;
        let path = self
            .root
            .join("revisions")
            .join(pack_id)
            .join("manifest.json");
        let bytes = fs::read(&path)
            .map_err(|source| ImportError::io("read pack manifest", &path, source))?;
        let manifest: PackManifest = serde_json::from_slice(&bytes)
            .map_err(|error| ImportError::invalid(path.display(), error))?;
        if manifest.pack_id != pack_id {
            return Err(ImportError::invalid(
                path.display(),
                "manifest pack ID does not match its revision directory",
            ));
        }
        for asset in &manifest.assets {
            validate_hash(&asset.sha256, "asset object hash")?;
            let expected_object = format!("objects/{}.bin", asset.sha256);
            if asset.object != expected_object {
                return Err(ImportError::invalid(
                    path.display(),
                    format!("asset `{}` has an invalid object path", asset.id),
                ));
            }
            let object = self.root.join(&asset.object);
            let mut file = fs::File::open(&object)
                .map_err(|source| ImportError::io("open cached asset", &object, source))?;
            let mut hasher = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = file
                    .read(&mut buffer)
                    .map_err(|source| ImportError::io("read cached asset", &object, source))?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            if format!("{:x}", hasher.finalize()) != asset.sha256 {
                return Err(ImportError::invalid(
                    object.display(),
                    format!("cached object does not match asset `{}`", asset.id),
                ));
            }
        }
        Ok(manifest)
    }

    pub fn revisions(&self) -> Result<Vec<String>, ImportError> {
        let path = self.root.join("revisions");
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(ImportError::io("list cache revisions", &path, source)),
        };
        let mut revisions = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|source| ImportError::io("read revision entry", &path, source))?;
            if entry
                .file_type()
                .map_err(|source| ImportError::io("inspect revision entry", &path, source))?
                .is_dir()
            {
                revisions.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        revisions.sort();
        Ok(revisions)
    }

    fn verify_existing_revision(
        &self,
        destination: &Path,
        expected: &[u8],
    ) -> Result<PathBuf, ImportError> {
        let manifest_path = destination.join("manifest.json");
        let actual = fs::read(&manifest_path)
            .map_err(|source| ImportError::io("read existing revision", &manifest_path, source))?;
        if actual != expected {
            return Err(ImportError::invalid(
                manifest_path.display(),
                "existing revision manifest differs despite having the same pack ID",
            ));
        }
        Ok(destination.to_path_buf())
    }
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_hash(hash: &str, resource: &str) -> Result<(), ImportError> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ImportError::invalid(
            resource,
            "expected 64 lowercase hexadecimal SHA-256 characters",
        ));
    }
    Ok(())
}
