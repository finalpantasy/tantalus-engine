use crate::{PackageDependency, PackageManifest, StagedPackage};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::{ContentId, ContentRevision, ModuleLock, ProfileLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageCandidate {
    pub manifest: PackageManifest,
    pub content_hash: String,
}

impl From<StagedPackage> for PackageCandidate {
    fn from(staged: StagedPackage) -> Self {
        Self {
            manifest: staged.manifest,
            content_hash: staged.content_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRequest {
    pub engine_version: String,
    pub sdk_version: String,
    pub base_revision: ContentRevision,
    pub available: Vec<PackageCandidate>,
    pub requested: Vec<String>,
    pub enabled_features: BTreeMap<String, Vec<String>>,
    pub load_order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProvider {
    pub service: ContentId,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProfile {
    pub lock: ProfileLock,
    pub providers: Vec<ResolvedProvider>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionErrorCode {
    InvalidRequest,
    UnknownPackage,
    VersionConflict,
    MissingDependency,
    DependencyCycle,
    IncompatibleEngine,
    IncompatibleSdk,
    InvalidFeature,
    DuplicateContent,
    ProviderConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionError {
    pub code: ResolutionErrorCode,
    pub package: Option<String>,
    pub context: String,
}

impl ResolutionError {
    fn new(code: ResolutionErrorCode, package: Option<String>, context: impl Into<String>) -> Self {
        Self {
            code,
            package,
            context: context.into(),
        }
    }
}

impl Display for ResolutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.package {
            Some(package) => write!(formatter, "mod resolution for {package}: {}", self.context),
            None => write!(formatter, "mod resolution: {}", self.context),
        }
    }
}

impl Error for ResolutionError {}

#[derive(Debug, Clone)]
struct PreparedCandidate {
    candidate: PackageCandidate,
    version: Version,
    engine_range: VersionReq,
    sdk_range: VersionReq,
}

#[derive(Debug, Clone)]
struct Requirement {
    package: String,
    range: VersionReq,
    range_text: String,
    required_by: String,
}

pub struct ModResolver;

impl ModResolver {
    pub fn resolve(request: &ProfileRequest) -> Result<ResolvedProfile, ResolutionError> {
        let engine = parse_version(&request.engine_version, None, "engine")?;
        let sdk = parse_version(&request.sdk_version, None, "SDK")?;
        let available = prepare_candidates(&request.available)?;
        validate_string_set(&request.requested, "requested package")?;
        validate_string_set(&request.load_order, "load-order entry")?;
        if request.requested.is_empty() {
            return Err(ResolutionError::new(
                ResolutionErrorCode::InvalidRequest,
                None,
                "at least one root package must be requested",
            ));
        }
        for package in request.enabled_features.keys() {
            if !available.contains_key(package) {
                return Err(ResolutionError::new(
                    ResolutionErrorCode::InvalidFeature,
                    Some(package.clone()),
                    "features were requested for an unavailable package",
                ));
            }
        }

        let requirements = request
            .requested
            .iter()
            .map(|package| Requirement {
                package: package.clone(),
                range: VersionReq::STAR,
                range_text: "*".to_owned(),
                required_by: "profile".to_owned(),
            })
            .collect();
        let selected = solve(
            &available,
            &request.enabled_features,
            &engine,
            &sdk,
            BTreeMap::new(),
            requirements,
        )?;
        let ordered = topological_order(&selected, &request.enabled_features, &request.load_order)?;
        let modules = locked_modules(&ordered, &selected, &request.enabled_features);
        let providers = resolve_providers(&ordered, &selected)?;
        validate_content_declarations(&ordered, &selected)?;
        let fingerprint = fingerprint_modules(request.base_revision, &modules);
        Ok(ResolvedProfile {
            lock: ProfileLock {
                content_revision: request.base_revision,
                content_fingerprint: fingerprint,
                modules,
            },
            providers,
        })
    }
}

fn prepare_candidates(
    candidates: &[PackageCandidate],
) -> Result<BTreeMap<String, Vec<PreparedCandidate>>, ResolutionError> {
    let mut available: BTreeMap<String, Vec<PreparedCandidate>> = BTreeMap::new();
    let mut identities = BTreeSet::new();
    for candidate in candidates {
        let manifest = &candidate.manifest;
        if manifest.package_id.is_empty() || candidate.content_hash.len() != 64 {
            return Err(ResolutionError::new(
                ResolutionErrorCode::InvalidRequest,
                Some(manifest.package_id.clone()),
                "package ID and a SHA-256 content hash are required",
            ));
        }
        let identity = (manifest.package_id.clone(), manifest.version.clone());
        if !identities.insert(identity) {
            return Err(ResolutionError::new(
                ResolutionErrorCode::VersionConflict,
                Some(manifest.package_id.clone()),
                format!("duplicate candidate version {}", manifest.version),
            ));
        }
        let mut feature_names = BTreeSet::new();
        for feature in &manifest.features {
            if feature.name.is_empty() || !feature_names.insert(&feature.name) {
                return Err(ResolutionError::new(
                    ResolutionErrorCode::InvalidFeature,
                    Some(manifest.package_id.clone()),
                    "feature names must be nonempty and unique",
                ));
            }
        }
        for dependency in manifest.dependencies.iter().chain(
            manifest
                .features
                .iter()
                .flat_map(|feature| feature.dependencies.iter()),
        ) {
            parse_range(
                &dependency.version_range,
                &manifest.package_id,
                "dependency",
            )?;
        }
        let prepared = PreparedCandidate {
            version: parse_version(&manifest.version, Some(&manifest.package_id), "package")?,
            engine_range: parse_range(
                &manifest.engine_range,
                &manifest.package_id,
                "engine compatibility",
            )?,
            sdk_range: parse_range(
                &manifest.sdk_range,
                &manifest.package_id,
                "SDK compatibility",
            )?,
            candidate: candidate.clone(),
        };
        available
            .entry(manifest.package_id.clone())
            .or_default()
            .push(prepared);
    }
    for versions in available.values_mut() {
        versions.sort_by(|left, right| right.version.cmp(&left.version));
    }
    Ok(available)
}

fn solve(
    available: &BTreeMap<String, Vec<PreparedCandidate>>,
    features: &BTreeMap<String, Vec<String>>,
    engine: &Version,
    sdk: &Version,
    selected: BTreeMap<String, PreparedCandidate>,
    mut requirements: Vec<Requirement>,
) -> Result<BTreeMap<String, PreparedCandidate>, ResolutionError> {
    if requirements.is_empty() {
        return Ok(selected);
    }
    let requirement = requirements.remove(0);
    if let Some(candidate) = selected.get(&requirement.package) {
        if requirement.range.matches(&candidate.version) {
            return solve(available, features, engine, sdk, selected, requirements);
        }
        return Err(ResolutionError::new(
            ResolutionErrorCode::VersionConflict,
            Some(requirement.package),
            format!(
                "{} requires {}, but {} was selected",
                requirement.required_by, requirement.range_text, candidate.version
            ),
        ));
    }

    let candidates = available.get(&requirement.package).ok_or_else(|| {
        let code = if requirement.required_by == "profile" {
            ResolutionErrorCode::UnknownPackage
        } else {
            ResolutionErrorCode::MissingDependency
        };
        ResolutionError::new(
            code,
            Some(requirement.package.clone()),
            format!(
                "{} requires {}, but no package candidate is available",
                requirement.required_by, requirement.range_text
            ),
        )
    })?;
    let mut last_error = None;
    for candidate in candidates {
        if !requirement.range.matches(&candidate.version) {
            continue;
        }
        if !candidate.engine_range.matches(engine) {
            last_error = Some(ResolutionError::new(
                ResolutionErrorCode::IncompatibleEngine,
                Some(requirement.package.clone()),
                format!(
                    "version {} requires engine {}, but the host is {}",
                    candidate.version, candidate.candidate.manifest.engine_range, engine
                ),
            ));
            continue;
        }
        if !candidate.sdk_range.matches(sdk) {
            last_error = Some(ResolutionError::new(
                ResolutionErrorCode::IncompatibleSdk,
                Some(requirement.package.clone()),
                format!(
                    "version {} requires SDK {}, but the host is {}",
                    candidate.version, candidate.candidate.manifest.sdk_range, sdk
                ),
            ));
            continue;
        }
        let selected_features = match selected_features(candidate, features) {
            Ok(features) => features,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let mut next_selected = selected.clone();
        next_selected.insert(requirement.package.clone(), candidate.clone());
        let mut next_requirements = requirements.clone();
        next_requirements.extend(requirements_for(candidate, &selected_features));
        match solve(
            available,
            features,
            engine,
            sdk,
            next_selected,
            next_requirements,
        ) {
            Ok(resolved) => return Ok(resolved),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        ResolutionError::new(
            ResolutionErrorCode::VersionConflict,
            Some(requirement.package.clone()),
            format!(
                "{} requires {}; no compatible version is available",
                requirement.required_by, requirement.range_text
            ),
        )
    }))
}

fn selected_features(
    candidate: &PreparedCandidate,
    requested: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, ResolutionError> {
    let manifest = &candidate.candidate.manifest;
    let selected: BTreeSet<String> = requested
        .get(&manifest.package_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    for feature in &selected {
        if !manifest.features.iter().any(|known| known.name == *feature) {
            return Err(ResolutionError::new(
                ResolutionErrorCode::InvalidFeature,
                Some(manifest.package_id.clone()),
                format!("unknown feature `{feature}`"),
            ));
        }
    }
    Ok(selected.into_iter().collect())
}

fn requirements_for(
    candidate: &PreparedCandidate,
    selected_features: &[String],
) -> Vec<Requirement> {
    let manifest = &candidate.candidate.manifest;
    let owner = manifest.package_id.clone();
    let dependencies = manifest
        .dependencies
        .iter()
        .filter(|dependency| !dependency.optional)
        .chain(
            manifest
                .features
                .iter()
                .filter(|feature| selected_features.contains(&feature.name))
                .flat_map(|feature| feature.dependencies.iter()),
        );
    dependencies
        .map(|dependency| requirement_for(&owner, dependency))
        .collect()
}

fn requirement_for(owner: &str, dependency: &PackageDependency) -> Requirement {
    Requirement {
        package: dependency.package.clone(),
        range: VersionReq::parse(&dependency.version_range).unwrap_or(VersionReq::STAR),
        range_text: dependency.version_range.clone(),
        required_by: owner.to_owned(),
    }
}

fn topological_order(
    selected: &BTreeMap<String, PreparedCandidate>,
    features: &BTreeMap<String, Vec<String>>,
    load_order: &[String],
) -> Result<Vec<String>, ResolutionError> {
    let priorities: BTreeMap<_, _> = load_order
        .iter()
        .enumerate()
        .map(|(index, package)| (package.as_str(), index))
        .collect();
    let mut indegree = BTreeMap::new();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (package, candidate) in selected {
        let selected_features = selected_features(candidate, features)?;
        let dependencies: BTreeSet<_> = requirements_for(candidate, &selected_features)
            .into_iter()
            .map(|requirement| requirement.package)
            .collect();
        indegree.insert(package.clone(), dependencies.len());
        for dependency in dependencies {
            dependents
                .entry(dependency)
                .or_default()
                .push(package.clone());
        }
    }
    let mut remaining: BTreeSet<String> = selected.keys().cloned().collect();
    let mut ordered = Vec::with_capacity(selected.len());
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .filter(|package| indegree.get(*package) == Some(&0))
            .min_by_key(|package| {
                (
                    priorities
                        .get(package.as_str())
                        .copied()
                        .unwrap_or(usize::MAX),
                    (*package).clone(),
                )
            })
            .cloned();
        let Some(next) = next else {
            return Err(ResolutionError::new(
                ResolutionErrorCode::DependencyCycle,
                None,
                format!(
                    "dependency cycle involves {}",
                    remaining.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            ));
        };
        remaining.remove(&next);
        ordered.push(next.clone());
        if let Some(children) = dependents.get(&next) {
            for child in children {
                if let Some(degree) = indegree.get_mut(child) {
                    *degree -= 1;
                }
            }
        }
    }
    Ok(ordered)
}

fn locked_modules(
    ordered: &[String],
    selected: &BTreeMap<String, PreparedCandidate>,
    features: &BTreeMap<String, Vec<String>>,
) -> Vec<ModuleLock> {
    ordered
        .iter()
        .map(|package| {
            let candidate = &selected[package];
            let manifest = &candidate.candidate.manifest;
            ModuleLock {
                package: package.clone(),
                version: manifest.version.clone(),
                required: true,
                state_schema: manifest.save_schema,
                content_hash: candidate.candidate.content_hash.clone(),
                features: selected_features(candidate, features).unwrap_or_default(),
            }
        })
        .collect()
}

fn resolve_providers(
    ordered: &[String],
    selected: &BTreeMap<String, PreparedCandidate>,
) -> Result<Vec<ResolvedProvider>, ResolutionError> {
    let mut providers: BTreeMap<ContentId, Vec<(String, bool)>> = BTreeMap::new();
    for package in ordered {
        for declaration in &selected[package].candidate.manifest.providers {
            providers
                .entry(declarationservice(declaration))
                .or_default()
                .push((package.clone(), declaration.exclusive));
        }
    }
    providers
        .into_iter()
        .map(|(service, entries)| {
            if entries.len() > 1 && entries.iter().any(|(_, exclusive)| *exclusive) {
                return Err(ResolutionError::new(
                    ResolutionErrorCode::ProviderConflict,
                    None,
                    format!(
                        "exclusive service `{service}` has multiple providers: {}",
                        entries
                            .iter()
                            .map(|(package, _)| package.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            Ok(ResolvedProvider {
                service,
                packages: entries.into_iter().map(|(package, _)| package).collect(),
            })
        })
        .collect()
}

fn declarationservice(declaration: &crate::ProviderDeclaration) -> ContentId {
    declaration.service.clone()
}

fn validate_content_declarations(
    ordered: &[String],
    selected: &BTreeMap<String, PreparedCandidate>,
) -> Result<(), ResolutionError> {
    let mut declarations = BTreeMap::new();
    for package in ordered {
        for record in &selected[package].candidate.manifest.records {
            match record.operation {
                crate::PackageOperation::Add => {
                    if let Some(previous) = declarations.insert(record.id.clone(), package.clone())
                    {
                        return Err(ResolutionError::new(
                            ResolutionErrorCode::DuplicateContent,
                            Some(package.clone()),
                            format!(
                                "content `{}` is also added by `{previous}`; use an explicit ordered replacement",
                                record.id
                            ),
                        ));
                    }
                }
                crate::PackageOperation::Replace => {
                    declarations.insert(record.id.clone(), package.clone());
                }
            }
        }
    }
    Ok(())
}

fn fingerprint_modules(revision: ContentRevision, modules: &[ModuleLock]) -> String {
    let bytes = serde_json::to_vec(&(revision, modules))
        .expect("serializing resolved package identities cannot fail");
    format!("{:x}", Sha256::digest(bytes))
}

fn parse_version(
    value: &str,
    package: Option<&str>,
    role: &str,
) -> Result<Version, ResolutionError> {
    Version::parse(value).map_err(|error| {
        ResolutionError::new(
            ResolutionErrorCode::InvalidRequest,
            package.map(str::to_owned),
            format!("invalid {role} version `{value}`: {error}"),
        )
    })
}

fn parse_range(value: &str, package: &str, role: &str) -> Result<VersionReq, ResolutionError> {
    VersionReq::parse(value).map_err(|error| {
        ResolutionError::new(
            ResolutionErrorCode::InvalidRequest,
            Some(package.to_owned()),
            format!("invalid {role} range `{value}`: {error}"),
        )
    })
}

fn validate_string_set(values: &[String], role: &str) -> Result<(), ResolutionError> {
    let mut unique = BTreeSet::new();
    for value in values {
        if value.is_empty() || !unique.insert(value) {
            return Err(ResolutionError::new(
                ResolutionErrorCode::InvalidRequest,
                Some(value.clone()),
                format!("{role} values must be nonempty and unique"),
            ));
        }
    }
    Ok(())
}
