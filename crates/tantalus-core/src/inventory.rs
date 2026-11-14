use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tantalus_contracts::{
    ContentId, EngineError, ErrorCategory, InstanceProperties, ItemInstanceId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemInstance {
    id: ItemInstanceId,
    definition: ContentId,
    properties: InstanceProperties,
}

impl ItemInstance {
    pub fn id(&self) -> ItemInstanceId {
        self.id
    }

    pub fn definition(&self) -> &ContentId {
        &self.definition
    }

    pub fn properties(&self) -> &InstanceProperties {
        &self.properties
    }

    pub fn properties_mut(&mut self) -> &mut InstanceProperties {
        &mut self.properties
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    items: BTreeMap<ItemInstanceId, ItemInstance>,
    next_id: u64,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            items: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl Inventory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn spawn(
        &mut self,
        definition: ContentId,
        properties: InstanceProperties,
    ) -> Result<ItemInstanceId, EngineError> {
        validate_properties(&properties)?;

        let id = ItemInstanceId::from_u64(self.next_id).ok_or_else(capacity_error)?;
        let Some(next_id) = self.next_id.checked_add(1) else {
            return Err(capacity_error());
        };

        self.items.insert(
            id,
            ItemInstance {
                id,
                definition,
                properties,
            },
        );
        self.next_id = next_id;
        Ok(id)
    }

    pub fn get(&self, id: ItemInstanceId) -> Option<&ItemInstance> {
        self.items.get(&id)
    }

    pub fn get_mut(&mut self, id: ItemInstanceId) -> Option<&mut ItemInstance> {
        self.items.get_mut(&id)
    }

    pub fn remove(&mut self, id: ItemInstanceId) -> Option<ItemInstance> {
        self.items.remove(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ItemInstance> {
        self.items.values()
    }
}

fn validate_properties(properties: &InstanceProperties) -> Result<(), EngineError> {
    let mut affixes = BTreeSet::new();
    for affix in &properties.affixes {
        if !affixes.insert(&affix.definition) {
            return Err(EngineError::new(
                ErrorCategory::InvalidProperties,
                format!(
                    "duplicate affix `{}` on one item instance",
                    affix.definition
                ),
                "tantalus-core/inventory",
                true,
            ));
        }
    }
    Ok(())
}

fn capacity_error() -> EngineError {
    EngineError::new(
        ErrorCategory::CapacityExhausted,
        "item instance ID space is exhausted",
        "tantalus-core/inventory",
        false,
    )
}
