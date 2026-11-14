use crate::{ModuleError, ModuleErrorKind, ModuleHost, ModuleLimits, StagedPackage};
use tantalus_contracts::{ContentId, MigrationRecord, ProfileLock, SaveEnvelope};

pub struct ModuleMigration<'a> {
    pub target_profile: &'a ProfileLock,
    pub staged: &'a StagedPackage,
    pub entrypoint: &'a str,
    pub section: &'a ContentId,
    pub limits: ModuleLimits,
}

impl ModuleMigration<'_> {
    pub fn migrate(&self, source: &SaveEnvelope) -> Result<SaveEnvelope, ModuleError> {
        let package = &self.staged.manifest.package_id;
        let target = self
            .target_profile
            .modules
            .iter()
            .find(|module| module.package == *package)
            .ok_or_else(|| {
                ModuleError::new(
                    ModuleErrorKind::HostValidation,
                    package,
                    "migration package is absent from the target profile",
                )
            })?;
        let section = source.modules.get(self.section).ok_or_else(|| {
            ModuleError::new(
                ModuleErrorKind::HostValidation,
                package,
                format!("module state section `{}` is missing", self.section),
            )
        })?;
        let from = section.schema_version;
        let to = target.state_schema;
        if from > to {
            return Err(ModuleError::new(
                ModuleErrorKind::HostValidation,
                package,
                format!("cannot downgrade module state from schema {from} to {to}"),
            ));
        }
        let mut migrated = source.clone();
        if from == to {
            migrated.profile = self.target_profile.clone();
            return Ok(migrated);
        }

        let mut host = ModuleHost::instantiate_entrypoint(
            self.target_profile,
            self.staged,
            self.entrypoint,
            self.limits.clone(),
        )?;
        host.restore_module_state(section.state.clone());
        let result = host.invoke(&serde_json::json!({
            "operation": "migrate",
            "from_schema": from,
            "to_schema": to,
        }))?;
        if !result.events.is_empty()
            || !host.authoritative_state().is_empty()
            || host.next_id() != 1
        {
            return Err(ModuleError::new(
                ModuleErrorKind::HostValidation,
                package,
                "migration emitted events or host mutations; source save is unchanged",
            ));
        }
        let migrated_section = migrated.modules.get_mut(self.section).ok_or_else(|| {
            ModuleError::new(
                ModuleErrorKind::HostValidation,
                package,
                "cloned module section disappeared during migration",
            )
        })?;
        migrated_section.schema_version = to;
        migrated_section.state = result.module_state;
        migrated.migration_history.push(MigrationRecord {
            owner: package.clone(),
            from_version: from,
            to_version: to,
        });
        migrated.profile = self.target_profile.clone();
        Ok(migrated)
    }
}
