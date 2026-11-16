//! Typed readers for the PC battle data files.
//!
//! Sources (pinned Memoria C#, `Assembly-CSharp/Global/`):
//! - `BTL_SCENE.cs` `ReadBattleScene`, `SB2/SB2_{HEAD,PATTERN,PUT,MON_PARM,ELEMENT}.cs`,
//!   `AA_DATA.cs` for `dbfile0000.raw16`.
//! - `BBGINFO.cs` `ReadBattleInfo` for `INB_*.inb`.
//! - `btlseq.cs` `ReadBattleSequence` for the `.raw17` header (structure level only).
//!
//! Fields whose meaning Memoria does not define are kept as named raw fields; they are
//! listed in [`UNKNOWN_FIELDS`].

use crate::install::ImportError;
use crate::pc::asset_bundle::{AssetBundleMetadata, AssetInfo};
use crate::pc::serialized::SerializedFile;
use crate::pc::text_asset::TextAsset;
use crate::pc::unity_raw::UnityRawBundle;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Header versions Memoria's reader handles: 7 is vanilla; 8, 9 and 10 are Memoria extensions.
const VERSION_VANILLA: u8 = 7;
const VERSION_SEQPATH: u8 = 9;
const VERSION_ELEMCLASSEX: u8 = 10;

/// Fields Memoria reads but never gives a meaning (kept raw).
pub const UNKNOWN_FIELDS: &[&str] = &[
    "SceneHeader.pad1 (SB2_HEAD.Pad1)",
    "Pattern.pad0 (SB2_PATTERN.Pad0)",
    "Placement.pad (SB2_PUT.Pad)",
    "Placement.pease (SB2_PUT.Pease: no consumer in BTL_SCENE.cs)",
    "MonsterStats.pad (SB2_ELEMENT.pad)",
    "MonsterType.pad0, pad1, pad2 (SB2_MON_PARM.Pad0 to Pad2)",
    "MonsterType.konran, mes_cnt (read, meaning defined elsewhere)",
    "BattleScene.trailing (tail after the last attack; Memoria never reads it)",
    "Attack.trailer_u16 (a ReadUInt16 whose result Memoria discards)",
    "Attack.vfx_word bits 9 and up",
    "Attack.flags_byte bits 3 and 4",
    "BattleInfo fields (BBGINFO.cs names them but gives no semantics)",
    "SequenceHeader.seq_block_offset and cam_offset (read; used past the header)",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneHeader {
    pub version: u8,
    pub pattern_count: u8,
    pub type_count: u8,
    pub attack_count: u8,
    pub flags: u16,
    pub pad1: i16,
    /// Present only when `version > 7`.
    pub battle_map_index: Option<u16>,
}

impl SceneHeader {
    /// Scene flag names from `SB2_HEAD` (`SB2_FLG_*`).
    pub fn flag_names(&self) -> Vec<&'static str> {
        const NAMES: [(u16, &str); 14] = [
            (0x1, "special"),
            (0x2, "back_attack"),
            (0x4, "no_game_over"),
            (0x8, "exp_zero"),
            (0x10, "no_win_pose"),
            (0x20, "no_run_away"),
            (0x40, "no_near_attack"),
            (0x80, "no_magical"),
            (0x100, "reverse_attack"),
            (0x200, "fixed_cam1"),
            (0x400, "fixed_cam2"),
            (0x800, "after_event"),
            (0x1000, "message_event"),
            (0x2000, "field_bgm"),
        ];
        NAMES
            .iter()
            .filter(|(bit, _)| self.flags & bit != 0)
            .map(|(_, name)| *name)
            .collect()
    }
}

/// `SB2_PUT`: one monster placement inside a pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub type_no: u8,
    /// Bit 0 targetable, bit 1 multipart (`FLG_TARGETABLE`, `FLG_MULTIPART`).
    pub flags: u8,
    pub pease: u8,
    pub pad: u8,
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub rot: i16,
}

impl Placement {
    pub const FLG_TARGETABLE: u8 = 1;
    pub const FLG_MULTIPART: u8 = 2;
}

/// `SB2_PATTERN`: always four placements on disk; `monster_count` says how many are used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub rate: u8,
    pub monster_count: u8,
    pub camera: u8,
    pub pad0: u8,
    pub ap: u32,
    pub monsters: [Placement; 4],
}

/// A set stored either as one raw byte (header versions below 10) or as a token list
/// (`ElementSet.ReadBinary` / `EnemyCategorySet.ReadBinary`, version 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByteOrTokens {
    Byte(u8),
    Tokens(Vec<i32>),
}

/// `SB2_ELEMENT`: monster stats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonsterStats {
    pub speed: u8,
    pub strength: u8,
    pub magic: u8,
    pub spirit: u8,
    pub pad: u8,
    pub trans: u8,
    pub cur_capa: u8,
    pub max_capa: u8,
}

/// `SB2_MON_PARM`: one monster type. Versions below 8 use narrow fields; all values are
/// widened here. Status fields are raw bit masks (version 7) or the OR of `1 << index`
/// over the stored list (versions 8 and up).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonsterType {
    pub resist_status: u64,
    pub auto_status: u64,
    pub initial_status: u64,
    pub max_hp: u32,
    pub max_mp: u32,
    pub win_gil: u32,
    pub win_exp: u32,
    pub win_items: [u32; 4],
    pub steal_items: [u32; 4],
    /// Vanilla files carry none; Memoria's defaults (256, 96, 32, 1) are filled in.
    pub win_item_rates: [u16; 4],
    /// Vanilla files carry none; Memoria's defaults (256, 64, 16, 1) are filled in.
    pub steal_item_rates: [u16; 4],
    /// Vanilla files carry none; Memoria's default (32) is filled in.
    pub win_card_rate: u16,
    pub radius: u16,
    pub geo: i16,
    pub mot: [u16; 6],
    pub mesh: [u16; 2],
    /// Bit 0 dead-attack, bit 1 dead-damage (`MON_PRM_FLG_*`).
    pub flags: u16,
    pub ap: u32,
    pub stats: MonsterStats,
    pub guard_element: ByteOrTokens,
    pub absorb_element: ByteOrTokens,
    pub half_element: ByteOrTokens,
    pub weak_element: ByteOrTokens,
    pub level: u8,
    pub category: ByteOrTokens,
    pub hit_rate: i32,
    pub physical_defence: i32,
    pub physical_evade: i32,
    pub magical_defence: i32,
    pub magical_evade: i32,
    pub blue_magic: i32,
    pub bone: [u8; 4],
    pub die_sfx: u16,
    pub konran: u8,
    pub mes_cnt: u8,
    pub icon_bone: [u8; 6],
    pub icon_y: [i8; 6],
    pub icon_z: [i8; 6],
    pub start_sfx: u16,
    pub shadow_x: u16,
    pub shadow_z: u16,
    pub shadow_bone: u8,
    pub win_card: i32,
    pub shadow_ofs_x: i16,
    pub shadow_ofs_z: i16,
    pub shadow_bone2: u8,
    pub pad0: u8,
    pub pad1: u16,
    pub pad2: u16,
}

/// `AA_DATA` plus its `BattleCommandInfo`/`BTL_REF` bit fields: one enemy attack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attack {
    /// Low 4 bits of the first byte (`TargetType`).
    pub target: u8,
    /// Bit 4 of the first byte.
    pub default_ally: bool,
    /// Bits 5 to 7 of the first byte (`TargetDisplay`).
    pub display_stats: u8,
    /// Whole vfx word (u16 in vanilla, u32 from version 8).
    pub vfx_word: u32,
    /// Low 9 bits of `vfx_word` (`BattleCommandInfo.VfxIndex`).
    pub vfx_index: u16,
    /// Third byte; bits 3 and 4 have no meaning in Memoria.
    pub flags_byte: u8,
    /// Bit 0 (version 9 and up only; false before).
    pub morph_force_access: bool,
    /// Bit 1 (version 9 and up only).
    pub morph_disable_access: bool,
    /// Bit 2 (version 9 and up only).
    pub alternate_idle_access: bool,
    /// Bit 5.
    pub for_dead: bool,
    /// Bit 6.
    pub default_camera: bool,
    /// Bit 7.
    pub default_on_dead: bool,
    pub script_id: i32,
    pub power: i32,
    pub elements: ByteOrTokens,
    pub rate: i32,
    pub category: u8,
    pub add_status_no: i32,
    pub mp: i32,
    pub attack_type: u8,
    pub vfx2: u16,
    pub trailer_u16: u16,
    /// Sequence file name (version 9 and up only).
    pub sequence_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleScene {
    pub header: SceneHeader,
    pub patterns: Vec<Pattern>,
    pub monster_types: Vec<MonsterType>,
    pub attacks: Vec<Attack>,
    /// Bytes left after the last attack record. Memoria never reads them. In the real
    /// bundle all 562 scenes have a 4 to 2048 byte tail (multiple of 4) that holds at most
    /// one leading non-zero u32 followed only by zero bytes; its meaning is unknown.
    pub trailing_bytes: usize,
    /// Whether every trailing byte is zero (padding) rather than unexplained data.
    pub trailing_all_zero: bool,
    pub file_len: usize,
    /// The unparsed tail itself (not serialized).
    #[serde(skip)]
    pub trailing: Vec<u8>,
}

impl BattleScene {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut r = Reader::new(bytes, source);
        let version = r.u8()?;
        let pattern_count = r.u8()?;
        let type_count = r.u8()?;
        let attack_count = r.u8()?;
        let flags = r.u16()?;
        let pad1 = r.i16()?;
        if !(VERSION_VANILLA..=VERSION_ELEMCLASSEX).contains(&version) {
            return Err(ImportError::invalid(
                source,
                format!("unsupported battle scene header version {version}"),
            ));
        }
        let battle_map_index = if version > VERSION_VANILLA {
            Some(r.u16()?)
        } else {
            None
        };
        let header = SceneHeader {
            version,
            pattern_count,
            type_count,
            attack_count,
            flags,
            pad1,
            battle_map_index,
        };
        let mut patterns = Vec::with_capacity(pattern_count as usize);
        for _ in 0..pattern_count {
            patterns.push(read_pattern(&mut r)?);
        }
        let mut monster_types = Vec::with_capacity(type_count as usize);
        for _ in 0..type_count {
            monster_types.push(read_monster_type(&mut r, version)?);
        }
        let mut attacks = Vec::with_capacity(attack_count as usize);
        for _ in 0..attack_count {
            attacks.push(read_attack(&mut r, version)?);
        }
        let trailing_bytes = bytes.len() - r.position;
        Ok(Self {
            header,
            patterns,
            monster_types,
            attacks,
            trailing_bytes,
            trailing_all_zero: bytes[r.position..].iter().all(|byte| *byte == 0),
            file_len: bytes.len(),
            trailing: bytes[r.position..].to_vec(),
        })
    }
}

fn read_pattern(r: &mut Reader<'_>) -> Result<Pattern, ImportError> {
    let rate = r.u8()?;
    let monster_count = r.u8()?;
    let camera = r.u8()?;
    let pad0 = r.u8()?;
    let ap = r.u32()?;
    let mut monsters = Vec::with_capacity(4);
    for _ in 0..4 {
        monsters.push(Placement {
            type_no: r.u8()?,
            flags: r.u8()?,
            pease: r.u8()?,
            pad: r.u8()?,
            x: r.i16()?,
            y: r.i16()?,
            z: r.i16()?,
            rot: r.i16()?,
        });
    }
    let monsters: [Placement; 4] = monsters
        .try_into()
        .map_err(|_| ImportError::invalid(r.source, "pattern placement count"))?;
    Ok(Pattern {
        rate,
        monster_count,
        camera,
        pad0,
        ap,
        monsters,
    })
}

fn read_status(r: &mut Reader<'_>, version: u8) -> Result<u64, ImportError> {
    if version > VERSION_VANILLA {
        // `ReadBattleStatus`: count, then that many bit indices.
        let count = r.u32()?;
        let mut mask = 0_u64;
        for _ in 0..count {
            let index = r.i32()?;
            if !(0..64).contains(&index) {
                return Err(ImportError::invalid(
                    r.source,
                    format!("battle status index {index} is outside 0..64"),
                ));
            }
            mask |= 1_u64 << index;
        }
        Ok(mask)
    } else {
        Ok(u64::from(r.u32()?))
    }
}

fn read_token_list(r: &mut Reader<'_>) -> Result<Vec<i32>, ImportError> {
    let count = r.u32()?;
    if count as usize > r.remaining() / 4 {
        return Err(ImportError::invalid(r.source, "token list exceeds file"));
    }
    (0..count).map(|_| r.i32()).collect()
}

fn read_set(r: &mut Reader<'_>, version: u8) -> Result<ByteOrTokens, ImportError> {
    if version >= VERSION_ELEMCLASSEX {
        Ok(ByteOrTokens::Tokens(read_token_list(r)?))
    } else {
        Ok(ByteOrTokens::Byte(r.u8()?))
    }
}

fn read_monster_type(r: &mut Reader<'_>, version: u8) -> Result<MonsterType, ImportError> {
    let extended = version > VERSION_VANILLA;
    let resist_status = read_status(r, version)?;
    let auto_status = read_status(r, version)?;
    let initial_status = read_status(r, version)?;
    let (max_hp, max_mp, win_gil, win_exp);
    let mut win_items = [0_u32; 4];
    let mut steal_items = [0_u32; 4];
    let mut win_item_rates = [256_u16, 96, 32, 1];
    let mut steal_item_rates = [256_u16, 64, 16, 1];
    let mut win_card_rate = 32_u16;
    if extended {
        max_hp = r.u32()?;
        max_mp = r.u32()?;
        win_gil = r.u32()?;
        win_exp = r.u32()?;
        for item in &mut win_items {
            *item = r.u32()?;
        }
        for item in &mut steal_items {
            *item = r.u32()?;
        }
        for rate in &mut win_item_rates {
            *rate = r.u32()? as u16;
        }
        for rate in &mut steal_item_rates {
            *rate = r.u32()? as u16;
        }
        win_card_rate = r.u32()? as u16;
    } else {
        max_hp = u32::from(r.u16()?);
        max_mp = u32::from(r.u16()?);
        win_gil = u32::from(r.u16()?);
        win_exp = u32::from(r.u16()?);
        for item in &mut win_items {
            *item = u32::from(r.u8()?);
        }
        for item in &mut steal_items {
            *item = u32::from(r.u8()?);
        }
    }
    let radius = r.u16()?;
    let geo = r.i16()?;
    let mut mot = [0_u16; 6];
    for value in &mut mot {
        *value = r.u16()?;
    }
    let mut mesh = [0_u16; 2];
    for value in &mut mesh {
        *value = r.u16()?;
    }
    let flags = r.u16()?;
    let (ap, speed, strength, magic, spirit);
    if extended {
        ap = r.u32()?;
        speed = r.u32()? as u8;
        strength = r.u32()? as u8;
        magic = r.u32()? as u8;
        spirit = r.u32()? as u8;
    } else {
        ap = u32::from(r.u16()?);
        speed = r.u8()?;
        strength = r.u8()?;
        magic = r.u8()?;
        spirit = r.u8()?;
    }
    let stats = MonsterStats {
        speed,
        strength,
        magic,
        spirit,
        pad: r.u8()?,
        trans: r.u8()?,
        cur_capa: r.u8()?,
        max_capa: r.u8()?,
    };
    let guard_element = read_set(r, version)?;
    let absorb_element = read_set(r, version)?;
    let half_element = read_set(r, version)?;
    let weak_element = read_set(r, version)?;
    let (level, category, hit_rate, physical_defence, physical_evade);
    let (magical_defence, magical_evade, blue_magic);
    if extended {
        level = r.u32()? as u8;
        category = read_set(r, version)?;
        hit_rate = r.i32()?;
        physical_defence = r.i32()?;
        physical_evade = r.i32()?;
        magical_defence = r.i32()?;
        magical_evade = r.i32()?;
        blue_magic = r.i32()?;
    } else {
        level = r.u8()?;
        category = ByteOrTokens::Byte(r.u8()?);
        hit_rate = i32::from(r.u8()?);
        physical_defence = i32::from(r.u8()?);
        physical_evade = i32::from(r.u8()?);
        magical_defence = i32::from(r.u8()?);
        magical_evade = i32::from(r.u8()?);
        blue_magic = i32::from(r.u8()?);
    }
    let mut bone = [0_u8; 4];
    for value in &mut bone {
        *value = r.u8()?;
    }
    let die_sfx = r.u16()?;
    let konran = r.u8()?;
    let mes_cnt = r.u8()?;
    let mut icon_bone = [0_u8; 6];
    for value in &mut icon_bone {
        *value = r.u8()?;
    }
    let mut icon_y = [0_i8; 6];
    for value in &mut icon_y {
        *value = r.u8()? as i8;
    }
    let mut icon_z = [0_i8; 6];
    for value in &mut icon_z {
        *value = r.u8()? as i8;
    }
    let start_sfx = r.u16()?;
    let shadow_x = r.u16()?;
    let shadow_z = r.u16()?;
    let shadow_bone = r.u8()?;
    let win_card = if extended {
        r.i32()?
    } else {
        i32::from(r.u8()?)
    };
    Ok(MonsterType {
        resist_status,
        auto_status,
        initial_status,
        max_hp,
        max_mp,
        win_gil,
        win_exp,
        win_items,
        steal_items,
        win_item_rates,
        steal_item_rates,
        win_card_rate,
        radius,
        geo,
        mot,
        mesh,
        flags,
        ap,
        stats,
        guard_element,
        absorb_element,
        half_element,
        weak_element,
        level,
        category,
        hit_rate,
        physical_defence,
        physical_evade,
        magical_defence,
        magical_evade,
        blue_magic,
        bone,
        die_sfx,
        konran,
        mes_cnt,
        icon_bone,
        icon_y,
        icon_z,
        start_sfx,
        shadow_x,
        shadow_z,
        shadow_bone,
        win_card,
        shadow_ofs_x: r.i16()?,
        shadow_ofs_z: r.i16()?,
        shadow_bone2: r.u8()?,
        pad0: r.u8()?,
        pad1: r.u16()?,
        pad2: r.u16()?,
    })
}

fn read_attack(r: &mut Reader<'_>, version: u8) -> Result<Attack, ImportError> {
    let extended = version > VERSION_VANILLA;
    let first = r.u8()?;
    let vfx_word = if extended {
        r.u32()?
    } else {
        u32::from(r.u16()?)
    };
    let flags_byte = r.u8()?;
    let seqpath = version >= VERSION_SEQPATH;
    let (script_id, power, elements, rate, category);
    let (add_status_no, mp, attack_type, vfx2);
    if extended {
        script_id = r.i32()?;
        power = r.i32()?;
        elements = read_set(r, version)?;
        rate = r.i32()?;
        category = r.u8()?;
        add_status_no = r.i32()?;
        mp = r.i32()?;
        attack_type = r.u8()?;
        vfx2 = r.u32()? as u16;
    } else {
        script_id = i32::from(r.u8()?);
        power = i32::from(r.u8()?);
        elements = ByteOrTokens::Byte(r.u8()?);
        rate = i32::from(r.u8()?);
        category = r.u8()?;
        add_status_no = i32::from(r.u8()?);
        mp = i32::from(r.u8()?);
        attack_type = r.u8()?;
        vfx2 = r.u16()?;
    }
    let trailer_u16 = r.u16()?;
    let sequence_file = if seqpath {
        // `ReadSizedUTF8`: i32 length, then that many UTF-8 bytes.
        let length = usize::try_from(r.i32()?)
            .map_err(|_| ImportError::invalid(r.source, "negative sequence file length"))?;
        let raw = r.take(length)?;
        Some(
            String::from_utf8(raw.to_vec())
                .map_err(|_| ImportError::invalid(r.source, "sequence file is not UTF-8"))?,
        )
    } else {
        None
    };
    Ok(Attack {
        target: first & 0x0F,
        default_ally: first & 0x10 != 0,
        display_stats: first >> 5,
        vfx_word,
        vfx_index: (vfx_word & 0x1FF) as u16,
        flags_byte,
        morph_force_access: seqpath && flags_byte & 1 != 0,
        morph_disable_access: seqpath && flags_byte & 2 != 0,
        alternate_idle_access: seqpath && flags_byte & 4 != 0,
        for_dead: flags_byte & 0x20 != 0,
        default_camera: flags_byte & 0x40 != 0,
        default_on_dead: flags_byte & 0x80 != 0,
        script_id,
        power,
        elements,
        rate,
        category,
        add_status_no,
        mp,
        attack_type,
        vfx2,
        trailer_u16,
        sequence_file,
    })
}

/// `INB_*.inb` (`BBGINFO.ReadBattleInfo`): sixteen bytes of named fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleInfo {
    pub bbgnumber: i16,
    pub texanim: i16,
    pub skyrotation: i16,
    pub fog: i16,
    pub objanim: i16,
    pub uvcount: i16,
    pub chr_r: u8,
    pub chr_g: u8,
    pub chr_b: u8,
    pub shadow: u8,
    pub trailing_bytes: usize,
}

impl BattleInfo {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut r = Reader::new(bytes, source);
        let mut info = Self {
            bbgnumber: r.i16()?,
            texanim: r.i16()?,
            skyrotation: r.i16()?,
            fog: r.i16()?,
            objanim: r.i16()?,
            uvcount: r.i16()?,
            chr_r: r.u8()?,
            chr_g: r.u8()?,
            chr_b: r.u8()?,
            shadow: r.u8()?,
            trailing_bytes: 0,
        };
        info.trailing_bytes = bytes.len() - r.position;
        Ok(info)
    }
}

/// `.raw17` header (`btlseq.ReadBattleSequence`), structure level only: the sequence
/// opcode streams are not decoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceHeader {
    pub seq_block_offset: i16,
    pub cam_offset: i16,
    pub seq_offsets: Vec<i16>,
    pub anim_list: Vec<i32>,
    pub seq_base_anim: Vec<u8>,
    /// Bytes consumed by the header tables.
    pub header_len: usize,
    pub file_len: usize,
}

impl SequenceHeader {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut r = Reader::new(bytes, source);
        let seq_block_offset = r.i16()?;
        let cam_offset = r.i16()?;
        let seq_count = usize::try_from(r.i16()?)
            .map_err(|_| ImportError::invalid(source, "negative sequence count"))?;
        let anim_count = usize::try_from(r.i16()?)
            .map_err(|_| ImportError::invalid(source, "negative animation count"))?;
        let seq_offsets = (0..seq_count)
            .map(|_| r.i16())
            .collect::<Result<Vec<_>, _>>()?;
        let anim_list = (0..anim_count)
            .map(|_| r.i32())
            .collect::<Result<Vec<_>, _>>()?;
        let seq_base_anim = (0..seq_count)
            .map(|_| r.u8())
            .collect::<Result<Vec<_>, _>>()?;
        // Memoria seeks to `seqOffset[i] + 4`; every start must lie inside the file.
        for offset in &seq_offsets {
            let start = i64::from(*offset) + 4;
            if start < 0 || start as usize > bytes.len() {
                return Err(ImportError::invalid(
                    source,
                    format!("sequence offset {offset} points outside the file"),
                ));
            }
        }
        Ok(Self {
            seq_block_offset,
            cam_offset,
            seq_offsets,
            anim_list,
            seq_base_anim,
            header_len: r.position,
            file_len: bytes.len(),
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    source: &'a str,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], source: &'a str) -> Self {
        Self {
            bytes,
            position: 0,
            source,
        }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ImportError> {
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                ImportError::invalid(
                    self.source,
                    format!(
                        "truncated: need {length} bytes at offset {}, file has {}",
                        self.position,
                        self.bytes.len()
                    ),
                )
            })?;
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ImportError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ImportError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Result<i16, ImportError> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32, ImportError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32, ImportError> {
        Ok(self.u32()? as i32)
    }
}

// ---------------------------------------------------------------------------
// Bundle access
// ---------------------------------------------------------------------------

const SCENE_PREFIX: &str = "assets/resources/battlemap/battlescene/evt_battle_";
const INFO_PREFIX: &str = "assets/resources/battlemap/battleinfo/";

/// Everything battle-related read from one bundle. Per-entry failures are kept, not hidden.
#[derive(Debug, Default)]
pub struct BattleBundleData {
    /// Keyed by upper-case `EVT_BATTLE_<name>`.
    pub scenes: BTreeMap<String, Result<BattleScene, ImportError>>,
    /// Keyed by upper-case `EVT_BATTLE_<name>`; the `.raw17` file name is kept alongside.
    pub sequences: BTreeMap<String, (String, Result<SequenceHeader, ImportError>)>,
    /// Raw `.raw17` bytes, keyed like `sequences` (input for the sequence decoder).
    pub sequence_bytes: BTreeMap<String, Vec<u8>>,
    /// Keyed by upper-case `INB_<name>`.
    pub infos: BTreeMap<String, Result<BattleInfo, ImportError>>,
}

impl BattleBundleData {
    /// Reads and parses every battle scene, sequence header and battle info in the bundle.
    pub fn load(bundle_path: &Path) -> Result<Self, ImportError> {
        let source = bundle_path.display().to_string();
        let bundle = UnityRawBundle::open(bundle_path)?;
        let bytes = bundle.read_entry(0, 600 * 1024 * 1024)?;
        let serialized = SerializedFile::parse(&bytes, &source)?;
        let metadata_index = serialized
            .report()
            .objects
            .iter()
            .position(|object| object.class_id == 142)
            .ok_or_else(|| ImportError::invalid(&source, "AssetBundle metadata is missing"))?;
        let metadata = AssetBundleMetadata::parse(
            serialized.object_data(metadata_index)?,
            &format!("{source}:AssetBundle"),
        )?;
        let index_by_path_id: BTreeMap<i64, usize> = serialized
            .report()
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| (object.path_id, index))
            .collect();
        let read = |info: &AssetInfo, label: &str| -> Result<Vec<u8>, ImportError> {
            let index = index_by_path_id.get(&info.path_id).ok_or_else(|| {
                ImportError::invalid(
                    label,
                    format!("path ID {} is not in the bundle", info.path_id),
                )
            })?;
            Ok(TextAsset::parse(serialized.object_data(*index)?, label)?.script)
        };

        let mut data = Self::default();
        for (path, info) in &metadata.container {
            if let Some(rest) = path.strip_prefix(SCENE_PREFIX) {
                let Some((scene, file)) = rest.split_once('/') else {
                    continue;
                };
                let key = format!("EVT_BATTLE_{}", scene.to_ascii_uppercase());
                if file == "dbfile0000.raw16.bytes" {
                    let parsed = read(info, &key).and_then(|b| BattleScene::parse(&b, &key));
                    data.scenes.insert(key, parsed);
                } else if let Some(name) = file.strip_suffix(".raw17.bytes") {
                    let parsed = match read(info, &key) {
                        Ok(raw) => {
                            let parsed = SequenceHeader::parse(&raw, &key);
                            data.sequence_bytes.insert(key.clone(), raw);
                            parsed
                        }
                        Err(error) => Err(error),
                    };
                    data.sequences
                        .insert(key, (format!("{name}.raw17"), parsed));
                }
            } else if let Some(file) = path.strip_prefix(INFO_PREFIX)
                && let Some(name) = file.strip_suffix(".inb.bytes")
            {
                let key = name.to_ascii_uppercase();
                let parsed = read(info, &key).and_then(|b| BattleInfo::parse(&b, &key));
                data.infos.insert(key, parsed);
            }
        }
        Ok(data)
    }

    /// Looks a scene up by `EVT_BATTLE_<name>` or bare `<name>`, ignoring case.
    pub fn scene(&self, name: &str) -> Option<(&str, &Result<BattleScene, ImportError>)> {
        let upper = name.to_ascii_uppercase();
        let key = if upper.starts_with("EVT_BATTLE_") {
            upper
        } else {
            format!("EVT_BATTLE_{upper}")
        };
        self.scenes
            .get_key_value(&key)
            .map(|(key, value)| (key.as_str(), value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    fn le32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn vanilla_pattern(out: &mut Vec<u8>) {
        out.extend_from_slice(&[10, 2, 1, 0]);
        le32(out, 99);
        for slot in 0..4_u8 {
            out.extend_from_slice(&[slot, 1, 7, 0]);
            le16(out, 100 + u16::from(slot));
            le16(out, 0);
            le16(out, 0xFFFE); // -2
            le16(out, 4);
        }
    }

    fn vanilla_monster(out: &mut Vec<u8>) {
        let start = out.len();
        for status in [0x11_u32, 0x22, 0x33] {
            le32(out, status);
        }
        for v in [500_u16, 20, 300, 40] {
            le16(out, v);
        }
        out.extend_from_slice(&[1, 2, 3, 4]); // win items
        out.extend_from_slice(&[5, 6, 7, 8]); // steal items
        le16(out, 77); // radius
        le16(out, 0xFFFF); // geo = -1
        for v in 0..6_u16 {
            le16(out, v);
        }
        le16(out, 11);
        le16(out, 12);
        le16(out, 3); // flags
        le16(out, 9); // ap
        out.extend_from_slice(&[21, 22, 23, 24]); // stats
        out.extend_from_slice(&[0, 1, 2, 3]); // pad, trans, cur, max
        out.extend_from_slice(&[4, 5, 6, 7]); // elements
        out.extend_from_slice(&[8, 9]); // level, category
        out.extend_from_slice(&[10, 11, 12, 13, 14, 15]);
        out.extend_from_slice(&[1, 2, 3, 4]); // bone
        le16(out, 300);
        out.extend_from_slice(&[0, 5]);
        out.extend_from_slice(&[1; 6]);
        out.extend_from_slice(&[0xFF, 2, 3, 4, 5, 6]); // icon y: first is -1
        out.extend_from_slice(&[7; 6]);
        le16(out, 1);
        le16(out, 2);
        le16(out, 3);
        out.push(4); // shadow bone
        out.push(5); // win card
        le16(out, 0xFFFF);
        le16(out, 6);
        out.push(7);
        out.push(0);
        le16(out, 0);
        le16(out, 0);
        assert_eq!(out.len() - start, 116);
    }

    fn vanilla_attack(out: &mut Vec<u8>) {
        let start = out.len();
        out.push(0b0101_0011); // target 3, default ally, display 2
        le16(out, 0x0205); // vfx word, index 0x005 | high bits
        out.push(0b1010_0000); // for_dead, default_on_dead
        out.extend_from_slice(&[11, 22, 33, 44, 55, 66, 77, 88]);
        le16(out, 0x1234); // vfx2
        le16(out, 0xBEEF); // trailer
        assert_eq!(out.len() - start, 16);
    }

    fn vanilla_scene() -> Vec<u8> {
        let mut out = vec![7, 1, 1, 1];
        le16(&mut out, 0x0021);
        le16(&mut out, 0);
        vanilla_pattern(&mut out);
        vanilla_monster(&mut out);
        vanilla_attack(&mut out);
        out
    }

    #[test]
    fn parses_vanilla_scene() {
        let scene = BattleScene::parse(&vanilla_scene(), "fixture").unwrap();
        assert_eq!(scene.trailing_bytes, 0);
        assert_eq!(scene.header.version, 7);
        assert_eq!(scene.header.battle_map_index, None);
        assert_eq!(scene.header.flag_names(), vec!["special", "no_run_away"]);
        let pattern = &scene.patterns[0];
        assert_eq!(
            (pattern.rate, pattern.monster_count, pattern.ap),
            (10, 2, 99)
        );
        assert_eq!(pattern.monsters[3].x, 103);
        assert_eq!(pattern.monsters[3].z, -2);
        let monster = &scene.monster_types[0];
        assert_eq!(monster.resist_status, 0x11);
        assert_eq!((monster.max_hp, monster.win_exp), (500, 40));
        assert_eq!(monster.steal_items, [5, 6, 7, 8]);
        assert_eq!(monster.win_item_rates, [256, 96, 32, 1]);
        assert_eq!(monster.geo, -1);
        assert_eq!(monster.stats.strength, 22);
        assert_eq!(monster.guard_element, ByteOrTokens::Byte(4));
        assert_eq!((monster.level, monster.hit_rate), (8, 10));
        assert_eq!(monster.icon_y[0], -1);
        assert_eq!(monster.win_card, 5);
        assert_eq!(monster.shadow_ofs_x, -1);
        let attack = &scene.attacks[0];
        assert_eq!(
            (attack.target, attack.default_ally, attack.display_stats),
            (3, true, 2)
        );
        assert_eq!(attack.vfx_index, 5);
        assert!(attack.for_dead && attack.default_on_dead && !attack.default_camera);
        assert!(!attack.morph_force_access);
        assert_eq!((attack.script_id, attack.mp, attack.vfx2), (11, 77, 0x1234));
        assert_eq!(attack.trailer_u16, 0xBEEF);
        assert!(attack.sequence_file.is_none());
    }

    #[test]
    fn reports_trailing_and_truncated() {
        let mut bytes = vanilla_scene();
        bytes.extend_from_slice(&[1, 2, 3]);
        assert_eq!(BattleScene::parse(&bytes, "f").unwrap().trailing_bytes, 3);
        bytes.truncate(bytes.len() - 10);
        assert!(BattleScene::parse(&bytes, "f").is_err());
    }

    #[test]
    fn rejects_unknown_version() {
        let mut bytes = vanilla_scene();
        bytes[0] = 11;
        assert!(BattleScene::parse(&bytes, "f").is_err());
    }

    #[test]
    fn parses_extended_version_ten_scene() {
        let mut out = vec![10, 0, 1, 1, 0, 0, 0, 0];
        le16(&mut out, 321); // battle map index
        le32(&mut out, 2); // resist status: indices 1 and 3
        le32(&mut out, 1);
        le32(&mut out, 3);
        le32(&mut out, 0); // auto
        le32(&mut out, 0); // initial
        for v in [70000_u32, 900, 1000, 2000] {
            le32(&mut out, v);
        }
        for v in 1..=8_u32 {
            le32(&mut out, v); // win items then steal items
        }
        for v in 1..=9_u32 {
            le32(&mut out, v); // 4 win rates, 4 steal rates, card rate
        }
        le16(&mut out, 5); // radius
        le16(&mut out, 6); // geo
        for _ in 0..9 {
            le16(&mut out, 0); // mot[6], mesh[2], flags
        }
        le32(&mut out, 123_456); // ap
        for v in [1_u32, 2, 3, 4] {
            le32(&mut out, v);
        }
        out.extend_from_slice(&[0; 4]); // pad, trans, cur, max
        for tokens in [&[1_i32, 2][..], &[], &[3], &[]] {
            le32(&mut out, tokens.len() as u32);
            for token in tokens {
                le32(&mut out, *token as u32);
            }
        }
        le32(&mut out, 50); // level
        le32(&mut out, 1); // category tokens
        le32(&mut out, 4);
        for v in [100_u32, 200, 300, 400, 500, 600] {
            le32(&mut out, v);
        }
        out.extend_from_slice(&[0; 4]); // bone
        le16(&mut out, 0); // die sfx
        out.extend_from_slice(&[0, 0]); // konran, mes_cnt
        out.extend_from_slice(&[0; 18]); // icon bone, y, z
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, 0);
        out.push(0); // shadow bone
        le32(&mut out, 12); // win card
        le16(&mut out, 0);
        le16(&mut out, 0);
        out.extend_from_slice(&[0, 0]);
        le16(&mut out, 0);
        le16(&mut out, 0);
        // attack
        out.push(0);
        le32(&mut out, 0x1FF);
        out.push(0b0000_0101);
        le32(&mut out, 1); // script
        le32(&mut out, 2); // power
        le32(&mut out, 1); // element tokens
        le32(&mut out, 9);
        le32(&mut out, 3); // rate
        out.push(4);
        le32(&mut out, 5);
        le32(&mut out, 6);
        out.push(7);
        le32(&mut out, 8);
        le16(&mut out, 0);
        le32(&mut out, 3);
        out.extend_from_slice(b"abc");

        let scene = BattleScene::parse(&out, "ext").unwrap();
        assert_eq!(scene.trailing_bytes, 0);
        assert_eq!(scene.header.battle_map_index, Some(321));
        let monster = &scene.monster_types[0];
        assert_eq!(monster.resist_status, 0b1010);
        assert_eq!((monster.max_hp, monster.ap), (70000, 123_456));
        assert_eq!(monster.win_item_rates, [1, 2, 3, 4]);
        assert_eq!(monster.win_card_rate, 9);
        assert_eq!(monster.guard_element, ByteOrTokens::Tokens(vec![1, 2]));
        assert_eq!(monster.category, ByteOrTokens::Tokens(vec![4]));
        assert_eq!(
            (monster.level, monster.blue_magic, monster.win_card),
            (50, 600, 12)
        );
        let attack = &scene.attacks[0];
        assert_eq!(attack.vfx_index, 0x1FF);
        assert!(attack.morph_force_access && attack.alternate_idle_access);
        assert!(!attack.morph_disable_access);
        assert_eq!(attack.elements, ByteOrTokens::Tokens(vec![9]));
        assert_eq!(attack.add_status_no, 5);
        assert_eq!(attack.sequence_file.as_deref(), Some("abc"));
    }

    #[test]
    fn parses_battle_info() {
        let mut bytes = Vec::new();
        for v in [3_i16, 1, -2, 4, 5, 6] {
            le16(&mut bytes, v as u16);
        }
        bytes.extend_from_slice(&[10, 20, 30, 1]);
        let info = BattleInfo::parse(&bytes, "inb").unwrap();
        assert_eq!((info.bbgnumber, info.skyrotation, info.uvcount), (3, -2, 6));
        assert_eq!((info.chr_r, info.chr_b, info.shadow), (10, 30, 1));
        assert_eq!(info.trailing_bytes, 0);
        assert!(BattleInfo::parse(&bytes[..13], "inb").is_err());
    }

    #[test]
    fn parses_sequence_header() {
        let mut bytes = Vec::new();
        for v in [14_u16, 20, 2, 1] {
            le16(&mut bytes, v);
        }
        le16(&mut bytes, 10);
        le16(&mut bytes, 12);
        le32(&mut bytes, 0x0102_0304);
        bytes.extend_from_slice(&[0, 1]);
        bytes.extend_from_slice(&[0; 8]);
        let header = SequenceHeader::parse(&bytes, "raw17").unwrap();
        assert_eq!(header.seq_offsets, vec![10, 12]);
        assert_eq!(header.anim_list, vec![0x0102_0304]);
        assert_eq!(header.seq_base_anim, vec![0, 1]);
        assert_eq!(header.header_len, 8 + 4 + 4 + 2);
        assert!(SequenceHeader::parse(&bytes[..9], "raw17").is_err());
    }

    /// Runs only when a real install is configured through `FFIX_BUNDLE_P0DATA2`.
    #[test]
    fn real_bundle_scenes_parse_with_known_tail() {
        let Some(path) = std::env::var_os("FFIX_BUNDLE_P0DATA2") else {
            return;
        };
        let path = Path::new(&path);
        if !path.is_file() {
            return;
        }
        let data = BattleBundleData::load(path).unwrap();
        assert_eq!(data.scenes.len(), 562);
        for (name, scene) in &data.scenes {
            let scene = scene.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
            // The tail is one optional leading dword followed by zero padding.
            assert_eq!(scene.trailing_bytes % 4, 0, "{name}");
            assert!(scene.trailing.iter().skip(4).all(|b| *b == 0), "{name}");
        }
    }
}
