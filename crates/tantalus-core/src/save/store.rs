use super::{MigrationRegistry, SaveError, SaveErrorCategory, SaveFailurePoint};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use tantalus_contracts::{
    CURRENT_SAVE_FORMAT_VERSION, ContentId, ProfileLock, SaveEnvelope, SaveId,
};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavePointer {
    latest: SaveId,
    previous: Option<SaveId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SaveRecord {
    id: SaveId,
    previous: Option<SaveId>,
    digest: String,
    envelope: SaveEnvelope,
}

#[derive(Debug, Clone)]
pub struct LoadPlan {
    pub save_id: SaveId,
    pub envelope: SaveEnvelope,
    pub target_profile: ProfileLock,
    pub missing_required_modules: Vec<ContentId>,
    pub migration_required: bool,
    pub profile_changed: bool,
}

impl LoadPlan {
    pub fn into_ready(self) -> Result<SaveEnvelope, SaveError> {
        if !self.missing_required_modules.is_empty() {
            return Err(SaveError::new(
                SaveErrorCategory::MissingRequiredModule,
                "resume",
                None,
                format!(
                    "required modules are unavailable: {:?}; no state was discarded",
                    self.missing_required_modules
                ),
                true,
            ));
        }
        if self.migration_required || self.profile_changed {
            return Err(SaveError::new(
                SaveErrorCategory::MigrationFailed,
                "resume",
                None,
                "the save requires an explicit successful migration before continuation",
                true,
            ));
        }
        Ok(self.envelope)
    }

    pub fn migrate(self, migrations: &MigrationRegistry) -> Result<SaveEnvelope, SaveError> {
        if !self.missing_required_modules.is_empty() {
            return Err(SaveError::new(
                SaveErrorCategory::MissingRequiredModule,
                "migrate",
                None,
                format!(
                    "required modules are unavailable: {:?}; no state was discarded",
                    self.missing_required_modules
                ),
                true,
            ));
        }
        let envelope = migrations.migrate_to_current(self.envelope)?;
        if envelope.profile != self.target_profile {
            return Err(SaveError::new(
                SaveErrorCategory::MigrationFailed,
                "migrate_profile",
                None,
                "save migration did not resolve the requested profile lock",
                true,
            ));
        }
        validate_envelope(&envelope)?;
        Ok(envelope)
    }
}

#[derive(Debug)]
pub struct SaveStore {
    root: PathBuf,
    fail_next: Option<SaveFailurePoint>,
}

impl SaveStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            fail_next: None,
        }
    }

    pub fn fail_next_commit(&mut self, point: SaveFailurePoint) {
        self.fail_next = Some(point);
    }

    pub fn commit(&mut self, snapshot: &SaveEnvelope) -> Result<SaveId, SaveError> {
        if snapshot.format_version != CURRENT_SAVE_FORMAT_VERSION {
            return Err(SaveError::new(
                SaveErrorCategory::UnsupportedVersion,
                "commit",
                None,
                "only current-format save envelopes can be committed",
                true,
            ));
        }
        validate_envelope(snapshot)?;
        self.create_layout()?;
        let previous = self.latest_pointer()?.map(|pointer| pointer.latest);
        let id = self.next_save_id(previous)?;
        let digest = digest_envelope(snapshot)?;
        let record = SaveRecord {
            id,
            previous,
            digest,
            envelope: snapshot.clone(),
        };
        let generation_path = self.generation_path(id);
        self.inject_failure(SaveFailurePoint::BeforeGenerationPublish)?;
        let generation = NamedTempFile::new_in(self.generations_path()).map_err(|error| {
            SaveError::io("create_generation_staging", &self.generations_path(), error)
        })?;
        write_json(&generation, &record, "stage_generation")?;
        generation
            .persist_noclobber(&generation_path)
            .map_err(|error| SaveError::io("publish_generation", &generation_path, error.error))?;

        let pointer = SavePointer {
            latest: id,
            previous,
        };
        self.inject_failure(SaveFailurePoint::BeforeLatestPublish)?;
        let pointer_file = NamedTempFile::new_in(&self.root)
            .map_err(|error| SaveError::io("create_pointer_staging", &self.root, error))?;
        write_json(&pointer_file, &pointer, "stage_pointer")?;
        pointer_file
            .persist(self.current_path())
            .map_err(|error| SaveError::io("publish_pointer", &self.current_path(), error.error))?;
        self.inject_failure(SaveFailurePoint::AfterLatestPublish)?;
        Ok(id)
    }

    pub fn commit_outcome(
        &mut self,
        snapshot: &SaveEnvelope,
        resolution_id: u64,
    ) -> Result<SaveId, SaveError> {
        if snapshot
            .outcomes
            .iter()
            .filter(|outcome| outcome.resolution_id == resolution_id)
            .count()
            != 1
        {
            return Err(SaveError::invalid(
                "an outcome transaction must contain exactly one matching resolution marker",
            ));
        }
        if let Some(latest) = self.latest()? {
            let current = self.read_record(latest)?.envelope;
            if current
                .outcomes
                .iter()
                .any(|outcome| outcome.resolution_id == resolution_id)
            {
                return Err(SaveError::new(
                    SaveErrorCategory::Conflict,
                    "commit_outcome",
                    Some(self.generation_path(latest)),
                    format!("resolution {resolution_id} is already committed"),
                    true,
                ));
            }
            if current
                .outcomes
                .iter()
                .any(|previous| !snapshot.outcomes.iter().any(|outcome| outcome == previous))
            {
                return Err(SaveError::invalid(
                    "an outcome transaction must retain every previously committed outcome",
                ));
            }
        }
        self.commit(snapshot)
    }

    pub fn latest(&self) -> Result<Option<SaveId>, SaveError> {
        Ok(self.latest_pointer()?.map(|pointer| pointer.latest))
    }

    pub fn load(&self, id: SaveId, profile: &ProfileLock) -> Result<LoadPlan, SaveError> {
        let record = self.read_record(id)?;
        let target_modules: BTreeSet<_> = profile
            .modules
            .iter()
            .map(|module| &module.package)
            .collect();
        let missing_required_modules = record
            .envelope
            .profile
            .modules
            .iter()
            .filter(|module| module.required && !target_modules.contains(&module.package))
            .map(|module| module.package.clone())
            .collect();
        let migration_required = record.envelope.format_version < CURRENT_SAVE_FORMAT_VERSION;
        let profile_changed = record.envelope.profile != *profile;
        Ok(LoadPlan {
            save_id: id,
            envelope: record.envelope,
            target_profile: profile.clone(),
            missing_required_modules,
            migration_required,
            profile_changed,
        })
    }

    pub fn recover_latest(&self, profile: &ProfileLock) -> Result<LoadPlan, SaveError> {
        let pointer = self
            .latest_pointer()?
            .ok_or_else(|| SaveError::invalid("no authoritative current save pointer exists"))?;
        let mut candidate = Some(pointer.latest);
        let mut visited = BTreeSet::new();
        while let Some(id) = candidate {
            if !visited.insert(id) {
                return Err(SaveError::new(
                    SaveErrorCategory::Corrupt,
                    "recover",
                    Some(self.current_path()),
                    "save generation history contains a cycle",
                    true,
                ));
            }
            match self.read_record(id) {
                Ok(_) => return self.load(id, profile),
                Err(error) if error.category == SaveErrorCategory::Corrupt => {
                    candidate = if id == pointer.latest {
                        pointer.previous
                    } else {
                        self.read_previous_link(id)?
                    };
                }
                Err(error) => return Err(error),
            }
        }
        Err(SaveError::new(
            SaveErrorCategory::Corrupt,
            "recover",
            Some(self.current_path()),
            "no valid authoritative save generation remains",
            true,
        ))
    }

    fn create_layout(&self) -> Result<(), SaveError> {
        fs::create_dir_all(self.generations_path())
            .map_err(|error| SaveError::io("create_layout", &self.generations_path(), error))
    }

    fn next_save_id(&self, previous: Option<SaveId>) -> Result<SaveId, SaveError> {
        let mut maximum = previous.map_or(0, SaveId::get);
        let entries = fs::read_dir(self.generations_path()).map_err(|error| {
            SaveError::io("enumerate_generations", &self.generations_path(), error)
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                SaveError::io("enumerate_generations", &self.generations_path(), error)
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Some(value) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .and_then(|value| value.parse::<u64>().ok())
            {
                maximum = maximum.max(value);
            }
        }
        maximum.checked_add(1).map(SaveId::new).ok_or_else(|| {
            SaveError::new(
                SaveErrorCategory::Conflict,
                "allocate_save_id",
                Some(self.generations_path()),
                "save ID space is exhausted",
                false,
            )
        })
    }

    fn latest_pointer(&self) -> Result<Option<SavePointer>, SaveError> {
        match File::open(self.current_path()) {
            Ok(file) => serde_json::from_reader(file).map(Some).map_err(|error| {
                SaveError::new(
                    SaveErrorCategory::Corrupt,
                    "read_pointer",
                    Some(self.current_path()),
                    error.to_string(),
                    true,
                )
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(SaveError::io("read_pointer", &self.current_path(), error)),
        }
    }

    fn read_record(&self, id: SaveId) -> Result<SaveRecord, SaveError> {
        let path = self.generation_path(id);
        let file =
            File::open(&path).map_err(|error| SaveError::io("read_generation", &path, error))?;
        let record: SaveRecord = serde_json::from_reader(file).map_err(|error| {
            SaveError::new(
                SaveErrorCategory::Corrupt,
                "decode_generation",
                Some(path.clone()),
                error.to_string(),
                true,
            )
        })?;
        if record.id != id || digest_envelope(&record.envelope)? != record.digest {
            return Err(SaveError::new(
                SaveErrorCategory::Corrupt,
                "verify_generation",
                Some(path),
                "generation identity or content digest does not match",
                true,
            ));
        }
        validate_envelope(&record.envelope)?;
        Ok(record)
    }

    fn read_previous_link(&self, id: SaveId) -> Result<Option<SaveId>, SaveError> {
        let path = self.generation_path(id);
        let file =
            File::open(&path).map_err(|error| SaveError::io("read_recovery_link", &path, error))?;
        let value: serde_json::Value = serde_json::from_reader(file).map_err(|error| {
            SaveError::new(
                SaveErrorCategory::Corrupt,
                "read_recovery_link",
                Some(path),
                error.to_string(),
                true,
            )
        })?;
        value
            .get("previous")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| {
                SaveError::new(
                    SaveErrorCategory::Corrupt,
                    "read_recovery_link",
                    Some(self.generation_path(id)),
                    error.to_string(),
                    true,
                )
            })
    }

    fn inject_failure(&mut self, point: SaveFailurePoint) -> Result<(), SaveError> {
        if self.fail_next == Some(point) {
            self.fail_next = None;
            return Err(SaveError::new(
                SaveErrorCategory::Io,
                "injected_failure",
                Some(self.root.clone()),
                format!("injected failure at {point:?}"),
                true,
            ));
        }
        Ok(())
    }

    fn generations_path(&self) -> PathBuf {
        self.root.join("generations")
    }

    fn current_path(&self) -> PathBuf {
        self.root.join("current.json")
    }

    fn generation_path(&self, id: SaveId) -> PathBuf {
        self.generations_path()
            .join(format!("{:020}.json", id.get()))
    }
}

fn write_json(
    file: &NamedTempFile,
    value: &impl Serialize,
    operation: &str,
) -> Result<(), SaveError> {
    let path = file.path().to_path_buf();
    let mut handle = file.as_file();
    serde_json::to_writer(&mut handle, value).map_err(|error| {
        SaveError::new(
            SaveErrorCategory::Io,
            operation,
            Some(path.clone()),
            error.to_string(),
            true,
        )
    })?;
    handle
        .write_all(b"\n")
        .and_then(|()| handle.sync_all())
        .map_err(|error| SaveError::io(operation, &path, error))
}

fn digest_envelope(envelope: &SaveEnvelope) -> Result<String, SaveError> {
    let bytes = serde_json::to_vec(envelope).map_err(|error| {
        SaveError::new(
            SaveErrorCategory::InvalidData,
            "canonicalize",
            None,
            error.to_string(),
            true,
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_envelope(envelope: &SaveEnvelope) -> Result<(), SaveError> {
    if envelope.format_version > CURRENT_SAVE_FORMAT_VERSION
        || envelope.engine.name.is_empty()
        || envelope.engine.version.is_empty()
        || envelope.profile.content_fingerprint.is_empty()
    {
        return Err(SaveError::invalid(
            "save identity, supported format version, and profile fingerprint are required",
        ));
    }
    validate_unique(
        envelope.state.entities.iter().map(|entity| entity.id.get()),
        "entity IDs",
    )?;
    validate_unique(
        envelope.state.items.iter().map(|item| item.id.get()),
        "item instance IDs",
    )?;
    validate_unique(
        envelope
            .state
            .encounters
            .iter()
            .map(|encounter| encounter.id.get()),
        "encounter IDs",
    )?;
    validate_unique(
        envelope
            .outcomes
            .iter()
            .map(|outcome| outcome.resolution_id),
        "outcome resolution IDs",
    )?;
    validate_unique(
        envelope
            .profile
            .modules
            .iter()
            .map(|module| module.package.to_string()),
        "profile module IDs",
    )?;
    validate_unique(
        envelope
            .unknown_sections
            .iter()
            .map(|section| section.name.clone()),
        "unknown section names",
    )?;
    Ok(())
}

fn validate_unique(values: impl Iterator<Item = impl Ord>, label: &str) -> Result<(), SaveError> {
    let mut unique = BTreeSet::new();
    for value in values {
        if !unique.insert(value) {
            return Err(SaveError::invalid(format!(
                "duplicate {label} are not allowed"
            )));
        }
    }
    Ok(())
}
