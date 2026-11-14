use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub const INSTALL_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutStatus {
    RecognizedPc,
    IncompletePc,
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildSupport {
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceRuntime {
    OriginalOrUnknown,
    MemoriaModified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceRole {
    Launcher,
    Executable,
    ManagedReference,
    AssetArchive,
    AssetDirectory,
    ModConfiguration,
    ModComponent,
    PlatformIntegration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceStatus {
    Present,
    Missing,
    WrongKind,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingObservation {
    Utf8,
    NonUtf8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub resource_key: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInspection {
    pub key: String,
    pub relative_path: String,
    pub role: ResourceRole,
    pub expected_kind: ResourceKind,
    pub required_for_layout: bool,
    pub status: ResourceStatus,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub encoding: Option<EncodingObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingDependency {
    pub resource_key: String,
    pub relative_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildEvidence {
    pub edition: Option<String>,
    pub support: BuildSupport,
    pub reference_runtime: ReferenceRuntime,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallReport {
    pub schema_version: u32,
    pub root: String,
    pub layout: LayoutStatus,
    pub build: BuildEvidence,
    pub resources: Vec<ResourceInspection>,
    pub missing_dependencies: Vec<MissingDependency>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub enum ImportError {
    RootNotFound(PathBuf),
    RootNotDirectory(PathBuf),
    RootUnavailable {
        path: PathBuf,
        source: io::Error,
    },
    Cancelled {
        cache: PathBuf,
    },
    Invalid {
        resource: String,
        message: String,
    },
    Unsupported {
        resource: String,
        format: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl ImportError {
    pub fn invalid(resource: impl Display, message: impl Display) -> Self {
        Self::Invalid {
            resource: resource.to_string(),
            message: message.to_string(),
        }
    }

    pub fn unsupported(resource: impl Display, format: impl Display) -> Self {
        Self::Unsupported {
            resource: resource.to_string(),
            format: format.to_string(),
        }
    }

    pub fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl Display for ImportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootNotFound(path) => {
                write!(
                    formatter,
                    "installation root does not exist: {}",
                    path.display()
                )
            }
            Self::RootNotDirectory(path) => write!(
                formatter,
                "installation root is not a directory: {}",
                path.display()
            ),
            Self::RootUnavailable { path, source } => write!(
                formatter,
                "installation root cannot be inspected: {}: {source}",
                path.display()
            ),
            Self::Cancelled { cache } => {
                write!(
                    formatter,
                    "import cancelled before publishing to {}",
                    cache.display()
                )
            }
            Self::Invalid { resource, message } => {
                write!(formatter, "invalid import resource `{resource}`: {message}")
            }
            Self::Unsupported { resource, format } => {
                write!(formatter, "unsupported format `{format}` for `{resource}`")
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "{operation} failed for {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for ImportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RootUnavailable { source, .. } | Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ResourceSpec {
    key: &'static str,
    relative_path: &'static str,
    role: ResourceRole,
    kind: ResourceKind,
    required: bool,
}

const RESOURCES: &[ResourceSpec] = &[
    ResourceSpec {
        key: "launcher",
        relative_path: "FF9_Launcher.exe",
        role: ResourceRole::Launcher,
        kind: ResourceKind::File,
        required: true,
    },
    ResourceSpec {
        key: "x64_executable",
        relative_path: "x64/FF9.exe",
        role: ResourceRole::Executable,
        kind: ResourceKind::File,
        required: true,
    },
    ResourceSpec {
        key: "managed_game_reference",
        relative_path: "x64/FF9_Data/Managed/Assembly-CSharp.dll",
        role: ResourceRole::ManagedReference,
        kind: ResourceKind::File,
        required: true,
    },
    ResourceSpec {
        key: "resources_assets",
        relative_path: "x64/FF9_Data/resources.assets",
        role: ResourceRole::AssetArchive,
        kind: ResourceKind::File,
        required: true,
    },
    ResourceSpec {
        key: "shared_ui_assets",
        relative_path: "x64/FF9_Data/sharedassets2.assets",
        role: ResourceRole::AssetArchive,
        kind: ResourceKind::File,
        required: true,
    },
    ResourceSpec {
        key: "streaming_assets",
        relative_path: "StreamingAssets",
        role: ResourceRole::AssetDirectory,
        kind: ResourceKind::Directory,
        required: true,
    },
    ResourceSpec {
        key: "memoria_configuration",
        relative_path: "Memoria.ini",
        role: ResourceRole::ModConfiguration,
        kind: ResourceKind::File,
        required: false,
    },
    ResourceSpec {
        key: "memoria_prime",
        relative_path: "x64/FF9_Data/Managed/Memoria.Prime.dll",
        role: ResourceRole::ModComponent,
        kind: ResourceKind::File,
        required: false,
    },
    ResourceSpec {
        key: "memoria_special_effects",
        relative_path: "x64/FF9_Data/Plugins/FF9SpecialEffectPlugin.dll",
        role: ResourceRole::ModComponent,
        kind: ResourceKind::File,
        required: false,
    },
    ResourceSpec {
        key: "steam_integration",
        relative_path: "x64/steam_api64.dll",
        role: ResourceRole::PlatformIntegration,
        kind: ResourceKind::File,
        required: false,
    },
];

pub fn inspect_install(root: &Path) -> Result<InstallReport, ImportError> {
    let root_metadata = fs::metadata(root).map_err(|source| match source.kind() {
        io::ErrorKind::NotFound => ImportError::RootNotFound(root.to_path_buf()),
        _ => ImportError::RootUnavailable {
            path: root.to_path_buf(),
            source,
        },
    })?;
    if !root_metadata.is_dir() {
        return Err(ImportError::RootNotDirectory(root.to_path_buf()));
    }

    let mut resources = Vec::with_capacity(RESOURCES.len());
    let mut missing_dependencies = Vec::new();
    let mut diagnostics = Vec::new();

    for spec in RESOURCES {
        let resource = inspect_resource(root, *spec, &mut diagnostics);
        if spec.required && resource.status != ResourceStatus::Present {
            missing_dependencies.push(MissingDependency {
                resource_key: spec.key.to_owned(),
                relative_path: spec.relative_path.to_owned(),
                reason: match resource.status {
                    ResourceStatus::Missing => "required resource is absent".to_owned(),
                    ResourceStatus::WrongKind => {
                        "required resource has the wrong file kind".to_owned()
                    }
                    ResourceStatus::Unavailable => "required resource could not be read".to_owned(),
                    ResourceStatus::Present => unreachable!(),
                },
            });
        }
        resources.push(resource);
    }

    let required_present = resources
        .iter()
        .filter(|resource| resource.required_for_layout)
        .filter(|resource| resource.status == ResourceStatus::Present)
        .count();
    let required_total = RESOURCES.iter().filter(|spec| spec.required).count();
    let layout = if required_present == required_total {
        LayoutStatus::RecognizedPc
    } else if required_present == 0 {
        LayoutStatus::Unrecognized
    } else {
        LayoutStatus::IncompletePc
    };

    let memoria_detected = resources.iter().any(|resource| {
        matches!(
            resource.key.as_str(),
            "memoria_prime" | "memoria_special_effects"
        ) && resource.status == ResourceStatus::Present
    });
    let evidence = resources
        .iter()
        .filter(|resource| resource.status == ResourceStatus::Present)
        .map(|resource| format!("{}:{}", resource.key, resource.relative_path))
        .collect();

    if layout == LayoutStatus::RecognizedPc {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Info,
            code: "build-support-unknown".to_owned(),
            message:
                "PC layout recognized, but no supported-build hash catalog has been established"
                    .to_owned(),
            resource_key: None,
            path: root.to_string_lossy().into_owned(),
        });
    }

    Ok(InstallReport {
        schema_version: INSTALL_REPORT_SCHEMA_VERSION,
        root: root.to_string_lossy().into_owned(),
        layout,
        build: BuildEvidence {
            edition: None,
            support: BuildSupport::Unknown,
            reference_runtime: if memoria_detected {
                ReferenceRuntime::MemoriaModified
            } else {
                ReferenceRuntime::OriginalOrUnknown
            },
            evidence,
        },
        resources,
        missing_dependencies,
        diagnostics,
    })
}

fn inspect_resource(
    root: &Path,
    spec: ResourceSpec,
    diagnostics: &mut Vec<Diagnostic>,
) -> ResourceInspection {
    let path = root.join(spec.relative_path);
    let mut result = ResourceInspection {
        key: spec.key.to_owned(),
        relative_path: spec.relative_path.to_owned(),
        role: spec.role,
        expected_kind: spec.kind,
        required_for_layout: spec.required,
        status: ResourceStatus::Missing,
        size_bytes: None,
        sha256: None,
        encoding: None,
    };

    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(source) => {
            if source.kind() == io::ErrorKind::NotFound {
                result.status = ResourceStatus::Missing;
            } else {
                result.status = ResourceStatus::Unavailable;
                diagnostics.push(resource_io_diagnostic(&spec, &path, "metadata", &source));
            }
            return result;
        }
    };

    let kind_matches = match spec.kind {
        ResourceKind::File => metadata.is_file(),
        ResourceKind::Directory => metadata.is_dir(),
    };
    if !kind_matches {
        result.status = ResourceStatus::WrongKind;
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: "resource-kind-mismatch".to_owned(),
            message: format!(
                "expected {:?}, found a different filesystem object",
                spec.kind
            ),
            resource_key: Some(spec.key.to_owned()),
            path: path.to_string_lossy().into_owned(),
        });
        return result;
    }

    result.status = ResourceStatus::Present;
    if spec.kind == ResourceKind::Directory {
        return result;
    }

    result.size_bytes = Some(metadata.len());
    match hash_file(&path) {
        Ok(hash) => result.sha256 = Some(hash),
        Err(source) => {
            result.status = ResourceStatus::Unavailable;
            diagnostics.push(resource_io_diagnostic(
                &spec,
                &path,
                "SHA-256 hashing",
                &source,
            ));
            return result;
        }
    }

    if is_text_resource(&path) {
        match fs::read(&path) {
            Ok(bytes) => {
                result.encoding = Some(if std::str::from_utf8(&bytes).is_ok() {
                    EncodingObservation::Utf8
                } else {
                    EncodingObservation::NonUtf8
                });
            }
            Err(source) => {
                result.status = ResourceStatus::Unavailable;
                diagnostics.push(resource_io_diagnostic(
                    &spec,
                    &path,
                    "encoding inspection",
                    &source,
                ));
            }
        }
    }

    result
}

fn resource_io_diagnostic(
    spec: &ResourceSpec,
    path: &Path,
    operation: &str,
    source: &io::Error,
) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "resource-unavailable".to_owned(),
        message: format!("{operation} failed for {}: {source}", spec.key),
        resource_key: Some(spec.key.to_owned()),
        path: path.to_string_lossy().into_owned(),
    }
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn is_text_resource(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["ini", "txt", "json", "xml", "csv"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_diagnostic_preserves_resource_operation_and_path() {
        let spec = ResourceSpec {
            key: "launcher",
            relative_path: "FF9_Launcher.exe",
            role: ResourceRole::Launcher,
            kind: ResourceKind::File,
            required: true,
        };
        let path = Path::new("C:\\Games\\FFIX\\FF9_Launcher.exe");
        let error = io::Error::new(io::ErrorKind::PermissionDenied, "fixture permission denied");

        let diagnostic = resource_io_diagnostic(&spec, path, "SHA-256 hashing", &error);

        assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostic.code, "resource-unavailable");
        assert_eq!(diagnostic.resource_key.as_deref(), Some("launcher"));
        assert!(diagnostic.message.contains("SHA-256 hashing"));
        assert!(diagnostic.message.contains("fixture permission denied"));
        assert!(diagnostic.path.contains("FF9_Launcher.exe"));
    }
}
