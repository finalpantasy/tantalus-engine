//! `ModelFactory.CreateModel` path resolution: which container directory a geo name loads from.
//!
//! The model id in a field script, an enemy record or a player row is a key of
//! `FF9BattleDB.GEO`, but the game never loads `Models/<type>/<that id>`. `CreateModel(name)` does
//! this (pinned Memoria `Global/Model/ModelFactory.cs`, commit c716e1fa; the installed
//! `Assembly-CSharp.dll` has the same two tables):
//!
//! 1. `CheckUpscale`: `upscaleTable[name]` replaces the name when it has a row (for example
//!    `GEO_MAIN_B0_000`, the battle Zidane, becomes `GEO_MAIN_UP0_ZDN`). The directory type
//!    (`Models/<type>`) is the second token of that name.
//! 2. `GetRenameModelPath`: `revertUpscaleTable[name]` maps the upscaled name back to a
//!    `FF9BattleDB.GEO` name (`GEO_MAIN_UP0_ZDN` is `GEO_MAIN_F0_ZDN`), else
//!    `GetNameFromFF9DBALL` (special cases for `UP3_ZDN` to `UP5_ZDN` and `UP4_GRN`, then the
//!    same table). The geo id of the result (`GetGEOID`, with two fixed ids for `GEO_MON_B3_110`
//!    and `GEO_MON_B3_109`; otherwise the first key of the name) names the directory
//!    `Models/<type>/<id>/<id>`; `GEO_WEP*` names use type 6. Ids 429 and 430 force type `sub`.
//!    A name without a geo id keeps its name as path (`Models/<type>/<name>/<name>`).
//!
//! So several names share one directory: the party members' battle models load the field model
//! (`GEO_MAIN_F0_ZDN`, `2/98`), which also carries a `battle_model` child that `CreateModel` hides
//! in the field and shows in battle (the `field_model` / `battle_model` child switch at the end of
//! `CreateModel`). `GEO_MAIN_F3_ZDN` to `F5_ZDN` load `2/98` and then replace the textures from
//! `Models/2/<668|669|670>/` (the `ZDN` branch of `CreateModel`); those directories hold textures
//! only, which is why they have no root object.

use super::battle_models::geo_id_by_name;
use serde::Serialize;

mod tables;

/// `ModelType.battle_weapon`.
pub const WEAPON_MODEL_TYPE: u32 = 6;
/// `ModelType.sub`, forced for geo ids 429 and 430.
const SUB_MODEL_TYPE: u32 = 5;

/// `ModelFactory.upscaleTable[name]`.
pub fn upscale_name(name: &str) -> Option<&'static str> {
    lookup(tables::UPSCALE, name)
}

/// `ModelFactory.revertUpscaleTable[name]`.
pub fn revert_name(name: &str) -> Option<&'static str> {
    lookup(tables::REVERT, name)
}

/// Rows in `ModelFactory.upscaleTable` and `ModelFactory.revertUpscaleTable`.
pub fn table_lens() -> (usize, usize) {
    (tables::UPSCALE.len(), tables::REVERT.len())
}

fn lookup(table: &'static [(&'static str, &'static str)], name: &str) -> Option<&'static str> {
    table
        .binary_search_by_key(&name, |(key, _)| *key)
        .ok()
        .map(|index| table[index].1)
}

/// `ModelType` number of a geo name token (`none` is 0).
pub fn model_type_number(token: &str) -> u32 {
    match token.to_ascii_lowercase().as_str() {
        "acc" => 1,
        "main" => 2,
        "mon" => 3,
        "npc" => 4,
        "sub" => 5,
        "battle_weapon" => 6,
        _ => 0,
    }
}

/// Where `CreateModel` looks for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoadTarget {
    /// The numeric directory `<type>/<geo id>`.
    Directory { model_type: u32, geo_id: u32 },
    /// The renamed name has no `FF9BattleDB.GEO` key, so the path keeps the name.
    ByName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelLoad {
    /// The name `CreateModel` was called with.
    pub requested: String,
    /// The name after `upscaleTable`; equals `requested` when the table has no row.
    pub upscaled: String,
    /// The `FF9BattleDB.GEO` name after the revert step.
    pub loaded_name: String,
    pub target: LoadTarget,
}

impl ModelLoad {
    /// Catalog label of the directory (`<type>/<geo id>`), when there is one.
    pub fn label(&self) -> Option<String> {
        match self.target {
            LoadTarget::Directory { model_type, geo_id } => Some(format!("{model_type}/{geo_id}")),
            LoadTarget::ByName => None,
        }
    }

    /// True when the loaded directory belongs to a different geo name than the requested one.
    pub fn is_shared(&self) -> bool {
        self.loaded_name != self.requested
    }
}

/// `GetNameFromFF9DBALL`: the special names, then `revertUpscaleTable`.
fn name_from_ff9dball(name: &str) -> &str {
    let special = match name {
        "GEO_MAIN_UP3_ZDN" | "GEO_MAIN_UP3_ZDN_0" | "GEO_MAIN_UP3_ZDN_1" => "GEO_MAIN_F3_ZDN",
        "GEO_MAIN_UP4_ZDN" | "GEO_MAIN_UP4_ZDN_0" | "GEO_MAIN_UP4_ZDN_1" => "GEO_MAIN_F4_ZDN",
        "GEO_MAIN_UP5_ZDN" | "GEO_MAIN_UP5_ZDN_0" | "GEO_MAIN_UP5_ZDN_1" => "GEO_MAIN_F5_ZDN",
        "GEO_MAIN_UP4_GRN" => "GEO_MAIN_F4_GRN",
        other => other,
    };
    revert_name(special).unwrap_or(special)
}

/// `GetGEOID` (`-1` is `None`).
fn geo_id_of(name: &str) -> Option<u32> {
    match name {
        "GEO_MON_B3_110" => Some(347),
        "GEO_MON_B3_109" => Some(5461),
        other => geo_id_by_name(other),
    }
}

/// Resolves a `GEO_<type>_<variant>_<name>` model name the way `CreateModel` does.
pub fn model_load(name: &str) -> ModelLoad {
    let upscaled = upscale_name(name).unwrap_or(name);
    let type_token = upscaled.split('_').nth(1).unwrap_or("");
    let loaded_name = match revert_name(upscaled) {
        Some(reverted) => reverted,
        None => name_from_ff9dball(upscaled),
    };
    let target = match geo_id_of(loaded_name) {
        None => LoadTarget::ByName,
        Some(geo_id) => {
            let model_type = if loaded_name.starts_with("GEO_WEP") {
                WEAPON_MODEL_TYPE
            } else if geo_id == 429 || geo_id == 430 {
                SUB_MODEL_TYPE
            } else {
                model_type_number(type_token)
            };
            LoadTarget::Directory { model_type, geo_id }
        }
    };
    ModelLoad {
        requested: name.to_owned(),
        upscaled: upscaled.to_owned(),
        loaded_name: loaded_name.to_owned(),
        target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_have_the_reference_row_counts() {
        assert_eq!(table_lens(), (174, 107));
        assert!(tables::UPSCALE.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(tables::REVERT.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn battle_zidane_loads_the_field_model_directory() {
        let load = model_load("GEO_MAIN_B0_000");
        assert_eq!(load.upscaled, "GEO_MAIN_UP0_ZDN");
        assert_eq!(load.loaded_name, "GEO_MAIN_F0_ZDN");
        assert_eq!(
            load.target,
            LoadTarget::Directory {
                model_type: 2,
                geo_id: 98
            }
        );
        assert_eq!(load.label().as_deref(), Some("2/98"));
        assert!(load.is_shared());
    }

    #[test]
    fn upscale_rows_reach_a_different_type_and_id() {
        // A field alias of a battle monster: type 3, the monster's id.
        let load = model_load("GEO_MON_F0_ZZZ");
        assert_eq!(load.upscaled, "GEO_MON_UP0_124");
        assert_eq!(load.label().as_deref(), Some("3/363"));
        // A `sub` name that loads a `mon` directory: the type is the upscaled name's.
        let load = model_load("GEO_SUB_F0_ZON");
        assert_eq!(load.label().as_deref(), Some("3/302"));
    }

    #[test]
    fn names_without_a_row_load_their_own_id() {
        let load = model_load("GEO_NPC_F1_BBA");
        assert_eq!(load.upscaled, "GEO_NPC_F1_BBA");
        assert_eq!(load.loaded_name, "GEO_NPC_F1_BBA");
        assert!(!load.is_shared());
        assert_eq!(load.label().as_deref(), Some("4/10"));
    }

    #[test]
    fn duplicate_geo_names_use_the_first_key() {
        // `FF9BattleDB.GEO` lists 5461 before 33 and 347 before 201 for these names.
        assert_eq!(
            model_load("GEO_MON_F0_EFM").label().as_deref(),
            Some("3/5461")
        );
        assert_eq!(
            model_load("GEO_MON_F2_EFM").label().as_deref(),
            Some("3/347")
        );
    }

    #[test]
    fn zidane_texture_variants_load_the_base_model() {
        for name in ["GEO_MAIN_F3_ZDN", "GEO_MAIN_F4_ZDN", "GEO_MAIN_F5_ZDN"] {
            assert_eq!(model_load(name).label().as_deref(), Some("2/98"), "{name}");
        }
        assert_eq!(name_from_ff9dball("GEO_MAIN_UP4_ZDN_1"), "GEO_MAIN_F4_ZDN");
        assert_eq!(name_from_ff9dball("GEO_MAIN_UP4_GRN"), "GEO_MAIN_F4_GRN");
    }

    #[test]
    fn weapons_use_the_weapon_type_and_unknown_names_load_by_name() {
        let unknown = model_load("GEO_MON_B3_999");
        assert_eq!(unknown.target, LoadTarget::ByName);
        assert_eq!(unknown.label(), None);
        // Any `GEO_WEP*` name that has a geo id is a type 6 directory.
        assert_eq!(
            model_load("GEO_WEP_B1_011").label().as_deref(),
            Some("6/37")
        );
    }

    #[test]
    fn type_tokens_map_to_model_types() {
        assert_eq!(model_type_number("ACC"), 1);
        assert_eq!(model_type_number("main"), 2);
        assert_eq!(model_type_number("sub"), 5);
        assert_eq!(model_type_number("WEP"), 0);
    }
}
