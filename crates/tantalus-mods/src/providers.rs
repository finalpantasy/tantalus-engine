use crate::{
    ModuleError, ModuleErrorKind, ModuleHost, ModuleInvocation, ModuleLimits, ResolvedProfile,
    StagedPackage,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, ProfileLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPhase {
    Running,
    Failed,
    Stopped,
}

struct ActiveProvider {
    package: String,
    phase: ProviderPhase,
    host: ModuleHost,
}

pub struct ProviderRegistry {
    profile: ProfileLock,
    providers: BTreeMap<ContentId, ActiveProvider>,
}

impl ProviderRegistry {
    pub fn instantiate(
        profile: &ResolvedProfile,
        packages: &[StagedPackage],
        limits: ModuleLimits,
    ) -> Result<Self, ModuleError> {
        let mut providers = BTreeMap::new();
        for resolved in &profile.providers {
            if resolved.bindings.len() != 1 {
                return Err(ModuleError::new(
                    ModuleErrorKind::HostValidation,
                    "",
                    format!(
                        "service `{}` requires one transaction provider; {} were resolved",
                        resolved.service,
                        resolved.bindings.len()
                    ),
                ));
            }
            let binding = &resolved.bindings[0];
            let staged = packages
                .iter()
                .find(|package| package.manifest.package_id == binding.package)
                .ok_or_else(|| {
                    ModuleError::new(
                        ModuleErrorKind::HostValidation,
                        &binding.package,
                        "resolved provider package was not staged",
                    )
                })?;
            let mut host = ModuleHost::instantiate_entrypoint(
                &profile.lock,
                staged,
                &binding.entrypoint,
                limits.clone(),
            )?;
            host.invoke(&serde_json::json!({
                "operation": "initialize",
                "service": resolved.service,
                "package": binding.package,
            }))?;
            providers.insert(
                resolved.service.clone(),
                ActiveProvider {
                    package: binding.package.clone(),
                    phase: ProviderPhase::Running,
                    host,
                },
            );
        }
        Ok(Self {
            profile: profile.lock.clone(),
            providers,
        })
    }

    pub fn phase(&self, service: &ContentId) -> Option<ProviderPhase> {
        self.providers.get(service).map(|provider| provider.phase)
    }

    pub fn dispatch(
        &mut self,
        service: &ContentId,
        input: &serde_json::Value,
    ) -> Result<ModuleInvocation, ModuleError> {
        let provider = self.provider_mut(service)?;
        provider.host.invoke(input).inspect_err(|_| {
            provider.phase = ProviderPhase::Failed;
        })
    }

    pub fn snapshot(&self, service: &ContentId) -> Result<serde_json::Value, ModuleError> {
        let provider = self.providers.get(service).ok_or_else(|| {
            ModuleError::new(
                ModuleErrorKind::HostValidation,
                "",
                format!("service `{service}` is not active"),
            )
        })?;
        if provider.phase != ProviderPhase::Running {
            return Err(ModuleError::new(
                ModuleErrorKind::HostValidation,
                &provider.package,
                "provider can snapshot only while running",
            ));
        }
        Ok(provider.host.module_state().clone())
    }

    pub fn shutdown(&mut self, service: &ContentId) -> Result<(), ModuleError> {
        let provider = self.provider_mut(service)?;
        provider
            .host
            .invoke(&serde_json::json!({"operation": "shutdown", "service": service}))
            .inspect_err(|_| {
                provider.phase = ProviderPhase::Failed;
            })?;
        provider.phase = ProviderPhase::Stopped;
        Ok(())
    }

    pub fn profile(&self) -> &ProfileLock {
        &self.profile
    }

    fn provider_mut(&mut self, service: &ContentId) -> Result<&mut ActiveProvider, ModuleError> {
        let provider = self.providers.get_mut(service).ok_or_else(|| {
            ModuleError::new(
                ModuleErrorKind::HostValidation,
                "",
                format!("service `{service}` has no active provider"),
            )
        })?;
        if provider.phase != ProviderPhase::Running {
            return Err(ModuleError::new(
                ModuleErrorKind::HostValidation,
                &provider.package,
                format!("provider is {:?}; no fallback is permitted", provider.phase),
            ));
        }
        Ok(provider)
    }
}
