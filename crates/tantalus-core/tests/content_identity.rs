use serde_json::{Value, json};
use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, InstanceProperties, PropertyValue};
use tantalus_core::registry::{ContentAlias, ContentRename};
use tantalus_core::{Inventory, Registry, RegistryPatch, RegistryRecord};

fn id(text: &str) -> ContentId {
    ContentId::parse(text).unwrap()
}

fn record(text: &str, value: Value) -> RegistryRecord {
    RegistryRecord::new(id(text), value).with_source("synthetic-fixture", format!("{text}.json"))
}

fn affix(text: &str, power: i64) -> tantalus_contracts::AffixRoll {
    tantalus_contracts::AffixRoll {
        definition: id(text),
        resolved_values: BTreeMap::from([(
            id("my_mod:property/power"),
            PropertyValue::Integer(power),
        )]),
    }
}

#[test]
fn additive_content_preserves_original_records() {
    let vanilla = record("ff9:item/potion", json!({"healing": 100}));
    let original_data = vanilla.data.clone();
    let mut registry = Registry::new();
    registry.apply(RegistryPatch::add(vanilla)).unwrap();

    registry
        .apply(RegistryPatch::add(record(
            "my_mod:weapon/raptor_talon",
            json!({"damage": 37}),
        )))
        .unwrap();

    assert_eq!(registry.len(), 2);
    assert_eq!(
        registry.get(&id("ff9:item/potion")).unwrap().data,
        original_data
    );
    assert_eq!(
        registry
            .get(&id("my_mod:weapon/raptor_talon"))
            .unwrap()
            .data["damage"],
        37
    );
}

#[test]
fn duplicate_addition_rolls_back_the_entire_patch() {
    let mut registry = Registry::new();
    registry
        .apply(RegistryPatch::add(record("ff9:item/potion", json!({}))))
        .unwrap();
    let original_revision = registry.revision();

    let report = registry
        .apply(RegistryPatch {
            additions: vec![
                record("my_mod:item/valid_but_rolled_back", json!({})),
                record(
                    "ff9:item/potion",
                    json!({"replacement": "not allowed by addition"}),
                ),
            ],
            ..RegistryPatch::default()
        })
        .unwrap_err();

    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "duplicate-content-id")
    );
    assert_eq!(registry.revision(), original_revision);
    assert_eq!(registry.len(), 1);
    assert!(
        registry
            .get(&id("my_mod:item/valid_but_rolled_back"))
            .is_none()
    );
}

#[test]
fn replacement_is_explicit_and_aliases_resolve_to_current_content() {
    let mut registry = Registry::new();
    registry
        .apply(RegistryPatch::add(record(
            "ff9:item/potion",
            json!({"healing": 100}),
        )))
        .unwrap();
    registry
        .apply(RegistryPatch {
            replacements: vec![record("ff9:item/potion", json!({"healing": 250}))],
            aliases: vec![ContentAlias {
                alias: id("my_mod:item/strong_potion"),
                target: id("ff9:item/potion"),
            }],
            ..RegistryPatch::default()
        })
        .unwrap();

    assert_eq!(
        registry.get(&id("my_mod:item/strong_potion")).unwrap().data["healing"],
        250
    );
}

#[test]
fn rename_retains_the_old_identity_as_an_alias() {
    let mut registry = Registry::new();
    registry
        .apply(RegistryPatch::add(record(
            "ff9:enemy/old_name",
            json!({"hp": 42}),
        )))
        .unwrap();
    registry
        .apply(RegistryPatch {
            renames: vec![ContentRename {
                from: id("ff9:enemy/old_name"),
                to: id("my_mod:enemy/new_name"),
            }],
            ..RegistryPatch::default()
        })
        .unwrap();

    assert_eq!(
        registry.get(&id("ff9:enemy/old_name")).unwrap().id,
        id("my_mod:enemy/new_name")
    );
    assert_eq!(
        registry.get(&id("my_mod:enemy/new_name")).unwrap().data["hp"],
        42
    );
    assert!(registry.aliases().any(|(alias, target)| {
        *alias == id("ff9:enemy/old_name") && *target == id("my_mod:enemy/new_name")
    }));
}

#[test]
fn aliases_cannot_chain_or_target_missing_content() {
    let mut registry = Registry::new();
    registry
        .apply(RegistryPatch::add(record("ff9:item/potion", json!({}))))
        .unwrap();
    registry
        .apply(RegistryPatch {
            aliases: vec![ContentAlias {
                alias: id("my_mod:item/drink"),
                target: id("ff9:item/potion"),
            }],
            ..RegistryPatch::default()
        })
        .unwrap();
    let revision = registry.revision();

    let report = registry
        .apply(RegistryPatch {
            aliases: vec![
                ContentAlias {
                    alias: id("my_mod:item/chained"),
                    target: id("my_mod:item/drink"),
                },
                ContentAlias {
                    alias: id("my_mod:item/missing"),
                    target: id("my_mod:item/absent"),
                },
            ],
            ..RegistryPatch::default()
        })
        .unwrap_err();

    assert_eq!(report.issues.len(), 2);
    assert!(
        report
            .issues
            .iter()
            .all(|issue| issue.code == "alias-target-missing")
    );
    assert_eq!(registry.revision(), revision);
}

#[test]
fn registry_results_do_not_depend_on_file_enumeration_order() {
    let records = vec![
        record("ff9:item/potion", json!({"index": 0})),
        record("my_mod:enemy/iron_raptor", json!({"index": 1})),
        record("my_mod:weapon/raptor_talon", json!({"index": 2})),
        record("my_mod:status/bleeding", json!({"index": 3})),
    ];
    let mut left = Registry::new();
    let mut right = Registry::new();

    left.apply(RegistryPatch {
        additions: records.clone(),
        ..RegistryPatch::default()
    })
    .unwrap();
    right
        .apply(RegistryPatch {
            additions: records.into_iter().rev().collect(),
            ..RegistryPatch::default()
        })
        .unwrap();

    assert_eq!(left.revision(), right.revision());
    assert_eq!(
        left.records().collect::<Vec<_>>(),
        right.records().collect::<Vec<_>>()
    );
}

#[test]
fn item_instances_keep_independent_rolls_custom_stats_and_slots() {
    let definition = id("my_mod:weapon/raptor_talon");
    let custom_stat = id("my_mod:stat/bleed_power");
    let custom_slot = id("my_mod:slot/talon");
    let first_properties = InstanceProperties {
        affixes: vec![affix("my_mod:affix/sharp", 7)],
        stats: BTreeMap::from([(custom_stat.clone(), PropertyValue::Integer(12))]),
        slot: Some(custom_slot.clone()),
        custom: BTreeMap::from([(
            id("my_mod:property/creator"),
            PropertyValue::Text("synthetic".to_owned()),
        )]),
    };
    let second_properties = InstanceProperties {
        affixes: vec![affix("my_mod:affix/heavy", 19)],
        stats: BTreeMap::from([(custom_stat.clone(), PropertyValue::Integer(4))]),
        slot: Some(custom_slot),
        ..InstanceProperties::default()
    };
    let mut inventory = Inventory::new();

    let first = inventory
        .spawn(definition.clone(), first_properties)
        .unwrap();
    let second = inventory.spawn(definition, second_properties).unwrap();

    assert_ne!(first, second);
    assert_eq!(first.get(), 1);
    assert_eq!(second.get(), 2);
    assert_eq!(
        inventory.get(first).unwrap().properties().affixes[0].definition,
        id("my_mod:affix/sharp")
    );
    assert_eq!(
        inventory.get(second).unwrap().properties().affixes[0].definition,
        id("my_mod:affix/heavy")
    );

    inventory
        .get_mut(first)
        .unwrap()
        .properties_mut()
        .stats
        .insert(custom_stat.clone(), PropertyValue::Integer(99));
    assert_eq!(
        inventory.get(second).unwrap().properties().stats[&custom_stat],
        PropertyValue::Integer(4)
    );

    let encoded = serde_json::to_string(&inventory).unwrap();
    let restored: Inventory = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.get(first).unwrap(), inventory.get(first).unwrap());
    assert_eq!(
        restored.get(second).unwrap(),
        inventory.get(second).unwrap()
    );
}

#[test]
fn invalid_properties_do_not_consume_an_instance_id() {
    let mut inventory = Inventory::new();
    inventory
        .spawn(
            id("my_mod:weapon/raptor_talon"),
            InstanceProperties::default(),
        )
        .unwrap();

    let invalid = InstanceProperties {
        affixes: vec![
            affix("my_mod:affix/sharp", 7),
            affix("my_mod:affix/sharp", 8),
        ],
        ..InstanceProperties::default()
    };
    let error = inventory
        .spawn(id("my_mod:weapon/raptor_talon"), invalid)
        .unwrap_err();
    assert!(error.context.contains("duplicate affix"));
    assert_eq!(inventory.len(), 1);

    let next = inventory
        .spawn(
            id("my_mod:weapon/raptor_talon"),
            InstanceProperties::default(),
        )
        .unwrap();
    assert_eq!(next.get(), 2);
}

#[test]
fn rename_updates_existing_aliases_without_creating_alias_chains() {
    let mut registry = Registry::new();
    registry
        .apply(RegistryPatch {
            additions: vec![record("ff9:enemy/old_name", json!({"hp": 42}))],
            aliases: vec![ContentAlias {
                alias: id("my_mod:enemy/familiar_name"),
                target: id("ff9:enemy/old_name"),
            }],
            ..RegistryPatch::default()
        })
        .unwrap();
    registry
        .apply(RegistryPatch {
            renames: vec![ContentRename {
                from: id("ff9:enemy/old_name"),
                to: id("my_mod:enemy/new_name"),
            }],
            ..RegistryPatch::default()
        })
        .unwrap();

    let resolved = registry.get(&id("my_mod:enemy/familiar_name")).unwrap();
    assert_eq!(resolved.id, id("my_mod:enemy/new_name"));
    assert!(registry.aliases().any(|(alias, target)| {
        *alias == id("my_mod:enemy/familiar_name") && *target == id("my_mod:enemy/new_name")
    }));
}

#[test]
fn removed_instance_ids_are_not_reused() {
    let mut inventory = Inventory::new();
    let first = inventory
        .spawn(id("ff9:item/potion"), InstanceProperties::default())
        .unwrap();
    inventory
        .spawn(id("ff9:item/potion"), InstanceProperties::default())
        .unwrap();
    assert!(inventory.remove(first).is_some());

    let third = inventory
        .spawn(id("ff9:item/potion"), InstanceProperties::default())
        .unwrap();
    assert_eq!(third.get(), 3);
}
