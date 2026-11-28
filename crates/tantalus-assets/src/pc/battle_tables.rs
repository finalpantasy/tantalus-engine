//! Typed vanilla battle, item, ability, shop and level tables.
//!
//! The installed `Assembly-CSharp.dll` stores these as C# static array
//! initializers (no embedded resource and no `resources.assets` TextAsset
//! holds them). [`super::managed_il`] evaluates the static constructors; this
//! module reads the resulting trees into typed rows. Field names follow the
//! installed constructors' parameter names, which are the same names Memoria's
//! `Global/` data classes use. No field is renamed or reinterpreted: where the
//! meaning is not stated by the source, the raw value keeps its source name.
//!
//! Source classes (installed names, Memoria pin c716e1fa for the layouts):
//!
//! | Table | Static field | Rows |
//! | --- | --- | --- |
//! | actions (`AA_DATA`) | `FF9BattleDB.aa_data` | 192 |
//! | status data (`STAT_DATA`) | `FF9BattleDB.status_data` | 32 |
//! | status sets | `FF9BattleDB.add_status` | 128 |
//! | items (`FF9ITEM_DATA`) | `ff9item._FF9Item_Data` | 256 |
//! | item use effects (`ITEM_DATA`) | `ff9item._FF9Item_Info` | 32 |
//! | weapons (`WEAPON`) | `ff9weap._FF9Weapon_Data` | 88 |
//! | armor (`DEF_PARAMS`) | `ff9armor._FF9Armor_Data` | 136 |
//! | equipment bonuses (`EQUIP_PRIVILEGE`) | `ff9equip._FF9EquipBonus_Data` | 176 |
//! | learnable abilities (`PA_DATA`) | `ff9abil._FF9Abil_PaData` | 16 sets |
//! | supporting abilities (`SA_DATA`) | `ff9abil._FF9Abil_SaData` | 64 |
//! | shop stock | `ff9buy._FF9Buy_Data` | 32 |
//! | synthesis (`FF9MIX_DATA`) | `ff9mix._FF9MIX_DATA` | 64 |
//! | commands (`FF9COMMAND`) | `rdata._FF9FAbil_ComData` | 48 |
//! | command ability ids | `rdata._FF9BMenu_ComAbil` | 192 |
//! | level base stats | `ff9level._FF9Level_Base` | 12 |
//! | level HP/MP | `ff9level._FF9Level_HpMp` | 99 |
//! | level experience | `ff9level._FF9Level_Exp` | 99 |

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::managed_il::{Assembly, Tree};

mod player;
mod player_known;
pub use player::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    pub table: String,
    pub detail: String,
}

impl TableError {
    fn new(table: &str, detail: impl Into<String>) -> Self {
        Self {
            table: table.to_owned(),
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for TableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "battle table {}: {}", self.table, self.detail)
    }
}

impl std::error::Error for TableError {}

type Result<T> = std::result::Result<T, TableError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmdInfo {
    pub cursor: u8,
    pub def_cur: u8,
    pub sub_win: u8,
    pub vfx_no: i16,
    pub sfx_no: i16,
    pub dead: u8,
    pub def_cam: u8,
    pub def_dead: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtlRef {
    pub prog_no: u8,
    pub power: u8,
    pub attr: u8,
    pub rate: u8,
}

/// `AA_DATA`. `name_arg` is the constructor's `Name` argument; the installed
/// constructor does not store it in any field, so it is kept as a raw
/// argument only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRow {
    pub id: u32,
    pub info: CmdInfo,
    pub btl_ref: BtlRef,
    pub category: u8,
    pub add_no: u8,
    pub mp: u8,
    pub kind: u8,
    pub vfx2: u16,
    pub name_arg: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusDataRow {
    pub id: u32,
    pub priority: u8,
    pub opr_cnt: u8,
    pub conti_cnt: u16,
    pub clear: u32,
    pub invalid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRow {
    pub id: u32,
    pub name: u16,
    pub help: u16,
    pub price: u16,
    pub equip: u16,
    pub shape: u8,
    pub color: u8,
    pub eq_lv: u8,
    pub bonus: u8,
    pub ability: [u8; 3],
    pub kind: u8,
    pub sort: u8,
    pub pad: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemUseRow {
    pub id: u32,
    pub info: CmdInfo,
    pub btl_ref: BtlRef,
    pub status: u32,
}

/// `WEAPON`. `model_no` is what the installed constructor computes: the first
/// `FF9BattleDB.GEO` key whose value equals `model_name`, or 0 when none does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponRow {
    pub id: u32,
    pub category: u8,
    pub add_no: u8,
    /// `None` when the installed row passes a null name (the last three rows).
    pub model_name: Option<String>,
    pub model_no: u16,
    pub model_found: bool,
    pub btl_ref: BtlRef,
    pub offset: [i16; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmorRow {
    pub id: u32,
    pub p_def: u8,
    pub p_ev: u8,
    pub m_def: u8,
    pub m_ev: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipBonusRow {
    pub id: u32,
    pub dex: u8,
    pub str_: u8,
    pub mgc: u8,
    pub wpr: u8,
    pub invalid: u8,
    pub absorb: u8,
    pub half: u8,
    pub weak: u8,
    pub p_up_attr: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityPaRow {
    pub id: u8,
    pub max_ap: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportAbilityRow {
    pub id: u32,
    pub category: u8,
    pub capa_val: u8,
    pub name: u16,
    pub help: u16,
    pub help_size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SynthesisRow {
    pub id: u32,
    pub price: u16,
    pub src: [u8; 2],
    pub dst: u8,
    pub shop: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRow {
    pub id: u32,
    pub name: u16,
    pub help: u16,
    pub help_size: u16,
    pub kind: u8,
    pub count: u8,
    pub ability: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelBaseRow {
    pub id: u32,
    pub dex: u8,
    pub str_: u8,
    pub mgc: u8,
    pub wpr: u8,
    pub cap: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelHpMpRow {
    /// Zero-based index; level 1 is index 0.
    pub index: u32,
    pub hp: u16,
    pub mp: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleTables {
    pub actions: Vec<ActionRow>,
    pub status_data: Vec<StatusDataRow>,
    pub status_sets: Vec<u32>,
    pub items: Vec<ItemRow>,
    pub item_use: Vec<ItemUseRow>,
    pub weapons: Vec<WeaponRow>,
    pub armor: Vec<ArmorRow>,
    pub equip_bonus: Vec<EquipBonusRow>,
    pub learnable_abilities: Vec<Vec<AbilityPaRow>>,
    pub support_abilities: Vec<SupportAbilityRow>,
    pub shops: Vec<Vec<u8>>,
    pub synthesis: Vec<SynthesisRow>,
    pub commands: Vec<CommandRow>,
    pub command_abilities: Vec<i32>,
    pub level_base: Vec<LevelBaseRow>,
    pub level_hp_mp: Vec<LevelHpMpRow>,
    pub level_exp: Vec<u64>,
    /// Party, equipment, command menus, names and text bindings (method-body
    /// literals, see `player`).
    #[serde(default)]
    pub player: PlayerTables,
}

// ---------------------------------------------------------------------------
// Tree accessors
// ---------------------------------------------------------------------------

struct Args<'a> {
    table: &'a str,
    expected: &'a str,
    args: &'a [(String, Tree)],
}

impl<'a> Args<'a> {
    fn of(table: &'a str, expected: &'a str, tree: &'a Tree) -> Result<Self> {
        match tree {
            Tree::Object { type_name, args } if type_name.rsplit('.').next() == Some(expected) => {
                Ok(Self {
                    table,
                    expected,
                    args,
                })
            }
            other => Err(TableError::new(
                table,
                format!("expected a `{expected}` constructor call, found {other:?}"),
            )),
        }
    }

    fn tree(&self, name: &str) -> Result<&'a Tree> {
        self.args
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, tree)| tree)
            .ok_or_else(|| {
                TableError::new(
                    self.table,
                    format!("`{}` has no constructor parameter `{name}`", self.expected),
                )
            })
    }

    fn int(&self, name: &str) -> Result<i64> {
        self.tree(name)?.as_int().ok_or_else(|| {
            TableError::new(
                self.table,
                format!("parameter `{name}` is not a constant integer"),
            )
        })
    }

    fn u8(&self, name: &str) -> Result<u8> {
        u8::try_from(self.int(name)?)
            .map_err(|_| TableError::new(self.table, format!("`{name}` does not fit u8")))
    }

    fn u16(&self, name: &str) -> Result<u16> {
        u16::try_from(self.int(name)?)
            .map_err(|_| TableError::new(self.table, format!("`{name}` does not fit u16")))
    }

    fn i16(&self, name: &str) -> Result<i16> {
        i16::try_from(self.int(name)?)
            .map_err(|_| TableError::new(self.table, format!("`{name}` does not fit i16")))
    }

    fn u32(&self, name: &str) -> Result<u32> {
        u32::try_from(self.int(name)?)
            .map_err(|_| TableError::new(self.table, format!("`{name}` does not fit u32")))
    }

    fn object(&self, name: &str, expected: &'a str) -> Result<Args<'a>> {
        Args::of(self.table, expected, self.tree(name)?)
    }

    fn bytes<const N: usize>(&self, name: &str) -> Result<[u8; N]> {
        let values = int_array(self.table, self.tree(name)?)?;
        let narrowed: Vec<u8> = values
            .iter()
            .map(|v| u8::try_from(*v))
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| TableError::new(self.table, format!("`{name}` element not a byte")))?;
        narrowed.try_into().map_err(|v: Vec<u8>| {
            TableError::new(
                self.table,
                format!("`{name}` has {} elements, expected {N}", v.len()),
            )
        })
    }
}

fn int_array(table: &str, tree: &Tree) -> Result<Vec<i64>> {
    match tree {
        Tree::Array { items, .. } => items
            .iter()
            .map(|item| {
                item.as_int()
                    .ok_or_else(|| TableError::new(table, "array element is not a constant"))
            })
            .collect(),
        other => Err(TableError::new(
            table,
            format!("expected an array, found {other:?}"),
        )),
    }
}

fn items<'a>(table: &str, tree: &'a Tree) -> Result<&'a [Tree]> {
    match tree {
        Tree::Array { items, .. } => Ok(items),
        other => Err(TableError::new(
            table,
            format!("expected an array, found {other:?}"),
        )),
    }
}

fn field<'a>(statics: &'a BTreeMap<String, Tree>, class: &str, name: &str) -> Result<&'a Tree> {
    let tree = statics
        .get(name)
        .ok_or_else(|| TableError::new(class, format!("static field `{name}` was not set")))?;
    if tree.contains_unsupported() {
        return Err(TableError::new(
            class,
            format!("static field `{name}` contains values the evaluator cannot read"),
        ));
    }
    Ok(tree)
}

fn cmd_info(parent: &Args<'_>, name: &str) -> Result<CmdInfo> {
    let a = parent.object(name, "CMD_INFO")?;
    Ok(CmdInfo {
        cursor: a.u8("cursor")?,
        def_cur: a.u8("def_cur")?,
        sub_win: a.u8("sub_win")?,
        vfx_no: a.i16("vfx_no")?,
        sfx_no: a.i16("sfx_no")?,
        dead: a.u8("dead")?,
        def_cam: a.u8("def_cam")?,
        def_dead: a.u8("def_dead")?,
    })
}

fn btl_ref(parent: &Args<'_>, name: &str) -> Result<BtlRef> {
    let a = parent.object(name, "BTL_REF")?;
    Ok(BtlRef {
        prog_no: a.u8("prog_no")?,
        power: a.u8("power")?,
        attr: a.u8("attr")?,
        rate: a.u8("rate")?,
    })
}

// ---------------------------------------------------------------------------
// Table parsers (take evaluated static fields)
// ---------------------------------------------------------------------------

pub fn parse_actions(tree: &Tree) -> Result<Vec<ActionRow>> {
    let table = "FF9BattleDB.aa_data";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "AA_DATA", row)?;
            Ok(ActionRow {
                id: id as u32,
                info: cmd_info(&a, "Info")?,
                btl_ref: btl_ref(&a, "Ref")?,
                category: a.u8("Category")?,
                add_no: a.u8("AddNo")?,
                mp: a.u8("MP")?,
                kind: a.u8("Type")?,
                vfx2: a.u16("Vfx2")?,
                name_arg: a.u16("Name")?,
            })
        })
        .collect()
}

pub fn parse_status_data(tree: &Tree) -> Result<Vec<StatusDataRow>> {
    let table = "FF9BattleDB.status_data";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "STAT_DATA", row)?;
            Ok(StatusDataRow {
                id: id as u32,
                priority: a.u8("priority")?,
                opr_cnt: a.u8("opr_cnt")?,
                conti_cnt: a.u16("conti_cnt")?,
                clear: a.u32("clear")?,
                invalid: a.u32("invalid")?,
            })
        })
        .collect()
}

pub fn parse_u32_array(table: &str, tree: &Tree) -> Result<Vec<u32>> {
    int_array(table, tree)?
        .into_iter()
        .map(|v| u32::try_from(v).map_err(|_| TableError::new(table, "value does not fit u32")))
        .collect()
}

pub fn parse_items(tree: &Tree) -> Result<Vec<ItemRow>> {
    let table = "ff9item._FF9Item_Data";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "FF9ITEM_DATA", row)?;
            Ok(ItemRow {
                id: id as u32,
                name: a.u16("name")?,
                help: a.u16("help")?,
                price: a.u16("price")?,
                equip: a.u16("equip")?,
                shape: a.u8("shape")?,
                color: a.u8("color")?,
                eq_lv: a.u8("eq_lv")?,
                bonus: a.u8("bonus")?,
                ability: a.bytes::<3>("ability")?,
                kind: a.u8("type")?,
                sort: a.u8("sort")?,
                pad: a.u8("pad")?,
            })
        })
        .collect()
}

pub fn parse_item_use(tree: &Tree) -> Result<Vec<ItemUseRow>> {
    let table = "ff9item._FF9Item_Info";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "ITEM_DATA", row)?;
            Ok(ItemUseRow {
                id: id as u32,
                info: cmd_info(&a, "info")?,
                btl_ref: btl_ref(&a, "Ref")?,
                status: a.u32("status")?,
            })
        })
        .collect()
}

/// `geo` is the evaluated `FF9BattleDB.GEO` map (`int` to model name).
pub fn parse_weapons(tree: &Tree, geo: &Tree) -> Result<Vec<WeaponRow>> {
    let table = "ff9weap._FF9Weapon_Data";
    let Tree::Map(entries) = geo else {
        return Err(TableError::new(table, "GEO is not a dictionary"));
    };
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "WEAPON", row)?;
            let model_name = match a.tree("model_name")? {
                Tree::Str(name) => Some(name.clone()),
                Tree::Null => None,
                other => {
                    return Err(TableError::new(
                        table,
                        format!("model_name is neither a string nor null: {other:?}"),
                    ));
                }
            };
            let found = model_name.as_ref().and_then(|name| {
                entries.iter().find_map(|(key, value)| match (key, value) {
                    (Tree::Int(k), Tree::Str(v)) if v == name => Some(*k),
                    _ => None,
                })
            });
            let offset = int_array(table, a.tree("offset")?)?;
            let [first, second] = offset[..] else {
                return Err(TableError::new(table, "offset is not two values"));
            };
            Ok(WeaponRow {
                id: id as u32,
                category: a.u8("category")?,
                add_no: a.u8("add_no")?,
                model_name,
                model_no: found.map_or(0, |k| k as u16),
                model_found: found.is_some(),
                btl_ref: btl_ref(&a, "Ref")?,
                offset: [
                    i16::try_from(first)
                        .map_err(|_| TableError::new(table, "offset does not fit i16"))?,
                    i16::try_from(second)
                        .map_err(|_| TableError::new(table, "offset does not fit i16"))?,
                ],
            })
        })
        .collect()
}

pub fn parse_armor(tree: &Tree) -> Result<Vec<ArmorRow>> {
    let table = "ff9armor._FF9Armor_Data";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "DEF_PARAMS", row)?;
            Ok(ArmorRow {
                id: id as u32,
                p_def: a.u8("p_def")?,
                p_ev: a.u8("p_ev")?,
                m_def: a.u8("m_def")?,
                m_ev: a.u8("m_ev")?,
            })
        })
        .collect()
}

pub fn parse_equip_bonus(tree: &Tree) -> Result<Vec<EquipBonusRow>> {
    let table = "ff9equip._FF9EquipBonus_Data";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "EQUIP_PRIVILEGE", row)?;
            let attr = a.object("def_attr", "DEF_ATTR")?;
            Ok(EquipBonusRow {
                id: id as u32,
                dex: a.u8("dex")?,
                str_: a.u8("str")?,
                mgc: a.u8("mgc")?,
                wpr: a.u8("wpr")?,
                invalid: attr.u8("invalid")?,
                absorb: attr.u8("absorb")?,
                half: attr.u8("half")?,
                weak: attr.u8("weak")?,
                p_up_attr: a.u8("p_up_attr")?,
            })
        })
        .collect()
}

pub fn parse_learnable(tree: &Tree) -> Result<Vec<Vec<AbilityPaRow>>> {
    let table = "ff9abil._FF9Abil_PaData";
    items(table, tree)?
        .iter()
        .map(|set| {
            items(table, set)?
                .iter()
                .map(|row| {
                    let a = Args::of(table, "PA_DATA", row)?;
                    Ok(AbilityPaRow {
                        id: a.u8("id")?,
                        max_ap: a.u8("max_ap")?,
                    })
                })
                .collect()
        })
        .collect()
}

pub fn parse_support_abilities(tree: &Tree) -> Result<Vec<SupportAbilityRow>> {
    let table = "ff9abil._FF9Abil_SaData";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "SA_DATA", row)?;
            Ok(SupportAbilityRow {
                id: id as u32,
                category: a.u8("category")?,
                capa_val: a.u8("capa_val")?,
                name: a.u16("name")?,
                help: a.u16("help")?,
                help_size: a.u16("help_size")?,
            })
        })
        .collect()
}

pub fn parse_shops(tree: &Tree) -> Result<Vec<Vec<u8>>> {
    let table = "ff9buy._FF9Buy_Data";
    items(table, tree)?
        .iter()
        .map(|shop| {
            int_array(table, shop)?
                .into_iter()
                .map(|v| u8::try_from(v).map_err(|_| TableError::new(table, "item id not a byte")))
                .collect()
        })
        .collect()
}

pub fn parse_synthesis(tree: &Tree) -> Result<Vec<SynthesisRow>> {
    let table = "ff9mix._FF9MIX_DATA";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "FF9MIX_DATA", row)?;
            Ok(SynthesisRow {
                id: id as u32,
                price: a.u16("price")?,
                src: a.bytes::<2>("src")?,
                dst: a.u8("dst")?,
                shop: a.u8("shop")?,
            })
        })
        .collect()
}

pub fn parse_commands(tree: &Tree) -> Result<Vec<CommandRow>> {
    let table = "rdata._FF9FAbil_ComData";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "FF9COMMAND", row)?;
            Ok(CommandRow {
                id: id as u32,
                name: a.u16("name")?,
                help: a.u16("help")?,
                help_size: a.u16("help_size")?,
                kind: a.u8("type")?,
                count: a.u8("count")?,
                ability: a.int("ability")? as u64,
            })
        })
        .collect()
}

pub fn parse_level_base(tree: &Tree) -> Result<Vec<LevelBaseRow>> {
    let table = "ff9level._FF9Level_Base";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(id, row)| {
            let a = Args::of(table, "FF9LEVEL_BASE", row)?;
            Ok(LevelBaseRow {
                id: id as u32,
                dex: a.u8("dex")?,
                str_: a.u8("str")?,
                mgc: a.u8("mgc")?,
                wpr: a.u8("wpr")?,
                cap: a.u8("cap")?,
            })
        })
        .collect()
}

pub fn parse_level_hp_mp(tree: &Tree) -> Result<Vec<LevelHpMpRow>> {
    let table = "ff9level._FF9Level_HpMp";
    items(table, tree)?
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let a = Args::of(table, "FF9LEVEL_HPMP", row)?;
            Ok(LevelHpMpRow {
                index: index as u32,
                hp: a.u16("hp")?,
                mp: a.u16("mp")?,
            })
        })
        .collect()
}

pub fn parse_level_exp(tree: &Tree) -> Result<Vec<u64>> {
    int_array("ff9level._FF9Level_Exp", tree).map(|v| v.into_iter().map(|x| x as u64).collect())
}

/// True when the assembly defines types in the `Memoria` namespace (the Memoria patcher
/// compiles its own code into `Assembly-CSharp.dll`).
pub fn is_memoria_patched(assembly: &Assembly<'_>) -> bool {
    assembly
        .type_names()
        .map(|names| names.iter().any(|n| n.starts_with("Memoria.")))
        .unwrap_or(false)
}

/// Evaluate every source class in the assembly and parse all tables.
///
/// A Memoria-patched `Assembly-CSharp.dll` does not carry these tables inline: its
/// `FF9BattleDB` static constructor loads CSV files at runtime (Memoria
/// `FF9BattleDB.cs`, `LoadActions` and friends), so the constant evaluator stops at a
/// branch. That case is reported as such instead of as a bare IL error; read the
/// vanilla assembly (Memoria's installer keeps it as `Assembly-CSharp.bak`) instead.
pub fn extract_battle_tables(assembly: &Assembly<'_>) -> Result<BattleTables> {
    let load = |class: &str| {
        assembly.static_initializer(class).map_err(|error| {
            let detail = error.to_string();
            if is_memoria_patched(assembly) {
                TableError::new(
                    class,
                    format!(
                        "{detail}; this assembly is Memoria-patched (tables are loaded from CSV at runtime, not stored in IL), use the vanilla Assembly-CSharp (Assembly-CSharp.bak)"
                    ),
                )
            } else {
                TableError::new(class, detail)
            }
        })
    };
    let battle_db = load("FF9BattleDB")?;
    let item = load("ff9item")?;
    let weapon = load("ff9weap")?;
    let armor = load("ff9armor")?;
    let equip = load("ff9equip")?;
    let abil = load("ff9abil")?;
    let buy = load("ff9buy")?;
    let mix = load("ff9mix")?;
    let rdata = load("rdata")?;
    let level = load("ff9level")?;
    let command_abilities = int_array(
        "rdata._FF9BMenu_ComAbil",
        field(&rdata, "rdata", "_FF9BMenu_ComAbil")?,
    )?
    .into_iter()
    .map(|v| {
        i32::try_from(v)
            .map_err(|_| TableError::new("rdata._FF9BMenu_ComAbil", "value does not fit i32"))
    })
    .collect::<Result<Vec<_>>>()?;
    Ok(BattleTables {
        actions: parse_actions(field(&battle_db, "FF9BattleDB", "aa_data")?)?,
        status_data: parse_status_data(field(&battle_db, "FF9BattleDB", "status_data")?)?,
        status_sets: parse_u32_array(
            "FF9BattleDB.add_status",
            field(&battle_db, "FF9BattleDB", "add_status")?,
        )?,
        items: parse_items(field(&item, "ff9item", "_FF9Item_Data")?)?,
        item_use: parse_item_use(field(&item, "ff9item", "_FF9Item_Info")?)?,
        weapons: parse_weapons(
            field(&weapon, "ff9weap", "_FF9Weapon_Data")?,
            field(&battle_db, "FF9BattleDB", "GEO")?,
        )?,
        armor: parse_armor(field(&armor, "ff9armor", "_FF9Armor_Data")?)?,
        equip_bonus: parse_equip_bonus(field(&equip, "ff9equip", "_FF9EquipBonus_Data")?)?,
        learnable_abilities: parse_learnable(field(&abil, "ff9abil", "_FF9Abil_PaData")?)?,
        support_abilities: parse_support_abilities(field(&abil, "ff9abil", "_FF9Abil_SaData")?)?,
        shops: parse_shops(field(&buy, "ff9buy", "_FF9Buy_Data")?)?,
        synthesis: parse_synthesis(field(&mix, "ff9mix", "_FF9MIX_DATA")?)?,
        commands: parse_commands(field(&rdata, "rdata", "_FF9FAbil_ComData")?)?,
        command_abilities,
        level_base: parse_level_base(field(&level, "ff9level", "_FF9Level_Base")?)?,
        level_hp_mp: parse_level_hp_mp(field(&level, "ff9level", "_FF9Level_HpMp")?)?,
        level_exp: parse_level_exp(field(&level, "ff9level", "_FF9Level_Exp")?)?,
        player: extract_player_tables(assembly)?,
    })
}

/// One known-row comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnownRowCheck {
    pub table: &'static str,
    pub row: usize,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
    pub source: &'static str,
}

/// Rows whose values are written in Memoria's shipped CSV data
/// (`Memoria.Patcher/StreamingAssets/Data/...`, pin c716e1fa), compared with
/// what was read from the installed assembly.
pub fn verify_known_rows(tables: &BattleTables) -> Vec<KnownRowCheck> {
    let mut checks = Vec::new();
    known_rows::run(tables, &mut checks);
    player_known::run(tables, &mut checks);
    checks
}

mod known_rows;

/// The audit report: counts, sample rows and what was not read.
pub fn summarize(
    tables: &BattleTables,
    checks: &[KnownRowCheck],
    assembly_sha256: &str,
) -> serde_json::Value {
    fn entry<T: Serialize>(
        name: &str,
        source: &str,
        rows: &[T],
        indexes: &[usize],
    ) -> serde_json::Value {
        let sample: Vec<_> = indexes
            .iter()
            .filter_map(|i| rows.get(*i).map(|row| (*i, row)))
            .map(|(i, row)| serde_json::json!({"index": i, "row": row}))
            .collect();
        serde_json::json!({
            "name": name,
            "source": source,
            "count": rows.len(),
            "sample_rows": sample,
        })
    }
    let tables_json = vec![
        entry(
            "actions",
            "FF9BattleDB.aa_data (AA_DATA)",
            &tables.actions,
            &[0, 1, 2, 191],
        ),
        entry(
            "status_data",
            "FF9BattleDB.status_data (STAT_DATA)",
            &tables.status_data,
            &[0, 1, 31],
        ),
        entry(
            "status_sets",
            "FF9BattleDB.add_status (uint[])",
            &tables.status_sets,
            &[0, 1, 5, 127],
        ),
        entry(
            "items",
            "ff9item._FF9Item_Data (FF9ITEM_DATA)",
            &tables.items,
            &[0, 1, 255],
        ),
        entry(
            "item_use",
            "ff9item._FF9Item_Info (ITEM_DATA)",
            &tables.item_use,
            &[0, 12, 31],
        ),
        entry(
            "weapons",
            "ff9weap._FF9Weapon_Data (WEAPON)",
            &tables.weapons,
            &[0, 1, 87],
        ),
        entry(
            "armor",
            "ff9armor._FF9Armor_Data (DEF_PARAMS)",
            &tables.armor,
            &[0, 1, 135],
        ),
        entry(
            "equip_bonus",
            "ff9equip._FF9EquipBonus_Data (EQUIP_PRIVILEGE)",
            &tables.equip_bonus,
            &[0, 1, 175],
        ),
        entry(
            "support_abilities",
            "ff9abil._FF9Abil_SaData (SA_DATA)",
            &tables.support_abilities,
            &[0, 1, 63],
        ),
        entry(
            "shops",
            "ff9buy._FF9Buy_Data (byte[][])",
            &tables.shops,
            &[0, 31],
        ),
        entry(
            "synthesis",
            "ff9mix._FF9MIX_DATA (FF9MIX_DATA)",
            &tables.synthesis,
            &[0, 1, 63],
        ),
        entry(
            "commands",
            "rdata._FF9FAbil_ComData (FF9COMMAND)",
            &tables.commands,
            &[0, 1, 47],
        ),
        entry(
            "command_abilities",
            "rdata._FF9BMenu_ComAbil (int[])",
            &tables.command_abilities,
            &[0, 1, 191],
        ),
        entry(
            "level_base",
            "ff9level._FF9Level_Base (FF9LEVEL_BASE)",
            &tables.level_base,
            &[0, 1, 11],
        ),
        entry(
            "level_hp_mp",
            "ff9level._FF9Level_HpMp (FF9LEVEL_HPMP)",
            &tables.level_hp_mp,
            &[0, 1, 98],
        ),
        entry(
            "level_exp",
            "ff9level._FF9Level_Exp (ulong[])",
            &tables.level_exp,
            &[0, 1, 98],
        ),
        entry(
            "command_menus",
            "rdata._FF9BMenu_MenuNormal [20,2] and _FF9BMenu_MenuTrance [20,3] (rank-2 arrays)",
            &tables.player.command_menus,
            &[0, 1, 8, 19],
        ),
        entry(
            "command_slots",
            "BattleHUD.GetCommandFromCommandIndex (integer control flow)",
            &tables.player.command_slots,
            &[0, 1, 2, 3, 4, 5],
        ),
        entry(
            "player_slots",
            "ff9play.FF9Play_New PLAYER_INFO[9] literal plus GetCharID, GetCategory, GetCharID3",
            &tables.player.player_slots,
            &[0, 4, 5, 8],
        ),
        entry(
            "default_equipment",
            "ff9play.FF9Play_SetDefEquips byte[16][5] literal",
            &tables.player.default_equipment,
            &[0, 1, 11, 15],
        ),
        entry(
            "initial_items",
            "ff9item.FF9Item_Init FF9ITEM[] literal",
            &tables.player.initial_items,
            &[0, 1, 6, 7],
        ),
        entry(
            "default_names",
            "FF9TextTool.CharacterDefaultName string[20] per language",
            &tables.player.default_names,
            &[0, 2],
        ),
        entry(
            "text_bindings",
            "FF9TextTool accessors and Initialize*Text iterators",
            &tables.player.text_bindings,
            &[0, 1, 6, 10],
        ),
        serde_json::json!({
            "name": "new_game",
            "source": "ff9play.FF9Play_Init run with integer loop counters",
            "count": 1,
            "value": &tables.player.new_game,
            "equipment_set_by_slot": &tables.player.equipment_set_by_slot,
            "menu_type_char_ids": &tables.player.menu_type_char_ids,
            "categories": &tables.player.categories,
            "ff9play_constants": &tables.player.ff9play_constants,
            "player_constants": &tables.player.player_constants,
            "text_root": &tables.player.text_root,
        }),
        serde_json::json!({
            "name": "learnable_abilities",
            "source": "ff9abil._FF9Abil_PaData (PA_DATA[][])",
            "count": tables.learnable_abilities.len(),
            "set_sizes": tables.learnable_abilities.iter().map(Vec::len).collect::<Vec<_>>(),
            "sample_rows": [{"index": 0, "row": tables.learnable_abilities.first().map(|s| s.iter().take(3).collect::<Vec<_>>())}],
        }),
    ];
    serde_json::json!({
        "schema": "tantalus.battle-tables.v1",
        "assembly": "x64/FF9_Data/Managed/Assembly-CSharp.dll",
        "assembly_sha256": assembly_sha256,
        "method": "static constructors evaluated as constant-only IL, plus method-body literals and integer control flow (managed_il::run_method); no game code run",
        "tables": tables_json,
        "known_row_checks": {
            "total": checks.len(),
            "passed": checks.iter().filter(|c| c.passed).count(),
            "failed": checks.iter().filter(|c| !c.passed).collect::<Vec<_>>(),
        },
        "unresolved": UNRESOLVED,
    })
}

/// Items this reader cannot or does not extract, with the reason.
pub const UNRESOLVED: &[&str] = &[
    "ff9play.FF9Play_GetSerialID: the model/serial number a player ends up with depends on the equipped weapon shape and event state (gEventGlobal); not evaluated. `serial_no` in player_slots is the literal's constructor argument, which FF9Play_New overwrites.",
    "ff9play.FF9Play_Build and FF9Play_Update (level 1 stats, HP/MP, equipment sums) are not evaluated; the typed level tables feed them.",
    "BattleHUD.GetCommandFromCommandIndex indexes 2 and 3 branch on Status.checkCurStat (trance); the source shows MenuNormal/MenuTrance columns 0 and 1, but the evaluator stops at the branch, so those two slots are recorded as stopped.",
    "MenuTrance column 2 (`trance_third`): read raw; BattleHUD.SetAbilityTrance compares it with 1 and subtracts 1, its meaning is not stated by the source.",
    "Enemy stat and attack tables are not in the assembly; they are per-scene battle data (see battle_scene.rs).",
    "Display names: the `name` and `help` fields of FF9ITEM_DATA, FF9COMMAND and SA_DATA are read only by their own constructors (no other method touches them), and AA_DATA.Name is never stored by the table constructor; the shown text comes from FF9TextTool arrays indexed by the table row index (see text_bindings). Sampled callers (ItemName, ActionAbilityName) pass ids, not those fields; all callers were not enumerated. Whether each `.mes` entry position equals the row index in the install's text files was not checked here.",
    "Base stats per character: _FF9Level_Base is indexed by the equipment/character id (0..11); Memoria's BaseStats.csv names rows by character but the installed table carries no names.",
    "ITEM_DATA (item_use) holds the battle-use effect for items 224..255 (FF9ITEM_INFO_START); the mapping from FF9ITEM_DATA.bonus/ability fields to Memoria's ItemEffects.csv is not decoded here.",
    "AA_DATA.Name: the constructor takes a name id but the installed constructor body does not store it; kept as `name_arg` only (the values are legacy text offsets, not entry indexes).",
];

#[cfg(test)]
mod tests;
