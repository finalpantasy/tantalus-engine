//! Party, default equipment, command menu layout, default names and text
//! bindings read from the installed assembly.
//!
//! Unlike the static tables in the parent module, most of these live inside
//! method bodies (`ff9play.FF9Play_New`, `FF9Play_SetDefEquips`,
//! `FF9Play_GetCategory`, `FF9Play_GetCharID3`, `ff9item.FF9Item_Init`,
//! `FF9TextTool.CharacterDefaultName`) as array literals, or are the result
//! of a pure integer function (`FF9Play_GetCharID`,
//! `BattleHUD.GetCommandFromCommandIndex`). They are read with
//! [`run_method`], which follows branches on integer constants and stops, with
//! a reason, at anything else; no game code runs. Rank-2 menu arrays come from
//! `rdata`'s static constructor.
//!
//! What each table is, as the installed IL shows it:
//!
//! | Table | Source |
//! | --- | --- |
//! | command menus | `rdata._FF9BMenu_MenuNormal` `[20,2]`, `_FF9BMenu_MenuTrance` `[20,3]` |
//! | player slots | `ff9play.FF9Play_New`, a `PLAYER_INFO[9]` literal |
//! | menu type to character id | `ff9play.FF9Play_GetCharID` evaluated for 0..19 |
//! | category by character id | `ff9play.FF9Play_GetCategory`, a `byte[12]` literal |
//! | equipment set by slot | `ff9play.FF9Play_GetCharID3` / `GetCharID2`, `byte[9][2]` literals |
//! | default equipment | `ff9play.FF9Play_SetDefEquips`, a `byte[16][5]` literal |
//! | new-game calls | `ff9play.FF9Play_Init` run with integer loop counters |
//! | initial items | `ff9item.FF9Item_Init`, a `FF9ITEM[]` literal |
//! | default names | `FF9TextTool.CharacterDefaultName`, one `string[20]` per language |
//! | text bindings | `FF9TextTool` accessors and the `Initialize*Text` iterators |

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::super::managed_il::{
    Assembly, MethodRun, Resolver, RunOptions, Tree, instructions, run_method,
};
use super::{Result, TableError};

/// One row of the two command menu tables for a `menu_type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandMenuRow {
    pub menu_type: u8,
    /// `_FF9BMenu_MenuNormal[menu_type, 0..2]`: the two class commands.
    pub normal: [u8; 2],
    /// `_FF9BMenu_MenuTrance[menu_type, 0..2]`.
    pub trance: [u8; 2],
    /// `_FF9BMenu_MenuTrance[menu_type, 2]`; `BattleHUD.SetAbilityTrance`
    /// compares it with 1 and subtracts 1 from it. Kept raw.
    pub trance_third: u8,
}

/// One `PLAYER_INFO` of the `FF9Play_New` literal, plus the values the same
/// installed methods derive from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSlotRow {
    pub slot_no: u8,
    pub serial_no: u8,
    pub row: u8,
    pub win_pose: u8,
    pub party: u8,
    pub menu_type: u8,
    /// `FF9Play_GetCharID(menu_type)`.
    pub char_id: i32,
    /// `FF9Play_GetCategory`'s table at `char_id`.
    pub category: u8,
    /// `GetCharID3` table at `[slot_no][(category & 0x10) != 0]`.
    pub equipment_set: u8,
    /// `CharacterDefaultName(menu_type)` for `English(US)`.
    pub default_name_en_us: String,
}

/// A `byte[5]` of `FF9Play_SetDefEquips`. 255 is `FF9PLAY_EQUIP_ID_NONE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentSetRow {
    pub id: u8,
    pub weapon: u8,
    pub head: u8,
    pub wrist: u8,
    pub armor: u8,
    pub accessory: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialItemRow {
    pub id: u16,
    pub count: u8,
}

/// `FF9Play_SetParty(party_slot, player_slot)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyAssignment {
    pub party_slot: i64,
    pub player_slot: i64,
}

/// What `FF9Play_Init` does, from running its integer control flow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewGameInit {
    /// Arguments of every `FF9Play_New` call, in order.
    pub players_built: Vec<i64>,
    /// Arguments of every `FF9Play_Add` call, in order.
    pub players_added: Vec<i64>,
    pub party: Vec<PartyAssignment>,
    /// `party.gil` store.
    pub gil: Option<i64>,
    /// `party.summon_flag` store.
    pub summon_flag: Option<i64>,
}

/// One entry of `BattleHUD.GetCommandFromCommandIndex`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSlotRow {
    pub index: u8,
    /// The command id when the method returns a constant for this index.
    pub command: Option<u8>,
    /// Why evaluation stopped when it did not return a constant.
    pub stopped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageNames {
    /// The case value `CharacterDefaultName` switches on.
    pub index: u8,
    pub language: String,
    /// Indexed by `menu_type`.
    pub names: Vec<String>,
}

/// A localized text array: which accessor reads it, and which `.mes` file
/// fills it (relative to `text_root` plus a language symbol).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextBinding {
    pub accessor: String,
    pub array_field: String,
    pub mes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerTables {
    pub command_menus: Vec<CommandMenuRow>,
    pub command_slots: Vec<CommandSlotRow>,
    pub player_slots: Vec<PlayerSlotRow>,
    pub menu_type_char_ids: Vec<i32>,
    pub categories: Vec<u8>,
    pub equipment_set_by_slot: Vec<[u8; 2]>,
    pub default_equipment: Vec<EquipmentSetRow>,
    pub new_game: NewGameInit,
    pub initial_items: Vec<InitialItemRow>,
    pub ff9play_constants: BTreeMap<String, i64>,
    pub player_constants: BTreeMap<String, i64>,
    pub default_names: Vec<LanguageNames>,
    pub text_root: String,
    pub text_bindings: Vec<TextBinding>,
}

const TABLE: &str = "player";

fn fail(detail: impl Into<String>) -> TableError {
    TableError::new(TABLE, detail)
}

fn body(assembly: &Assembly<'_>, class: &str, method: &str) -> Result<Vec<u8>> {
    let row = assembly
        .find_type(class)
        .map_err(|e| fail(e.to_string()))?
        .ok_or_else(|| fail(format!("type `{class}` not found")))?;
    assembly
        .method_body(row, method)
        .map_err(|e| fail(e.to_string()))?
        .ok_or_else(|| fail(format!("`{class}::{method}` has no body")))
}

fn run(
    assembly: &Assembly<'_>,
    class: &str,
    method: &str,
    options: &RunOptions,
) -> Result<MethodRun> {
    let code = body(assembly, class, method)?;
    run_method(&code, assembly, options).map_err(|e| fail(format!("{class}::{method}: {e}")))
}

fn ints(what: &str, tree: &Tree) -> Result<Vec<i64>> {
    let Tree::Array { items, .. } = tree else {
        return Err(fail(format!("{what}: expected an array")));
    };
    items
        .iter()
        .map(|item| {
            item.as_int()
                .ok_or_else(|| fail(format!("{what}: element is not a constant integer")))
        })
        .collect()
}

fn byte(what: &str, value: i64) -> Result<u8> {
    u8::try_from(value).map_err(|_| fail(format!("{what}: {value} does not fit a byte")))
}

/// The one local that holds an array whose element type is `element`.
fn array_local<'a>(run: &'a MethodRun, what: &str, element: &str) -> Result<&'a [Tree]> {
    let mut found = run.locals.values().filter_map(|tree| match tree {
        Tree::Array {
            element: e, items, ..
        } if e == element && !items.is_empty() => Some(items.as_slice()),
        _ => None,
    });
    let first = found
        .next()
        .ok_or_else(|| fail(format!("{what}: no local array of `{element}`")))?;
    for other in found {
        if other != first {
            return Err(fail(format!(
                "{what}: more than one distinct local array of `{element}`"
            )));
        }
    }
    Ok(first)
}

/// A run that must reach `ret` (its result is a constant, not a literal).
fn require_finished(run: &MethodRun, what: &str) -> Result<()> {
    match &run.stop {
        Some(stop) => Err(fail(format!(
            "{what}: stopped at {:#x}: {}",
            stop.offset, stop.reason
        ))),
        None => Ok(()),
    }
}

fn command_menus(rdata: &BTreeMap<String, Tree>) -> Result<Vec<CommandMenuRow>> {
    fn matrix(rdata: &BTreeMap<String, Tree>, name: &str, columns: usize) -> Result<Vec<Vec<u8>>> {
        let tree = rdata
            .get(name)
            .ok_or_else(|| fail(format!("rdata.{name} was not set")))?;
        if tree.contains_unsupported() {
            return Err(fail(format!("rdata.{name} holds unsupported values")));
        }
        let Tree::Array { items, .. } = tree else {
            return Err(fail(format!("rdata.{name} is not an array")));
        };
        items
            .iter()
            .map(|row| {
                let values = ints(name, row)?;
                if values.len() != columns {
                    return Err(fail(format!("rdata.{name}: row is not {columns} wide")));
                }
                values.iter().map(|v| byte(name, *v)).collect()
            })
            .collect()
    }
    let normal = matrix(rdata, "_FF9BMenu_MenuNormal", 2)?;
    let trance = matrix(rdata, "_FF9BMenu_MenuTrance", 3)?;
    if normal.len() != trance.len() {
        return Err(fail("the two command menu tables differ in length"));
    }
    normal
        .iter()
        .zip(&trance)
        .enumerate()
        .map(|(index, (n, t))| {
            Ok(CommandMenuRow {
                menu_type: u8::try_from(index).map_err(|_| fail("too many menu types"))?,
                normal: [n[0], n[1]],
                trance: [t[0], t[1]],
                trance_third: t[2],
            })
        })
        .collect()
}

fn jagged(what: &str, items: &[Tree], width: usize) -> Result<Vec<Vec<u8>>> {
    items
        .iter()
        .map(|row| {
            let values = ints(what, row)?;
            if values.len() != width {
                return Err(fail(format!("{what}: row is not {width} wide")));
            }
            values.iter().map(|v| byte(what, *v)).collect()
        })
        .collect()
}

fn names(assembly: &Assembly<'_>) -> Result<Vec<LanguageNames>> {
    let code = body(assembly, "FF9TextTool", "CharacterDefaultName")?;
    let all = instructions(&code).map_err(|e| fail(e.to_string()))?;
    // The language dictionary is built right before its first `.ctor` call.
    let mut dictionary_entry = None;
    for (index, instruction) in all.iter().enumerate() {
        if instruction.opcode == 0x73 {
            let info = Resolver::method(assembly, instruction.operand.unwrap_or(0))
                .map_err(|e| fail(e.to_string()))?;
            if info
                .owner
                .starts_with("System.Collections.Generic.Dictionary`")
            {
                dictionary_entry = index.checked_sub(1).map(|i| all[i]);
                break;
            }
        }
    }
    let entry = dictionary_entry.ok_or_else(|| fail("CharacterDefaultName: no language map"))?;
    let map_field = "<>f__switch$map15".to_owned();
    let first = run(
        assembly,
        "FF9TextTool",
        "CharacterDefaultName",
        &RunOptions {
            entry: entry.offset,
            null_statics: vec![map_field.clone()],
            ..RunOptions::default()
        },
    )?;
    let Some(Tree::Map(pairs)) = first.statics.get(&map_field) else {
        return Err(fail("CharacterDefaultName: language map was not built"));
    };
    let switch = all
        .iter()
        .find(|i| i.opcode == 0x45)
        .ok_or_else(|| fail("CharacterDefaultName: no switch"))?;
    let mut out = Vec::new();
    for (key, value) in pairs {
        let (Tree::Str(language), Some(case)) = (key, value.as_int()) else {
            return Err(fail("CharacterDefaultName: language map entry"));
        };
        let result = run(
            assembly,
            "FF9TextTool",
            "CharacterDefaultName",
            &RunOptions {
                entry: switch.offset,
                stack: vec![case],
                ..RunOptions::default()
            },
        )?;
        let strings = result
            .locals
            .values()
            .find_map(|tree| match tree {
                Tree::Array { element, items } if element == "System.String" => Some(items),
                _ => None,
            })
            .ok_or_else(|| fail(format!("no name array for language case {case}")))?;
        let list = strings
            .iter()
            .map(|s| match s {
                Tree::Str(text) => Ok(text.clone()),
                _ => Err(fail("default name is not a string")),
            })
            .collect::<Result<Vec<_>>>()?;
        out.push(LanguageNames {
            index: byte("language case", case)?,
            language: language.clone(),
            names: list,
        });
    }
    out.sort_by_key(|n| n.index);
    Ok(out)
}

fn text_bindings(assembly: &Assembly<'_>) -> Result<(String, Vec<TextBinding>)> {
    // `Initialize*Text` iterators: every `ldstr "...mes"` is followed by the
    // `stsfld` of the array it fills.
    let mut mes_by_field: BTreeMap<String, String> = BTreeMap::new();
    for type_name in assembly.type_names().map_err(|e| fail(e.to_string()))? {
        if !(type_name.starts_with("<Initialize")
            && type_name.contains("Text>c__Iterator")
            && !type_name.starts_with("<InitializeFieldText>")
            && !type_name.starts_with("<InitializeBattleText>")
            && !type_name.starts_with("<InitializeLocationText>"))
        {
            continue;
        }
        let code = body(assembly, &type_name, "MoveNext")?;
        let all = instructions(&code).map_err(|e| fail(e.to_string()))?;
        let mut pending: Option<String> = None;
        for instruction in &all {
            let Some(token) = instruction.operand else {
                continue;
            };
            match instruction.opcode {
                0x72 => {
                    let text =
                        Resolver::user_string(assembly, token).map_err(|e| fail(e.to_string()))?;
                    if text.ends_with(".mes") {
                        pending = Some(text);
                    }
                }
                0x80 => {
                    let field =
                        Resolver::field_name(assembly, token).map_err(|e| fail(e.to_string()))?;
                    if let Some(mes) = pending.take() {
                        mes_by_field.insert(field, mes);
                    }
                }
                _ => {}
            }
        }
    }
    let accessors = [
        "ItemName",
        "ItemHelpDescription",
        "ItemBattleDescription",
        "ImportantItemName",
        "ImportantItemHelpDescription",
        "ImportantItemSkin",
        "ActionAbilityName",
        "ActionAbilityHelpDescription",
        "SupportAbilityName",
        "SupportAbilityHelpDescription",
        "CommandName",
        "CommandHelpDescription",
    ];
    let mut bindings = Vec::new();
    for accessor in accessors {
        let code = body(assembly, "FF9TextTool", accessor)?;
        let all = instructions(&code).map_err(|e| fail(e.to_string()))?;
        let mut fields: Vec<String> = Vec::new();
        for instruction in all.iter().filter(|i| i.opcode == 0x7E) {
            let name = Resolver::field_name(assembly, instruction.operand.unwrap_or(0))
                .map_err(|e| fail(e.to_string()))?;
            if !name.ends_with(".Empty") && !fields.contains(&name) {
                fields.push(name);
            }
        }
        let [field] = fields.as_slice() else {
            return Err(fail(format!(
                "FF9TextTool.{accessor} reads {} static arrays, expected one",
                fields.len()
            )));
        };
        let mes = mes_by_field
            .get(field)
            .ok_or_else(|| fail(format!("no `.mes` file fills `{field}`")))?;
        bindings.push(TextBinding {
            accessor: accessor.to_owned(),
            array_field: field.clone(),
            mes: mes.clone(),
        });
    }
    let path = body(assembly, "Localization", "GetPath")?;
    let root = instructions(&path)
        .map_err(|e| fail(e.to_string()))?
        .iter()
        .find(|i| i.opcode == 0x72)
        .map(|i| Resolver::user_string(assembly, i.operand.unwrap_or(0)))
        .transpose()
        .map_err(|e| fail(e.to_string()))?
        .ok_or_else(|| fail("Localization.GetPath has no string root"))?;
    Ok((root, bindings))
}

/// Read every table in this module.
pub fn extract_player_tables(assembly: &Assembly<'_>) -> Result<PlayerTables> {
    // Constants other classes expose to `rdata`'s constructor.
    let com = assembly
        .static_initializer("FF9.ff9com")
        .map_err(|e| fail(e.to_string()))?;
    let seeds: BTreeMap<String, i64> = com
        .iter()
        .filter_map(|(name, tree)| tree.as_int().map(|v| (name.clone(), v)))
        .collect();
    let rdata = assembly
        .static_initializer_seeded("rdata", &seeds)
        .map_err(|e| fail(e.to_string()))?;
    let command_menus = command_menus(&rdata)?;

    // Player slots.
    let new_run = run(assembly, "ff9play", "FF9Play_New", &RunOptions::default())?;
    let slot_trees = array_local(&new_run, "FF9Play_New", "PLAYER_INFO")?;
    let mut raw_slots = Vec::new();
    for tree in slot_trees {
        let Tree::Object { type_name, args } = tree else {
            return Err(fail("PLAYER_INFO element is not a constructor call"));
        };
        if type_name != "PLAYER_INFO" {
            return Err(fail(format!("unexpected element type `{type_name}`")));
        }
        let get = |name: &str| -> Result<u8> {
            let value = args
                .iter()
                .find(|(n, _)| n == name)
                .and_then(|(_, t)| t.as_int())
                .ok_or_else(|| fail(format!("PLAYER_INFO has no constant `{name}`")))?;
            byte(name, value)
        };
        raw_slots.push([
            get("slot_no")?,
            get("serial_no")?,
            get("row")?,
            get("win_pose")?,
            get("party")?,
            get("menu_type")?,
        ]);
    }

    // The calls FF9Play_New makes confirm which helpers feed its fields.
    for needed in [
        "FF9Play_GetCategory",
        "FF9Play_GetCharID3",
        "FF9Play_SetDefEquips",
        "CharacterDefaultName",
    ] {
        if !new_run.calls.iter().any(|c| c.name == needed) {
            return Err(fail(format!("FF9Play_New does not call {needed}")));
        }
    }

    // menu_type -> character id.
    let mut menu_type_char_ids = Vec::new();
    for menu_type in 0..command_menus.len() {
        let result = run(
            assembly,
            "ff9play",
            "FF9Play_GetCharID",
            &RunOptions {
                args: vec![Some(menu_type as i64)],
                ..RunOptions::default()
            },
        )?;
        require_finished(&result, "FF9Play_GetCharID")?;
        let id = result
            .returned
            .as_ref()
            .and_then(Tree::as_int)
            .ok_or_else(|| fail("FF9Play_GetCharID did not return a constant"))?;
        menu_type_char_ids.push(
            i32::try_from(id).map_err(|_| fail("FF9Play_GetCharID result does not fit i32"))?,
        );
    }

    // Category by character id.
    let category_run = run(
        assembly,
        "ff9play",
        "FF9Play_GetCategory",
        &RunOptions::default(),
    )?;
    if !category_run
        .calls
        .iter()
        .any(|c| c.name == "FF9Play_GetCharID")
    {
        return Err(fail(
            "FF9Play_GetCategory does not index by FF9Play_GetCharID",
        ));
    }
    let categories: Vec<u8> = array_local(&category_run, "FF9Play_GetCategory", "System.Byte")?
        .iter()
        .map(|t| byte("category", t.as_int().unwrap_or(-1)))
        .collect::<Result<_>>()?;

    // Equipment set by slot (GetCharID3, with GetCharID2 required to agree).
    let mut tables = Vec::new();
    for method in ["FF9Play_GetCharID3", "FF9Play_GetCharID2"] {
        let result = run(assembly, "ff9play", method, &RunOptions::default())?;
        let rows = array_local(&result, method, "<typespec>")?;
        tables.push(jagged(method, rows, 2)?);
    }
    if tables[0] != tables[1] {
        return Err(fail("GetCharID2 and GetCharID3 tables differ"));
    }
    let equipment_set_by_slot: Vec<[u8; 2]> =
        tables[0].iter().map(|row| [row[0], row[1]]).collect();

    // Default equipment.
    let equip_run = run(
        assembly,
        "ff9play",
        "FF9Play_SetDefEquips",
        &RunOptions::default(),
    )?;
    let default_equipment = jagged(
        "FF9Play_SetDefEquips",
        array_local(&equip_run, "FF9Play_SetDefEquips", "<typespec>")?,
        5,
    )?
    .iter()
    .enumerate()
    .map(|(id, set)| {
        Ok(EquipmentSetRow {
            id: u8::try_from(id).map_err(|_| fail("too many equipment sets"))?,
            weapon: set[0],
            head: set[1],
            wrist: set[2],
            armor: set[3],
            accessory: set[4],
        })
    })
    .collect::<Result<Vec<_>>>()?;

    // Names.
    let default_names = names(assembly)?;
    let en_us = default_names
        .iter()
        .find(|n| n.language == "English(US)")
        .ok_or_else(|| fail("no English(US) default names"))?;

    // Slots with derived values.
    let mut player_slots = Vec::new();
    for [slot_no, serial_no, row, win_pose, party, menu_type] in raw_slots {
        let char_id = *menu_type_char_ids
            .get(usize::from(menu_type))
            .ok_or_else(|| fail("menu_type outside the menu tables"))?;
        let category = *usize::try_from(char_id)
            .ok()
            .and_then(|i| categories.get(i))
            .ok_or_else(|| fail("character id outside the category table"))?;
        let sets = equipment_set_by_slot
            .get(usize::from(slot_no))
            .ok_or_else(|| fail("slot outside the equipment-set table"))?;
        let equipment_set = sets[usize::from(category & 0x10 != 0)];
        let default_name_en_us = en_us
            .names
            .get(usize::from(menu_type))
            .cloned()
            .ok_or_else(|| fail("menu_type outside the default name table"))?;
        player_slots.push(PlayerSlotRow {
            slot_no,
            serial_no,
            row,
            win_pose,
            party,
            menu_type,
            char_id,
            category,
            equipment_set,
            default_name_en_us,
        });
    }

    // FF9Play_Init.
    let init = run(assembly, "ff9play", "FF9Play_Init", &RunOptions::default())?;
    require_finished(&init, "FF9Play_Init")?;
    let ints_of = |name: &str| -> Result<Vec<Vec<i64>>> {
        init.calls
            .iter()
            .filter(|c| c.name == name)
            .map(|c| {
                c.args
                    .iter()
                    .map(|a| {
                        a.as_int()
                            .ok_or_else(|| fail(format!("{name} argument is not constant")))
                    })
                    .collect()
            })
            .collect()
    };
    let store = |name: &str| {
        init.stores
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, t)| t.as_int())
    };
    let new_game = NewGameInit {
        players_built: ints_of("FF9Play_New")?.into_iter().flatten().collect(),
        players_added: ints_of("FF9Play_Add")?.into_iter().flatten().collect(),
        party: ints_of("FF9Play_SetParty")?
            .into_iter()
            .map(|a| match a.as_slice() {
                [party_slot, player_slot] => Ok(PartyAssignment {
                    party_slot: *party_slot,
                    player_slot: *player_slot,
                }),
                _ => Err(fail("FF9Play_SetParty argument count")),
            })
            .collect::<Result<_>>()?,
        gil: store("gil"),
        summon_flag: store("summon_flag"),
    };

    // Initial items.
    let items_run = run(assembly, "ff9item", "FF9Item_Init", &RunOptions::default())?;
    let initial_items = array_local(&items_run, "FF9Item_Init", "FF9ITEM")?
        .iter()
        .map(|tree| {
            let Tree::Object { args, .. } = tree else {
                return Err(fail("FF9ITEM element is not a constructor call"));
            };
            let get = |name: &str| {
                args.iter()
                    .find(|(n, _)| n == name)
                    .and_then(|(_, t)| t.as_int())
                    .ok_or_else(|| fail(format!("FF9ITEM has no constant `{name}`")))
            };
            Ok(InitialItemRow {
                id: u16::try_from(get("id")?).map_err(|_| fail("item id"))?,
                count: byte("item count", get("count")?)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // Command slots.
    let mut command_slots = Vec::new();
    for index in 0..6u8 {
        let result = run(
            assembly,
            "BattleHUD",
            "GetCommandFromCommandIndex",
            &RunOptions {
                args: vec![None, Some(i64::from(index))],
                ..RunOptions::default()
            },
        )?;
        let command = result
            .returned
            .as_ref()
            .and_then(Tree::as_int)
            .and_then(|v| u8::try_from(v).ok());
        command_slots.push(CommandSlotRow {
            index,
            command,
            stopped: result
                .stop
                .map(|s| format!("at {:#x}: {}", s.offset, s.reason)),
        });
    }

    let (text_root, text_bindings) = text_bindings(assembly)?;
    Ok(PlayerTables {
        command_menus,
        command_slots,
        player_slots,
        menu_type_char_ids,
        categories,
        equipment_set_by_slot,
        default_equipment,
        new_game,
        initial_items,
        ff9play_constants: assembly
            .literal_constants("ff9play")
            .map_err(|e| fail(e.to_string()))?,
        player_constants: assembly
            .literal_constants("PLAYER")
            .map_err(|e| fail(e.to_string()))?,
        default_names,
        text_root,
        text_bindings,
    })
}

#[cfg(test)]
mod tests;
