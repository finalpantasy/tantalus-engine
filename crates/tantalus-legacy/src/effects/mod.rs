use std::collections::BTreeSet;
use tantalus_contracts::{ContentId, ValidationIssue, ValidationReport};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalEffectPath {
    pub id: ContentId,
    pub required_resources: Vec<ContentId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEffect {
    pub id: ContentId,
    pub required_resources: Vec<ContentId>,
}

impl OriginalEffectPath {
    pub fn validate(
        &self,
        available: &BTreeSet<ContentId>,
    ) -> Result<ValidatedEffect, ValidationReport> {
        let mut report = ValidationReport::default();
        for resource in &self.required_resources {
            if !available.contains(resource) {
                report.push(ValidationIssue::error(
                    "missing-effect-resource",
                    format!("effect `{}` requires `{resource}`", self.id),
                    Some(self.id.clone()),
                    None,
                ));
            }
        }
        if report.is_valid() {
            Ok(ValidatedEffect {
                id: self.id.clone(),
                required_resources: self.required_resources.clone(),
            })
        } else {
            Err(report)
        }
    }
}
