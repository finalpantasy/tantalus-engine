use crate::ContentId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum PropertyValue {
    Integer(i64),
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffixRoll {
    pub definition: ContentId,
    pub resolved_values: BTreeMap<ContentId, PropertyValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceProperties {
    pub affixes: Vec<AffixRoll>,
    pub stats: BTreeMap<ContentId, PropertyValue>,
    pub slot: Option<ContentId>,
    pub custom: BTreeMap<ContentId, PropertyValue>,
}
