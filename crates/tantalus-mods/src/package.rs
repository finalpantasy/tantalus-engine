use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tantalus_contracts::{ContentId, ValidationIssue, ValidationReport};
use tantalus_core::{RegistryPatch, RegistryRecord};

pub const PACKAGE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDependency {
    pub package: String,
    pub version_range: String,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFeature {
    pub name: String,
    #[serde(default)]
    pub dependencies: Vec<PackageDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDeclaration {
    pub service: ContentId,
    #[serde(default)]
    pub exclusive: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldPackage {
    pub spawn_regions: Vec<ContentId>,
}

fn any_version() -> String {
    "*".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub schema_version: u32,
    pub package_id: String,
    pub version: String,
    #[serde(default = "any_version")]
    pub engine_range: String,
    #[serde(default = "any_version")]
    pub sdk_range: String,
    #[serde(default)]
    pub save_schema: u32,
    #[serde(default)]
    pub dependencies: Vec<PackageDependency>,
    #[serde(default)]
    pub features: Vec<PackageFeature>,
    #[serde(default)]
    pub providers: Vec<ProviderDeclaration>,
    #[serde(default)]
    pub entrypoints: Vec<ModuleEntrypoint>,
    pub world: WorldPackage,
    pub records: Vec<PackageRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleEntrypoint {
    pub name: String,
    pub source_path: String,
    pub abi_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedModule {
    pub name: String,
    pub source_path: String,
    pub abi_version: i32,
    pub wasm: Vec<u8>,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRecord {
    pub id: ContentId,
    pub source_path: String,
    pub operation: PackageOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOperation {
    Add,
    Replace,
}

#[derive(Debug, Clone)]
pub struct StagedPackage {
    pub manifest: PackageManifest,
    pub patch: RegistryPatch,
    pub modules: Vec<StagedModule>,
    pub content_hash: String,
}

pub struct PackageLoader;

impl PackageLoader {
    pub fn stage(path: &Path) -> Result<StagedPackage, ValidationReport> {
        let mut report = ValidationReport::default();
        let manifest_path = path.join("tantalus-package.json");
        let manifest: PackageManifest = match read_json(&manifest_path) {
            Ok(manifest) => manifest,
            Err(issue) => {
                report.push(issue);
                return Err(report);
            }
        };

        if manifest.schema_version != PACKAGE_SCHEMA_VERSION {
            report.push(issue(
                "unsupported-package-schema",
                format!(
                    "expected package schema {PACKAGE_SCHEMA_VERSION}, found {}",
                    manifest.schema_version
                ),
                None,
                &manifest_path,
            ));
        }
        if manifest.package_id.is_empty() || manifest.version.is_empty() {
            report.push(issue(
                "invalid-package-identity",
                "package ID and version must be nonempty",
                None,
                &manifest_path,
            ));
        }
        if manifest.records.is_empty() && manifest.entrypoints.is_empty() {
            report.push(issue(
                "empty-package",
                "package must contain at least one content record or module entrypoint",
                None,
                &manifest_path,
            ));
        }

        let canonical_root = match fs::canonicalize(path) {
            Ok(root) => root,
            Err(error) => {
                report.push(issue(
                    "package-root-unavailable",
                    error.to_string(),
                    None,
                    path,
                ));
                return Err(report);
            }
        };

        let mut patch = RegistryPatch::default();
        let mut identities = BTreeSet::new();
        for record in &manifest.records {
            if !identities.insert(record.id.clone()) {
                report.push(issue(
                    "duplicate-package-content",
                    format!("content `{}` is declared more than once", record.id),
                    Some(record.id.clone()),
                    path,
                ));
                continue;
            }
            let relative = match safe_relative(&record.source_path) {
                Ok(relative) => relative,
                Err(message) => {
                    report.push(issue(
                        "invalid-package-path",
                        message,
                        Some(record.id.clone()),
                        path,
                    ));
                    continue;
                }
            };
            let source_path = canonical_root.join(relative);
            let canonical_source = match fs::canonicalize(&source_path) {
                Ok(source) if source.starts_with(&canonical_root) => source,
                Ok(_) => {
                    report.push(issue(
                        "package-path-escape",
                        "resource escapes the package root",
                        Some(record.id.clone()),
                        &source_path,
                    ));
                    continue;
                }
                Err(error) => {
                    report.push(issue(
                        "package-resource-unavailable",
                        error.to_string(),
                        Some(record.id.clone()),
                        &source_path,
                    ));
                    continue;
                }
            };
            let data = match read_json(&canonical_source) {
                Ok(data) => data,
                Err(issue_value) => {
                    report.push(issue_value);
                    continue;
                }
            };
            let registry_record = RegistryRecord::new(record.id.clone(), data)
                .with_source(manifest.package_id.clone(), record.source_path.clone());
            match record.operation {
                PackageOperation::Add => patch.additions.push(registry_record),
                PackageOperation::Replace => patch.replacements.push(registry_record),
            }
        }

        let mut modules = Vec::new();
        let mut entrypoint_names = BTreeSet::new();
        for entrypoint in &manifest.entrypoints {
            if entrypoint.name.is_empty()
                || !entrypoint_names.insert(entrypoint.name.clone())
                || entrypoint.abi_version != crate::wasm_host::MODULE_ABI_VERSION
            {
                report.push(issue(
                    "invalid-entrypoint",
                    "entrypoint names must be nonempty/unique and use the supported ABI",
                    None,
                    path,
                ));
                continue;
            }
            let relative = match safe_relative(&entrypoint.source_path) {
                Ok(relative) => relative,
                Err(message) => {
                    report.push(issue("invalid-package-path", message, None, path));
                    continue;
                }
            };
            let source_path = canonical_root.join(relative);
            let canonical_source = match fs::canonicalize(&source_path) {
                Ok(source) if source.starts_with(&canonical_root) => source,
                Ok(_) => {
                    report.push(issue(
                        "package-path-escape",
                        "module escapes the package root",
                        None,
                        &source_path,
                    ));
                    continue;
                }
                Err(error) => {
                    report.push(issue(
                        "package-resource-unavailable",
                        error.to_string(),
                        None,
                        &source_path,
                    ));
                    continue;
                }
            };
            let length = fs::metadata(&canonical_source)
                .map(|metadata| metadata.len())
                .unwrap_or(u64::MAX);
            if length > 16 * 1024 * 1024 {
                report.push(issue(
                    "module-too-large",
                    "WASM entrypoint exceeds the 16 MiB staging limit",
                    None,
                    &canonical_source,
                ));
                continue;
            }
            let wasm = match fs::read(&canonical_source) {
                Ok(wasm) if wasm.starts_with(b"\0asm") => wasm,
                Ok(_) => {
                    report.push(issue(
                        "invalid-wasm",
                        "module does not have a WASM binary header",
                        None,
                        &canonical_source,
                    ));
                    continue;
                }
                Err(error) => {
                    report.push(issue(
                        "package-resource-unavailable",
                        error.to_string(),
                        None,
                        &canonical_source,
                    ));
                    continue;
                }
            };
            let sha256 = format!("{:x}", Sha256::digest(&wasm));
            modules.push(StagedModule {
                name: entrypoint.name.clone(),
                source_path: entrypoint.source_path.clone(),
                abi_version: entrypoint.abi_version,
                wasm,
                sha256,
            });
        }

        if !report.is_valid() {
            return Err(report);
        }

        let mut content_records: Vec<(PackageOperation, RegistryRecord)> = patch
            .additions
            .iter()
            .cloned()
            .map(|record| (PackageOperation::Add, record))
            .chain(
                patch
                    .replacements
                    .iter()
                    .cloned()
                    .map(|record| (PackageOperation::Replace, record)),
            )
            .collect();
        content_records.sort_by(|left, right| left.1.id.cmp(&right.1.id));
        let mut module_identities = modules
            .iter()
            .map(|module| (&module.name, &module.sha256, module.abi_version))
            .collect::<Vec<_>>();
        module_identities.sort_by_key(|(name, _, _)| name.as_str());
        let content_bytes = serde_json::to_vec(&(content_records, module_identities))
            .expect("serializing validated content and module identities cannot fail");
        let content_hash = format!("{:x}", Sha256::digest(content_bytes));
        Ok(StagedPackage {
            manifest,
            patch,
            modules,
            content_hash,
        })
    }
}

fn read_json<T>(path: &Path) -> Result<T, ValidationIssue>
where
    T: serde::de::DeserializeOwned,
{
    let bytes = fs::read(path)
        .map_err(|error| issue("resource-read-failed", error.to_string(), None, path))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| issue("resource-json-invalid", error.to_string(), None, path))
}

fn safe_relative(text: &str) -> Result<PathBuf, String> {
    let path = Path::new(text);
    if text.is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(
            "package paths must be relative and contain no root, prefix, or parent components"
                .to_owned(),
        );
    }
    Ok(path.to_path_buf())
}

fn issue(
    code: &str,
    message: impl Into<String>,
    content_id: Option<ContentId>,
    path: &Path,
) -> ValidationIssue {
    ValidationIssue::error(
        code,
        message,
        content_id,
        Some(path.to_string_lossy().into_owned()),
    )
}
