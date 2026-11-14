use super::{SaveError, SaveErrorCategory};
use std::collections::BTreeMap;
use std::sync::Arc;
use tantalus_contracts::{CURRENT_SAVE_FORMAT_VERSION, MigrationRecord, SaveEnvelope};

type Migration = Arc<dyn Fn(SaveEnvelope) -> Result<SaveEnvelope, SaveError> + Send + Sync>;

#[derive(Default, Clone)]
pub struct MigrationRegistry {
    steps: BTreeMap<u32, Migration>,
}

impl MigrationRegistry {
    pub fn register<F>(&mut self, from_version: u32, migration: F) -> Result<(), SaveError>
    where
        F: Fn(SaveEnvelope) -> Result<SaveEnvelope, SaveError> + Send + Sync + 'static,
    {
        if from_version >= CURRENT_SAVE_FORMAT_VERSION {
            return Err(SaveError::new(
                SaveErrorCategory::UnsupportedVersion,
                "register_migration",
                None,
                format!("invalid source save version {from_version}"),
                false,
            ));
        }
        if self.steps.contains_key(&from_version) {
            return Err(SaveError::new(
                SaveErrorCategory::Conflict,
                "register_migration",
                None,
                format!("a migration from version {from_version} is already registered"),
                false,
            ));
        }
        self.steps.insert(from_version, Arc::new(migration));
        Ok(())
    }

    pub fn migrate_to_current(
        &self,
        mut envelope: SaveEnvelope,
    ) -> Result<SaveEnvelope, SaveError> {
        while envelope.format_version < CURRENT_SAVE_FORMAT_VERSION {
            let from = envelope.format_version;
            let migration = self.steps.get(&from).ok_or_else(|| {
                SaveError::new(
                    SaveErrorCategory::MigrationFailed,
                    "migrate",
                    None,
                    format!("no migration is registered from save version {from}"),
                    true,
                )
            })?;
            envelope = migration(envelope).map_err(|error| {
                SaveError::new(
                    SaveErrorCategory::MigrationFailed,
                    "migrate",
                    None,
                    format!(
                        "migration from version {from} failed; source unchanged: {}",
                        error.context
                    ),
                    true,
                )
            })?;
            if envelope.format_version != from + 1 {
                return Err(SaveError::new(
                    SaveErrorCategory::MigrationFailed,
                    "migrate",
                    None,
                    format!(
                        "migration from version {from} did not produce version {}",
                        from + 1
                    ),
                    true,
                ));
            }
            envelope.migration_history.push(MigrationRecord {
                owner: "tantalus-core/save".to_owned(),
                from_version: from,
                to_version: from + 1,
            });
        }
        if envelope.format_version != CURRENT_SAVE_FORMAT_VERSION {
            return Err(SaveError::new(
                SaveErrorCategory::UnsupportedVersion,
                "migrate",
                None,
                "save version is newer than this engine supports",
                true,
            ));
        }
        Ok(envelope)
    }
}
