use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, ContentRevision, ValidationIssue, ValidationReport};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryRecord {
    pub id: ContentId,
    pub source: String,
    pub source_location: String,
    pub data: serde_json::Value,
}

impl RegistryRecord {
    pub fn new(id: ContentId, data: serde_json::Value) -> Self {
        Self {
            id,
            source: "unspecified".to_owned(),
            source_location: "unspecified".to_owned(),
            data,
        }
    }

    pub fn with_source(
        mut self,
        source: impl Into<String>,
        source_location: impl Into<String>,
    ) -> Self {
        self.source = source.into();
        self.source_location = source_location.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentAlias {
    pub alias: ContentId,
    pub target: ContentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRename {
    pub from: ContentId,
    pub to: ContentId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryPatch {
    pub additions: Vec<RegistryRecord>,
    pub replacements: Vec<RegistryRecord>,
    pub renames: Vec<ContentRename>,
    pub aliases: Vec<ContentAlias>,
}

impl RegistryPatch {
    pub fn add(record: RegistryRecord) -> Self {
        Self {
            additions: vec![record],
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.additions.is_empty()
            && self.replacements.is_empty()
            && self.renames.is_empty()
            && self.aliases.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    records: BTreeMap<ContentId, RegistryRecord>,
    aliases: BTreeMap<ContentId, ContentId>,
    revision: ContentRevision,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> ContentRevision {
        self.revision
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn get(&self, id: &ContentId) -> Option<&RegistryRecord> {
        let canonical = self.resolve(id)?;
        self.records.get(canonical)
    }

    pub fn resolve<'a>(&'a self, id: &'a ContentId) -> Option<&'a ContentId> {
        if self.records.contains_key(id) {
            Some(id)
        } else {
            self.aliases.get(id)
        }
    }

    pub fn records(&self) -> impl Iterator<Item = &RegistryRecord> {
        self.records.values()
    }

    pub fn aliases(&self) -> impl Iterator<Item = (&ContentId, &ContentId)> {
        self.aliases.iter()
    }

    pub fn apply(&mut self, patch: RegistryPatch) -> Result<ContentRevision, ValidationReport> {
        if patch.is_empty() {
            return Ok(self.revision);
        }

        let mut staged = self.clone();
        let mut report = ValidationReport::default();
        staged.apply_patch(patch, &mut report);
        if !report.is_valid() {
            return Err(report);
        }

        let Some(revision) = staged.revision.checked_next() else {
            let mut report = ValidationReport::default();
            report.push(ValidationIssue::error(
                "revision-exhausted",
                "content revision cannot advance beyond u64::MAX",
                None,
                None,
            ));
            return Err(report);
        };
        staged.revision = revision;
        *self = staged;
        Ok(revision)
    }

    fn apply_patch(&mut self, patch: RegistryPatch, report: &mut ValidationReport) {
        for record in patch.additions {
            if self.records.contains_key(&record.id) || self.aliases.contains_key(&record.id) {
                push_issue(
                    report,
                    "duplicate-content-id",
                    format!(
                        "cannot add `{}` because that identity is already in use",
                        record.id
                    ),
                    &record,
                );
                continue;
            }
            self.records.insert(record.id.clone(), record);
        }

        for record in patch.replacements {
            if self.aliases.contains_key(&record.id) {
                push_issue(
                    report,
                    "replacement-target-is-alias",
                    format!("cannot replace `{}` because it is an alias", record.id),
                    &record,
                );
                continue;
            }
            if !self.records.contains_key(&record.id) {
                push_issue(
                    report,
                    "replacement-target-missing",
                    format!("cannot replace unknown content `{}`", record.id),
                    &record,
                );
                continue;
            }
            self.records.insert(record.id.clone(), record);
        }

        for rename in patch.renames {
            let Some(mut record) = self.records.get(&rename.from).cloned() else {
                report.push(ValidationIssue::error(
                    "rename-source-missing",
                    format!("cannot rename unknown content `{}`", rename.from),
                    Some(rename.from),
                    None,
                ));
                continue;
            };
            if self.records.contains_key(&rename.to) || self.aliases.contains_key(&rename.to) {
                report.push(ValidationIssue::error(
                    "rename-destination-in-use",
                    format!(
                        "cannot rename `{}` to `{}` because the destination is already in use",
                        rename.from, rename.to
                    ),
                    Some(rename.to),
                    Some(record.source_location.clone()),
                ));
                continue;
            }

            self.records.remove(&rename.from);
            for target in self.aliases.values_mut() {
                if *target == rename.from {
                    *target = rename.to.clone();
                }
            }
            record.id = rename.to.clone();
            self.records.insert(rename.to.clone(), record);
            self.aliases.insert(rename.from, rename.to);
        }

        for alias in patch.aliases {
            if self.records.contains_key(&alias.alias) || self.aliases.contains_key(&alias.alias) {
                report.push(ValidationIssue::error(
                    "duplicate-alias",
                    format!(
                        "cannot create alias `{}` because that identity is already in use",
                        alias.alias
                    ),
                    Some(alias.alias),
                    None,
                ));
                continue;
            }
            if !self.records.contains_key(&alias.target) {
                report.push(ValidationIssue::error(
                    "alias-target-missing",
                    format!(
                        "cannot alias `{}` to `{}`: the target must be an existing canonical record, not an alias",
                        alias.alias, alias.target
                    ),
                    Some(alias.target),
                    None,
                ));
                continue;
            }
            self.aliases.insert(alias.alias, alias.target);
        }
    }
}

fn push_issue(report: &mut ValidationReport, code: &str, message: String, record: &RegistryRecord) {
    report.push(ValidationIssue::error(
        code,
        message,
        Some(record.id.clone()),
        Some(record.source_location.clone()),
    ));
}
