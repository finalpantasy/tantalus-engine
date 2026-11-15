//! Typed legacy FF9 save tree (milestone G7): the `Data` tree that pinned Memoria's
//! `JsonParser.ParseFromFF9StateSystem` builds and `SharedDataBytesStorage` flattens into the
//! `SAVE` payload, plus the `MemoriaExtraData` sidecar tree.
//!
//! Every field name, type and array length below is read from the C# in
//! `Assembly-CSharp/Global/JsonParser.cs`; `audit/save-tree-schema.md` lists each one with its
//! source line and a "Not source-backed" section for what could not be read.
//!
//! # API note
//!
//! * [`Ff9SaveTree`] is the base tree (`Data` sections `10000_State` .. `99000_Other`).
//!   [`Ff9SaveTree::blank`] builds an all-zero valid tree, [`Ff9SaveTree::encode_payload`]
//!   flattens it to the payload bytes that follow the `SAVE` magic (the same bytes as
//!   `BaseSaveBlock::payload`), and [`Ff9SaveTree::decode_payload`] is the inverse. Decoding
//!   requires the payload to be exactly [`Ff9SaveTree::PAYLOAD_BYTES`] long.
//! * The traversal reuses [`BaseSavePayload`] for the primitive layer. Class keys are visited in
//!   sorted order; [`KeyOrder`] picks the comparer. The reference calls `List<string>.Sort()`
//!   (culture-sensitive); that comparer is not verified against a real save, so both
//!   [`KeyOrder::Ordinal`] (the default, same as `BaseSaveSchemaNode`) and an approximation of
//!   culture order are offered. They only differ for `80000_Achievement` and `98000_Achievement`.
//! * [`EventGlobal`] wraps the 2048-byte `gEventGlobal` array (base64 in the tree) with the named
//!   byte-range accessors Memoria uses.
//! * [`MemoriaExtra`] is the typed sidecar (`SimpleJsonNode` binary tree). Numeric leaves are
//!   written with the tag rule from `JSONData.Serialize`; `time` keeps its original leaf so a
//!   decoded sidecar re-encodes byte for byte.
//! * Only synthetic data is covered by tests; real saves are `NOT_RUN`.

use super::{
    BaseSaveDataNode as D, BaseSaveFieldType as T, BaseSavePayload, BaseSaveSchemaNode as S,
    BaseSaveValue as V, LegacySaveError, SimpleJsonNode,
};
use std::cmp::Ordering;
use std::collections::BTreeMap;

type E = LegacySaveError;
type Pair = (S, D);

fn err(context: impl Into<String>) -> E {
    E::new(0, context)
}

/// Comparer used to order class keys when flattening (`nodeFieldList.Sort()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyOrder {
    /// Byte-wise comparison (matches `BaseSaveSchemaNode`).
    #[default]
    Ordinal,
    /// Approximation of .NET culture comparison: symbols < digits < letters, case-insensitive
    /// first, lowercase before uppercase on ties. Not verified against a real save.
    CultureApprox,
}

impl KeyOrder {
    fn compare(self, left: &str, right: &str) -> Ordering {
        match self {
            Self::Ordinal => left.cmp(right),
            Self::CultureApprox => {
                let weight = |c: char| {
                    let class = if c.is_ascii_digit() {
                        1
                    } else if c.is_alphabetic() {
                        2
                    } else {
                        0
                    };
                    (class, c.to_lowercase().next().unwrap_or(c))
                };
                let primary = left.chars().map(weight).cmp(right.chars().map(weight));
                // Lowercase sorts before uppercase on a case-only tie.
                primary.then_with(|| right.cmp(left))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Flattening (mirror of ParseJsonTreeToDataList / ParseDataListToJsonTree).
// ---------------------------------------------------------------------------------------------

fn sorted_children<X>(items: &[(String, X)], order: KeyOrder) -> Vec<&(String, X)> {
    let mut refs: Vec<_> = items.iter().collect();
    refs.sort_by(|l, r| order.compare(&l.0, &r.0));
    refs
}

fn flatten_types(schema: &S, order: KeyOrder, out: &mut Vec<T>) {
    match schema {
        S::Field(t) => out.push(*t),
        S::Array(children) => children.iter().for_each(|c| flatten_types(c, order, out)),
        S::Class(children) => {
            for (_, child) in sorted_children(children, order) {
                flatten_types(child, order, out);
            }
        }
    }
}

fn flatten_values(data: &D, schema: &S, order: KeyOrder, out: &mut Vec<V>) -> Result<(), E> {
    match (data, schema) {
        (D::Value(v), S::Field(_)) => out.push(v.clone()),
        (D::Array(values), S::Array(children)) if values.len() == children.len() => {
            for (value, child) in values.iter().zip(children) {
                flatten_values(value, child, order, out)?;
            }
        }
        (D::Class(values), S::Class(children)) => {
            let mut map = BTreeMap::new();
            for (key, value) in values {
                if map.insert(key.as_str(), value).is_some() {
                    return Err(err(format!("duplicate class key `{key}`")));
                }
            }
            if map.len() != children.len() {
                return Err(err("class key set differs from the schema"));
            }
            for (key, child) in sorted_children(children, order) {
                let value = map
                    .get(key.as_str())
                    .ok_or_else(|| err(format!("missing class key `{key}`")))?;
                flatten_values(value, child, order, out)?;
            }
        }
        _ => return Err(err("tree shape does not match the schema")),
    }
    Ok(())
}

fn unflatten(schema: &S, order: KeyOrder, values: &mut std::vec::IntoIter<V>) -> Result<D, E> {
    Ok(match schema {
        S::Field(_) => D::Value(values.next().ok_or_else(|| err("value list exhausted"))?),
        S::Array(children) => D::Array(
            children
                .iter()
                .map(|c| unflatten(c, order, values))
                .collect::<Result<_, _>>()?,
        ),
        S::Class(children) => {
            let mut out = Vec::with_capacity(children.len());
            for (key, child) in sorted_children(children, order) {
                out.push((key.clone(), unflatten(child, order, values)?));
            }
            D::Class(out)
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Pair builders (schema and data are produced together so key lists exist once).
// ---------------------------------------------------------------------------------------------

fn leaf(t: T, v: V) -> Pair {
    (S::Field(t), D::Value(v))
}
fn p_u8(x: u8) -> Pair {
    leaf(T::Byte, V::U8(x))
}
fn p_i8(x: i8) -> Pair {
    leaf(T::SByte, V::I8(x))
}
fn p_i16(x: i16) -> Pair {
    leaf(T::Int16, V::I16(x))
}
fn p_u16(x: u16) -> Pair {
    leaf(T::UInt16, V::U16(x))
}
fn p_i32(x: i32) -> Pair {
    leaf(T::Int32, V::I32(x))
}
fn p_u32(x: u32) -> Pair {
    leaf(T::UInt32, V::U32(x))
}
fn p_u64(x: u64) -> Pair {
    leaf(T::UInt64, V::U64(x))
}
fn p_f32(x: f32) -> Pair {
    leaf(T::Single, V::F32(x))
}
fn p_f64(x: f64) -> Pair {
    leaf(T::Double, V::F64(x))
}
fn p_bool(x: bool) -> Pair {
    leaf(T::Boolean, V::Bool(x))
}
fn p_str(x: &str) -> Pair {
    leaf(T::String128, V::String(x.to_owned()))
}
fn p_arr(items: Vec<Pair>) -> Pair {
    let (s, d): (Vec<S>, Vec<D>) = items.into_iter().unzip();
    (S::Array(s), D::Array(d))
}
fn p_cls(items: Vec<(&str, Pair)>) -> Pair {
    let (s, d): (Vec<_>, Vec<_>) = items
        .into_iter()
        .map(|(k, (s, d))| ((k.to_owned(), s), (k.to_owned(), d)))
        .unzip();
    (S::Class(s), D::Class(d))
}
fn p_i32s(items: &[i32], len: usize, name: &str) -> Result<Pair, E> {
    check_len(name, items.len(), len)?;
    Ok(p_arr(items.iter().map(|x| p_i32(*x)).collect()))
}
fn check_len(name: &str, got: usize, want: usize) -> Result<(), E> {
    if got == want {
        Ok(())
    } else {
        Err(err(format!(
            "{name} has {got} elements, schema needs {want}"
        )))
    }
}

// ---------------------------------------------------------------------------------------------
// Data node readers.
// ---------------------------------------------------------------------------------------------

fn get<'a>(node: &'a D, key: &str) -> Result<&'a D, E> {
    match node {
        D::Class(fields) => fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .ok_or_else(|| err(format!("missing key `{key}`"))),
        _ => Err(err(format!("expected a class while reading `{key}`"))),
    }
}
fn items(node: &D) -> Result<&[D], E> {
    match node {
        D::Array(values) => Ok(values),
        _ => Err(err("expected an array")),
    }
}
macro_rules! scalar_reader {
    ($name:ident, $ty:ty, $variant:ident) => {
        fn $name(node: &D) -> Result<$ty, E> {
            match node {
                D::Value(V::$variant(x)) => Ok(*x),
                _ => Err(err(concat!("expected ", stringify!($variant)))),
            }
        }
    };
}
scalar_reader!(r_u8, u8, U8);
scalar_reader!(r_i8, i8, I8);
scalar_reader!(r_i16, i16, I16);
scalar_reader!(r_u16, u16, U16);
scalar_reader!(r_i32, i32, I32);
scalar_reader!(r_u32, u32, U32);
scalar_reader!(r_u64, u64, U64);
scalar_reader!(r_f32, f32, F32);
scalar_reader!(r_f64, f64, F64);
scalar_reader!(r_bool, bool, Bool);
fn r_str(node: &D) -> Result<String, E> {
    match node {
        D::Value(V::String(x)) => Ok(x.clone()),
        _ => Err(err("expected String")),
    }
}
fn r_i32s(node: &D, len: usize, name: &str) -> Result<Vec<i32>, E> {
    let values = items(node)?;
    check_len(name, values.len(), len)?;
    values.iter().map(r_i32).collect()
}

// ---------------------------------------------------------------------------------------------
// Base64 (standard alphabet, padded), for gEventGlobal.
// ---------------------------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(text: &str) -> Result<Vec<u8>, E> {
    let raw = text.as_bytes();
    if !raw.len().is_multiple_of(4) {
        return Err(err("base64 length is not a multiple of 4"));
    }
    let mut out = Vec::with_capacity(raw.len() / 4 * 3);
    for (index, quad) in raw.chunks(4).enumerate() {
        let last = (index + 1) * 4 == raw.len();
        let pad = quad.iter().rev().take_while(|b| **b == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return Err(err("base64 padding is misplaced"));
        }
        let mut n = 0u32;
        for (position, byte) in quad.iter().enumerate() {
            let value = if position >= 4 - pad {
                0
            } else {
                B64.iter()
                    .position(|c| c == byte)
                    .ok_or_else(|| err("invalid base64 character"))? as u32
            };
            n = (n << 6) | value;
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..4 - pad]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// gEventGlobal.
// ---------------------------------------------------------------------------------------------

/// The 2048-byte `gEventGlobal` array (`EventState.cs:10`), stored as base64 in the tree.
#[derive(Clone, PartialEq, Eq)]
pub struct EventGlobal {
    bytes: Vec<u8>,
}

impl std::fmt::Debug for EventGlobal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "EventGlobal({} bytes, {} non-zero)",
            self.bytes.len(),
            self.bytes.iter().filter(|b| **b != 0).count()
        )
    }
}

impl Default for EventGlobal {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl EventGlobal {
    pub const LEN: usize = 2048;
    /// `ScenarioCounter`: little-endian u16 at bytes 0-1 (`EventState.cs:16-24`).
    pub const SCENARIO_COUNTER: usize = 0;
    /// `FieldEntrance`: little-endian u16 at bytes 2-3 (`EventState.cs:26-34`).
    pub const FIELD_ENTRANCE: usize = 2;
    /// Chocograph "opened" (gem) bit field, 24 bits at bytes 184-186 (`ChocographUI.cs:251`).
    pub const CHOCOGRAPH_OPENED: std::ops::RangeInclusive<usize> = 184..=186;
    /// Chocograph "found" (map) bit field, 24 bits at bytes 187-189 (`ChocographUI.cs:250`).
    pub const CHOCOGRAPH_FOUND: std::ops::RangeInclusive<usize> = 187..=189;
    /// Chocograph ability byte (`ChocographUI.cs:245`).
    pub const CHOCOGRAPH_ABILITY: usize = 191;
    /// Treasure Hunter point ranges (`EventState.cs:65-70`): one point per set bit in the first
    /// two ranges, two points per set bit in the third.
    pub const TREASURE_HUNTER_ONE_POINT: [std::ops::RangeInclusive<usize>; 2] =
        [896..=960, 966..=975];
    pub const TREASURE_HUNTER_TWO_POINTS: std::ops::RangeInclusive<usize> = 182..=186;
    /// Legacy ability usage counters lived at `1100 + id` for ids 1..192 (`JsonParser.cs:538`).
    pub const LEGACY_ABILITY_USAGE_BASE: usize = 1100;

    pub fn zeroed() -> Self {
        Self {
            bytes: vec![0; Self::LEN],
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, E> {
        check_len("gEventGlobal", bytes.len(), Self::LEN)?;
        Ok(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    pub fn to_base64(&self) -> String {
        base64_encode(&self.bytes)
    }

    pub fn from_base64(text: &str) -> Result<Self, E> {
        Self::from_bytes(base64_decode(text)?)
    }

    fn le(&self, range: std::ops::RangeInclusive<usize>) -> u32 {
        range.enumerate().fold(0, |acc, (i, at)| {
            acc | (u32::from(self.bytes[at]) << (8 * i))
        })
    }

    pub fn scenario_counter(&self) -> u16 {
        self.le(Self::SCENARIO_COUNTER..=1) as u16
    }
    pub fn set_scenario_counter(&mut self, value: u16) {
        self.bytes[0..2].copy_from_slice(&value.to_le_bytes());
    }
    pub fn field_entrance(&self) -> u16 {
        self.le(Self::FIELD_ENTRANCE..=3) as u16
    }
    pub fn set_field_entrance(&mut self, value: u16) {
        self.bytes[2..4].copy_from_slice(&value.to_le_bytes());
    }
    /// Bit `i` (0..24) set means hint map `i` was opened.
    pub fn chocograph_opened(&self) -> u32 {
        self.le(Self::CHOCOGRAPH_OPENED)
    }
    pub fn set_chocograph_opened(&mut self, bits: u32) {
        self.bytes[184..=186].copy_from_slice(&bits.to_le_bytes()[..3]);
    }
    /// Bit `i` (0..24) set means hint map `i` was found.
    pub fn chocograph_found(&self) -> u32 {
        self.le(Self::CHOCOGRAPH_FOUND)
    }
    pub fn set_chocograph_found(&mut self, bits: u32) {
        self.bytes[187..=189].copy_from_slice(&bits.to_le_bytes()[..3]);
    }
    pub fn chocograph_ability(&self) -> u8 {
        self.bytes[Self::CHOCOGRAPH_ABILITY]
    }
    pub fn set_chocograph_ability(&mut self, value: u8) {
        self.bytes[Self::CHOCOGRAPH_ABILITY] = value;
    }
    /// `EventState.GetTreasureHunterPoints`.
    pub fn treasure_hunter_points(&self) -> u32 {
        let count = |range: std::ops::RangeInclusive<usize>| -> u32 {
            range.map(|i| self.bytes[i].count_ones()).sum()
        };
        count(Self::TREASURE_HUNTER_ONE_POINT[0].clone())
            + count(Self::TREASURE_HUNTER_ONE_POINT[1].clone())
            + 2 * count(Self::TREASURE_HUNTER_TWO_POINTS)
    }
    /// Legacy `gEventGlobal[1100 + id]` ability usage counter for `id` in 1..192.
    pub fn legacy_ability_usage(&self, id: usize) -> Option<u8> {
        (1..192)
            .contains(&id)
            .then(|| self.bytes[Self::LEGACY_ABILITY_USAGE_BASE + id])
    }
}

// ---------------------------------------------------------------------------------------------
// Base tree sections.
// ---------------------------------------------------------------------------------------------

/// `10000_State` (`JsonParser.cs:415-513`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StateSection {
    pub mode: u8,
    pub prev_mode: u8,
    pub fld_map_no: i16,
    pub fld_loc_no: i16,
    pub btl_map_no: i16,
    pub btl_sub_map_no: i8,
    pub wld_map_no: i16,
    pub wld_loc_no: i16,
    pub time_counter: f32,
    pub timer_control: bool,
    pub timer_display: bool,
}

impl StateSection {
    fn pair(&self) -> Pair {
        p_cls(vec![
            ("mode", p_u8(self.mode)),
            ("prevMode", p_u8(self.prev_mode)),
            ("fldMapNo", p_i16(self.fld_map_no)),
            ("fldLocNo", p_i16(self.fld_loc_no)),
            ("btlMapNo", p_i16(self.btl_map_no)),
            ("btlSubMapNo", p_i8(self.btl_sub_map_no)),
            ("wldMapNo", p_i16(self.wld_map_no)),
            ("wldLocNo", p_i16(self.wld_loc_no)),
            ("timeCounter", p_f32(self.time_counter)),
            ("timerControl", p_bool(self.timer_control)),
            ("timerDisplay", p_bool(self.timer_display)),
        ])
    }
    fn read(n: &D) -> Result<Self, E> {
        Ok(Self {
            mode: r_u8(get(n, "mode")?)?,
            prev_mode: r_u8(get(n, "prevMode")?)?,
            fld_map_no: r_i16(get(n, "fldMapNo")?)?,
            fld_loc_no: r_i16(get(n, "fldLocNo")?)?,
            btl_map_no: r_i16(get(n, "btlMapNo")?)?,
            btl_sub_map_no: r_i8(get(n, "btlSubMapNo")?)?,
            wld_map_no: r_i16(get(n, "wldMapNo")?)?,
            wld_loc_no: r_i16(get(n, "wldLocNo")?)?,
            time_counter: r_f32(get(n, "timeCounter")?)?,
            timer_control: r_bool(get(n, "timerControl")?)?,
            timer_display: r_bool(get(n, "timerDisplay")?)?,
        })
    }
}

/// `20000_Event` base format: `gStepCount` Int32 and `gEventGlobal` as a 4096-byte string
/// (`JsonParser.cs:575-633`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EventSection {
    pub step_count: i32,
    pub event_global: EventGlobal,
}

impl EventSection {
    fn pair(&self) -> Pair {
        p_cls(vec![
            ("gStepCount", p_i32(self.step_count)),
            (
                "gEventGlobal",
                leaf(T::String4K, V::String(self.event_global.to_base64())),
            ),
        ])
    }
    fn read(n: &D) -> Result<Self, E> {
        Ok(Self {
            step_count: r_i32(get(n, "gStepCount")?)?,
            event_global: EventGlobal::from_base64(&r_str(get(n, "gEventGlobal")?)?)?,
        })
    }
}

/// One Tetra Master card slot (`JsonParser.cs:741-761`). An empty slot is all 255.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardSlot {
    pub id: u8,
    pub side: u8,
    pub atk: u8,
    /// Int32 in the schema; empty slots write 255.
    pub card_type: i32,
    pub pdef: u8,
    pub mdef: u8,
    pub cpoint: u8,
    pub arrow: u8,
}

impl CardSlot {
    pub const EMPTY: Self = Self {
        id: 255,
        side: 255,
        atk: 255,
        card_type: 255,
        pdef: 255,
        mdef: 255,
        cpoint: 255,
        arrow: 255,
    };
}

/// `30000_MiniGame` (`JsonParser.cs:690-763`): three Int16 counters and 100 card slots.
#[derive(Debug, Clone, PartialEq)]
pub struct MiniGameSection {
    pub s_win: i16,
    pub s_lose: i16,
    pub s_draw: i16,
    pub cards: Vec<CardSlot>,
}

impl MiniGameSection {
    pub const CARDS: usize = 100;
    fn blank() -> Self {
        Self {
            s_win: 0,
            s_lose: 0,
            s_draw: 0,
            cards: vec![CardSlot::EMPTY; Self::CARDS],
        }
    }
    fn pair(&self) -> Result<Pair, E> {
        check_len("MiniGameCard", self.cards.len(), Self::CARDS)?;
        let cards = self
            .cards
            .iter()
            .map(|c| {
                p_cls(vec![
                    ("id", p_u8(c.id)),
                    ("side", p_u8(c.side)),
                    ("atk", p_u8(c.atk)),
                    ("type", p_i32(c.card_type)),
                    ("pdef", p_u8(c.pdef)),
                    ("mdef", p_u8(c.mdef)),
                    ("cpoint", p_u8(c.cpoint)),
                    ("arrow", p_u8(c.arrow)),
                ])
            })
            .collect();
        Ok(p_cls(vec![
            ("sWin", p_i16(self.s_win)),
            ("sLose", p_i16(self.s_lose)),
            ("sDraw", p_i16(self.s_draw)),
            ("MiniGameCard", p_arr(cards)),
        ]))
    }
    fn read(n: &D) -> Result<Self, E> {
        let cards = items(get(n, "MiniGameCard")?)?;
        check_len("MiniGameCard", cards.len(), Self::CARDS)?;
        Ok(Self {
            s_win: r_i16(get(n, "sWin")?)?,
            s_lose: r_i16(get(n, "sLose")?)?,
            s_draw: r_i16(get(n, "sDraw")?)?,
            cards: cards
                .iter()
                .map(|c| {
                    Ok(CardSlot {
                        id: r_u8(get(c, "id")?)?,
                        side: r_u8(get(c, "side")?)?,
                        atk: r_u8(get(c, "atk")?)?,
                        card_type: r_i32(get(c, "type")?)?,
                        pdef: r_u8(get(c, "pdef")?)?,
                        mdef: r_u8(get(c, "mdef")?)?,
                        cpoint: r_u8(get(c, "cpoint")?)?,
                        arrow: r_u8(get(c, "arrow")?)?,
                    })
                })
                .collect::<Result<_, E>>()?,
        })
    }
}

/// `cur` / `max` resource block of a player (`JsonParser.cs:1028-1043`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resources {
    pub hp: u16,
    pub mp: i16,
    pub at: i16,
    pub at_coef: i8,
    pub capa: u8,
}

impl Resources {
    fn pair(&self) -> Pair {
        p_cls(vec![
            ("hp", p_u16(self.hp)),
            ("mp", p_i16(self.mp)),
            ("at", p_i16(self.at)),
            ("at_coef", p_i8(self.at_coef)),
            ("capa", p_u8(self.capa)),
        ])
    }
    fn read(n: &D) -> Result<Self, E> {
        Ok(Self {
            hp: r_u16(get(n, "hp")?)?,
            mp: r_i16(get(n, "mp")?)?,
            at: r_i16(get(n, "at")?)?,
            at_coef: r_i8(get(n, "at_coef")?)?,
            capa: r_u8(get(n, "capa")?)?,
        })
    }
}

/// One of the nine `players` entries of `40000_Common` (`JsonParser.cs:1022-1098`).
///
/// The JSON key for `wep_bone` is written as `web_bone` (`JsonParser.cs:812`); this struct keeps
/// the written key. `status` is a Byte in the schema although the writer stringifies an Int32.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRecord {
    pub name: String,
    pub category: u8,
    pub level: u8,
    pub exp: u32,
    pub cur: Resources,
    pub max: Resources,
    pub trance: u8,
    pub web_bone: u8,
    /// `elem`: dex, str, mgc, wpr.
    pub elem: [u8; 4],
    /// `defence`: p_def, p_ev, m_def, m_ev.
    pub defence: [u8; 4],
    pub basis_max_hp: i16,
    pub basis_max_mp: i16,
    /// `basis`: dex, str, mgc, wpr.
    pub basis_stats: [u8; 4],
    /// `info`: slot_no, serial_no, row, win_pose, party, menu_type.
    pub info: [u8; 6],
    pub status: u8,
    pub equip: [u8; 5],
    /// `bonus`: dex, str, mgc, wpr (UInt16 each).
    pub bonus: [u16; 4],
    pub pa: Vec<u8>,
    pub sa: [u32; 2],
}

impl PlayerRecord {
    pub const PA_LEN: usize = 48;
    fn blank() -> Self {
        Self {
            name: String::new(),
            category: 0,
            level: 0,
            exp: 0,
            cur: Resources::default(),
            max: Resources::default(),
            trance: 0,
            web_bone: 0,
            elem: [0; 4],
            defence: [0; 4],
            basis_max_hp: 0,
            basis_max_mp: 0,
            basis_stats: [0; 4],
            info: [0; 6],
            status: 0,
            equip: [0; 5],
            bonus: [0; 4],
            pa: vec![0; Self::PA_LEN],
            sa: [0; 2],
        }
    }
    fn pair(&self) -> Result<Pair, E> {
        check_len("pa", self.pa.len(), Self::PA_LEN)?;
        let b4 = |names: [&'static str; 4], v: [u8; 4]| {
            p_cls(
                names
                    .into_iter()
                    .zip(v)
                    .map(|(k, x)| (k, p_u8(x)))
                    .collect(),
            )
        };
        Ok(p_cls(vec![
            ("name", p_str(&self.name)),
            ("category", p_u8(self.category)),
            ("level", p_u8(self.level)),
            ("exp", p_u32(self.exp)),
            ("cur", self.cur.pair()),
            ("max", self.max.pair()),
            ("trance", p_u8(self.trance)),
            ("web_bone", p_u8(self.web_bone)),
            ("elem", b4(["dex", "str", "mgc", "wpr"], self.elem)),
            (
                "defence",
                b4(["p_def", "p_ev", "m_def", "m_ev"], self.defence),
            ),
            (
                "basis",
                p_cls(vec![
                    ("max_hp", p_i16(self.basis_max_hp)),
                    ("max_mp", p_i16(self.basis_max_mp)),
                    ("dex", p_u8(self.basis_stats[0])),
                    ("str", p_u8(self.basis_stats[1])),
                    ("mgc", p_u8(self.basis_stats[2])),
                    ("wpr", p_u8(self.basis_stats[3])),
                ]),
            ),
            (
                "info",
                p_cls(
                    [
                        "slot_no",
                        "serial_no",
                        "row",
                        "win_pose",
                        "party",
                        "menu_type",
                    ]
                    .into_iter()
                    .zip(self.info)
                    .map(|(k, x)| (k, p_u8(x)))
                    .collect(),
                ),
            ),
            ("status", p_u8(self.status)),
            (
                "equip",
                p_arr(self.equip.iter().map(|x| p_u8(*x)).collect()),
            ),
            (
                "bonus",
                p_cls(
                    ["dex", "str", "mgc", "wpr"]
                        .into_iter()
                        .zip(self.bonus)
                        .map(|(k, x)| (k, p_u16(x)))
                        .collect(),
                ),
            ),
            ("pa", p_arr(self.pa.iter().map(|x| p_u8(*x)).collect())),
            ("sa", p_arr(self.sa.iter().map(|x| p_u32(*x)).collect())),
        ]))
    }
    fn read(n: &D) -> Result<Self, E> {
        let b4 = |node: &D, names: [&str; 4]| -> Result<[u8; 4], E> {
            Ok([
                r_u8(get(node, names[0])?)?,
                r_u8(get(node, names[1])?)?,
                r_u8(get(node, names[2])?)?,
                r_u8(get(node, names[3])?)?,
            ])
        };
        let basis = get(n, "basis")?;
        let info = get(n, "info")?;
        let bonus = get(n, "bonus")?;
        let equip = items(get(n, "equip")?)?;
        let pa = items(get(n, "pa")?)?;
        let sa = items(get(n, "sa")?)?;
        check_len("equip", equip.len(), 5)?;
        check_len("pa", pa.len(), Self::PA_LEN)?;
        check_len("sa", sa.len(), 2)?;
        let mut equip_out = [0u8; 5];
        for (slot, node) in equip_out.iter_mut().zip(equip) {
            *slot = r_u8(node)?;
        }
        Ok(Self {
            name: r_str(get(n, "name")?)?,
            category: r_u8(get(n, "category")?)?,
            level: r_u8(get(n, "level")?)?,
            exp: r_u32(get(n, "exp")?)?,
            cur: Resources::read(get(n, "cur")?)?,
            max: Resources::read(get(n, "max")?)?,
            trance: r_u8(get(n, "trance")?)?,
            web_bone: r_u8(get(n, "web_bone")?)?,
            elem: b4(get(n, "elem")?, ["dex", "str", "mgc", "wpr"])?,
            defence: b4(get(n, "defence")?, ["p_def", "p_ev", "m_def", "m_ev"])?,
            basis_max_hp: r_i16(get(basis, "max_hp")?)?,
            basis_max_mp: r_i16(get(basis, "max_mp")?)?,
            basis_stats: b4(basis, ["dex", "str", "mgc", "wpr"])?,
            info: {
                let mut out = [0u8; 6];
                for (slot, key) in out.iter_mut().zip([
                    "slot_no",
                    "serial_no",
                    "row",
                    "win_pose",
                    "party",
                    "menu_type",
                ]) {
                    *slot = r_u8(get(info, key)?)?;
                }
                out
            },
            status: r_u8(get(n, "status")?)?,
            equip: equip_out,
            bonus: {
                let mut out = [0u16; 4];
                for (slot, key) in out.iter_mut().zip(["dex", "str", "mgc", "wpr"]) {
                    *slot = r_u16(get(bonus, key)?)?;
                }
                out
            },
            pa: pa.iter().map(r_u8).collect::<Result<_, _>>()?,
            sa: [r_u32(&sa[0])?, r_u32(&sa[1])?],
        })
    }
}

/// `40000_Common`, old (base) format (`JsonParser.cs:765-1126`).
#[derive(Debug, Clone, PartialEq)]
pub struct CommonSection {
    /// Nine players in the legacy slot order (`SelectOldSaveSlot`).
    pub players: Vec<PlayerRecord>,
    /// Party member slots; 255 means empty.
    pub slot: [u8; 4],
    pub escape_no: u16,
    pub summon_flag: u16,
    pub gil: u32,
    pub frog_no: i16,
    pub steal_no: i16,
    pub dragon_no: i16,
    /// 256 `(id, count)` item slots; an empty slot is `(255, 0)`.
    pub items: Vec<(u8, u8)>,
    /// 64 bytes of rare-item flags (`GetRareItemByteFormat`: bit 2k obtained, 2k+1 used).
    pub rare_items: Vec<u8>,
}

impl CommonSection {
    pub const PLAYERS: usize = 9;
    pub const ITEMS: usize = 256;
    pub const RARE_ITEMS: usize = 64;
    fn blank() -> Self {
        Self {
            players: vec![PlayerRecord::blank(); Self::PLAYERS],
            slot: [255; 4],
            escape_no: 0,
            summon_flag: 0,
            gil: 0,
            frog_no: 0,
            steal_no: 0,
            dragon_no: 0,
            items: vec![(255, 0); Self::ITEMS],
            rare_items: vec![0; Self::RARE_ITEMS],
        }
    }
    fn pair(&self) -> Result<Pair, E> {
        check_len("players", self.players.len(), Self::PLAYERS)?;
        check_len("items", self.items.len(), Self::ITEMS)?;
        check_len("rareItems", self.rare_items.len(), Self::RARE_ITEMS)?;
        Ok(p_cls(vec![
            (
                "players",
                p_arr(
                    self.players
                        .iter()
                        .map(PlayerRecord::pair)
                        .collect::<Result<_, _>>()?,
                ),
            ),
            ("slot", p_arr(self.slot.iter().map(|x| p_u8(*x)).collect())),
            ("escape_no", p_u16(self.escape_no)),
            ("summon_flag", p_u16(self.summon_flag)),
            ("gil", p_u32(self.gil)),
            ("frog_no", p_i16(self.frog_no)),
            ("steal_no", p_i16(self.steal_no)),
            ("dragon_no", p_i16(self.dragon_no)),
            (
                "items",
                p_arr(
                    self.items
                        .iter()
                        .map(|(id, count)| p_cls(vec![("id", p_u8(*id)), ("count", p_u8(*count))]))
                        .collect(),
                ),
            ),
            (
                "rareItems",
                p_arr(self.rare_items.iter().map(|x| p_u8(*x)).collect()),
            ),
        ]))
    }
    fn read(n: &D) -> Result<Self, E> {
        let players = items(get(n, "players")?)?;
        check_len("players", players.len(), Self::PLAYERS)?;
        let slot = items(get(n, "slot")?)?;
        check_len("slot", slot.len(), 4)?;
        let item_nodes = items(get(n, "items")?)?;
        check_len("items", item_nodes.len(), Self::ITEMS)?;
        let rare = items(get(n, "rareItems")?)?;
        check_len("rareItems", rare.len(), Self::RARE_ITEMS)?;
        Ok(Self {
            players: players
                .iter()
                .map(PlayerRecord::read)
                .collect::<Result<_, _>>()?,
            slot: [
                r_u8(&slot[0])?,
                r_u8(&slot[1])?,
                r_u8(&slot[2])?,
                r_u8(&slot[3])?,
            ],
            escape_no: r_u16(get(n, "escape_no")?)?,
            summon_flag: r_u16(get(n, "summon_flag")?)?,
            gil: r_u32(get(n, "gil")?)?,
            frog_no: r_i16(get(n, "frog_no")?)?,
            steal_no: r_i16(get(n, "steal_no")?)?,
            dragon_no: r_i16(get(n, "dragon_no")?)?,
            items: item_nodes
                .iter()
                .map(|i| Ok((r_u8(get(i, "id")?)?, r_u8(get(i, "count")?)?)))
                .collect::<Result<_, E>>()?,
            rare_items: rare.iter().map(r_u8).collect::<Result<_, _>>()?,
        })
    }
}

/// `50000_Setting` (`JsonParser.cs:1410-1470`). All `cfg` scalars are UInt64.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingSection {
    pub sound: u64,
    pub sound_effect: u64,
    pub control: u64,
    pub cursor: u64,
    pub atb: u64,
    pub camera: u64,
    pub move_: u64,
    pub vibe: u64,
    pub btl_speed: u64,
    pub fld_msg: u64,
    pub here_icon: u64,
    pub win_type: u64,
    pub target_win: u64,
    pub control_data: u64,
    pub skip_btl_camera: u64,
    /// `HonoInputManager.DefaultInputKeysCount` = 10 Int32 key codes.
    pub control_data_keyboard: Vec<i32>,
    /// Ten 128-byte strings.
    pub control_data_joystick: Vec<String>,
    /// Always written as -1 (`JsonParser.cs:1439`).
    pub time: f32,
}

impl SettingSection {
    pub const KEYS: usize = 10;
    const CFG_KEYS: [&'static str; 15] = [
        "sound",
        "sound_effect",
        "control",
        "cursor",
        "atb",
        "camera",
        "move",
        "vibe",
        "btl_speed",
        "fld_msg",
        "here_icon",
        "win_type",
        "target_win",
        "control_data",
        "skip_btl_camera",
    ];
    fn blank() -> Self {
        Self {
            sound: 0,
            sound_effect: 0,
            control: 0,
            cursor: 0,
            atb: 0,
            camera: 0,
            move_: 0,
            vibe: 0,
            btl_speed: 0,
            fld_msg: 0,
            here_icon: 0,
            win_type: 0,
            target_win: 0,
            control_data: 0,
            skip_btl_camera: 0,
            control_data_keyboard: vec![0; Self::KEYS],
            control_data_joystick: vec![String::new(); Self::KEYS],
            time: -1.0,
        }
    }
    fn scalars(&self) -> [u64; 15] {
        [
            self.sound,
            self.sound_effect,
            self.control,
            self.cursor,
            self.atb,
            self.camera,
            self.move_,
            self.vibe,
            self.btl_speed,
            self.fld_msg,
            self.here_icon,
            self.win_type,
            self.target_win,
            self.control_data,
            self.skip_btl_camera,
        ]
    }
    fn pair(&self) -> Result<Pair, E> {
        check_len(
            "control_data_keyboard",
            self.control_data_keyboard.len(),
            Self::KEYS,
        )?;
        check_len(
            "control_data_joystick",
            self.control_data_joystick.len(),
            Self::KEYS,
        )?;
        let mut cfg: Vec<(&str, Pair)> = Self::CFG_KEYS
            .into_iter()
            .zip(self.scalars())
            .map(|(k, v)| (k, p_u64(v)))
            .collect();
        cfg.push((
            "control_data_keyboard",
            p_arr(
                self.control_data_keyboard
                    .iter()
                    .map(|x| p_i32(*x))
                    .collect(),
            ),
        ));
        cfg.push((
            "control_data_joystick",
            p_arr(
                self.control_data_joystick
                    .iter()
                    .map(|x| p_str(x))
                    .collect(),
            ),
        ));
        Ok(p_cls(vec![("cfg", p_cls(cfg)), ("time", p_f32(self.time))]))
    }
    fn read(n: &D) -> Result<Self, E> {
        let cfg = get(n, "cfg")?;
        let mut s = [0u64; 15];
        for (slot, key) in s.iter_mut().zip(Self::CFG_KEYS) {
            *slot = r_u64(get(cfg, key)?)?;
        }
        let kb = items(get(cfg, "control_data_keyboard")?)?;
        let joy = items(get(cfg, "control_data_joystick")?)?;
        check_len("control_data_keyboard", kb.len(), Self::KEYS)?;
        check_len("control_data_joystick", joy.len(), Self::KEYS)?;
        Ok(Self {
            sound: s[0],
            sound_effect: s[1],
            control: s[2],
            cursor: s[3],
            atb: s[4],
            camera: s[5],
            move_: s[6],
            vibe: s[7],
            btl_speed: s[8],
            fld_msg: s[9],
            here_icon: s[10],
            win_type: s[11],
            target_win: s[12],
            control_data: s[13],
            skip_btl_camera: s[14],
            control_data_keyboard: kb.iter().map(r_i32).collect::<Result<_, _>>()?,
            control_data_joystick: joy.iter().map(r_str).collect::<Result<_, _>>()?,
            time: r_f32(get(n, "time")?)?,
        })
    }
}

/// `70000_World` (`JsonParser.cs:1554-1620`). Keys contain dots; they are literal key names.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldSection {
    pub rotation_max: f32,
    pub upper_counter: i16,
    pub upper_counter_speed: i32,
    pub upper_counter_force: bool,
    pub rotation: f32,
    pub rotation_rev: f32,
    pub hintmap: u32,
}

impl WorldSection {
    fn pair(&self) -> Pair {
        p_cls(vec![
            ("data.cameraState.rotationMax", p_f32(self.rotation_max)),
            ("data.cameraState.upperCounter", p_i16(self.upper_counter)),
            (
                "data.cameraState.upperCounterSpeed",
                p_i32(self.upper_counter_speed),
            ),
            (
                "data.cameraState.upperCounterForce",
                p_bool(self.upper_counter_force),
            ),
            ("data.cameraState.rotation", p_f32(self.rotation)),
            ("data.cameraState.rotationRev", p_f32(self.rotation_rev)),
            ("data.hintmap", p_u32(self.hintmap)),
        ])
    }
    fn read(n: &D) -> Result<Self, E> {
        Ok(Self {
            rotation_max: r_f32(get(n, "data.cameraState.rotationMax")?)?,
            upper_counter: r_i16(get(n, "data.cameraState.upperCounter")?)?,
            upper_counter_speed: r_i32(get(n, "data.cameraState.upperCounterSpeed")?)?,
            upper_counter_force: r_bool(get(n, "data.cameraState.upperCounterForce")?)?,
            rotation: r_f32(get(n, "data.cameraState.rotation")?)?,
            rotation_rev: r_f32(get(n, "data.cameraState.rotationRev")?)?,
            hintmap: r_u32(get(n, "data.hintmap")?)?,
        })
    }
}

/// `80000_Achievement` (`JsonParser.cs:1642-1728`). Unused list slots hold -1.
#[derive(Debug, Clone, PartialEq)]
pub struct AchievementSection {
    pub ate_check: Vec<i32>,
    pub evt_reserved: Vec<i32>,
    pub blk_mag_no: i32,
    pub wht_mag_no: i32,
    pub blu_mag_no: i32,
    pub summon_no: i32,
    pub enemy_no: i32,
    pub back_atk_no: i32,
    pub defence_no: i32,
    pub trance_no: i32,
    pub abilities: Vec<i32>,
    pub passive_abilities: Vec<i32>,
    pub synthesis_count: i32,
    pub auction_time: i32,
    pub stiltzkin_buy: i32,
    pub quadmist_win_list: Vec<i32>,
}

impl AchievementSection {
    pub const ATE_CHECK: usize = 100;
    pub const EVT_RESERVED: usize = 17;
    pub const ABILITIES: usize = 221;
    pub const PASSIVE_ABILITIES: usize = 63;
    pub const QUADMIST_WIN_LIST: usize = 300;
    fn blank() -> Self {
        Self {
            ate_check: vec![0; Self::ATE_CHECK],
            evt_reserved: vec![0; Self::EVT_RESERVED],
            blk_mag_no: 0,
            wht_mag_no: 0,
            blu_mag_no: 0,
            summon_no: 0,
            enemy_no: 0,
            back_atk_no: 0,
            defence_no: 0,
            trance_no: 0,
            abilities: vec![-1; Self::ABILITIES],
            passive_abilities: vec![-1; Self::PASSIVE_ABILITIES],
            synthesis_count: 0,
            auction_time: 0,
            stiltzkin_buy: 0,
            quadmist_win_list: vec![-1; Self::QUADMIST_WIN_LIST],
        }
    }
    fn pair(&self) -> Result<Pair, E> {
        Ok(p_cls(vec![
            (
                "AteCheckArray",
                p_i32s(&self.ate_check, Self::ATE_CHECK, "AteCheckArray")?,
            ),
            (
                "EvtReservedArray",
                p_i32s(&self.evt_reserved, Self::EVT_RESERVED, "EvtReservedArray")?,
            ),
            ("blkMag_no", p_i32(self.blk_mag_no)),
            ("whtMag_no", p_i32(self.wht_mag_no)),
            ("bluMag_no", p_i32(self.blu_mag_no)),
            ("summon_no", p_i32(self.summon_no)),
            ("enemy_no", p_i32(self.enemy_no)),
            ("backAtk_no", p_i32(self.back_atk_no)),
            ("defence_no", p_i32(self.defence_no)),
            ("trance_no", p_i32(self.trance_no)),
            (
                "abilities",
                p_i32s(&self.abilities, Self::ABILITIES, "abilities")?,
            ),
            (
                "passiveAbilities",
                p_i32s(
                    &self.passive_abilities,
                    Self::PASSIVE_ABILITIES,
                    "passiveAbilities",
                )?,
            ),
            ("synthesisCount", p_i32(self.synthesis_count)),
            ("AuctionTime", p_i32(self.auction_time)),
            ("StiltzkinBuy", p_i32(self.stiltzkin_buy)),
            (
                "QuadmistWinList",
                p_i32s(
                    &self.quadmist_win_list,
                    Self::QUADMIST_WIN_LIST,
                    "QuadmistWinList",
                )?,
            ),
        ]))
    }
    fn read(n: &D) -> Result<Self, E> {
        Ok(Self {
            ate_check: r_i32s(get(n, "AteCheckArray")?, Self::ATE_CHECK, "AteCheckArray")?,
            evt_reserved: r_i32s(
                get(n, "EvtReservedArray")?,
                Self::EVT_RESERVED,
                "EvtReservedArray",
            )?,
            blk_mag_no: r_i32(get(n, "blkMag_no")?)?,
            wht_mag_no: r_i32(get(n, "whtMag_no")?)?,
            blu_mag_no: r_i32(get(n, "bluMag_no")?)?,
            summon_no: r_i32(get(n, "summon_no")?)?,
            enemy_no: r_i32(get(n, "enemy_no")?)?,
            back_atk_no: r_i32(get(n, "backAtk_no")?)?,
            defence_no: r_i32(get(n, "defence_no")?)?,
            trance_no: r_i32(get(n, "trance_no")?)?,
            abilities: r_i32s(get(n, "abilities")?, Self::ABILITIES, "abilities")?,
            passive_abilities: r_i32s(
                get(n, "passiveAbilities")?,
                Self::PASSIVE_ABILITIES,
                "passiveAbilities",
            )?,
            synthesis_count: r_i32(get(n, "synthesisCount")?)?,
            auction_time: r_i32(get(n, "AuctionTime")?)?,
            stiltzkin_buy: r_i32(get(n, "StiltzkinBuy")?)?,
            quadmist_win_list: r_i32s(
                get(n, "QuadmistWinList")?,
                Self::QUADMIST_WIN_LIST,
                "QuadmistWinList",
            )?,
        })
    }
}

/// `98000_Achievement`: UInt32 `abnormal_status`, sixteen summon flags, 123 reserved Int32
/// (`JsonParser.cs:84-226`). Key digits are as written, including the six-digit `000010..000017`.
#[derive(Debug, Clone, PartialEq)]
pub struct Achievement98000 {
    pub abnormal_status: u32,
    /// Order: shiva, ifrit, ramuh, carbuncle_reflector, carbuncle_haste, carbuncle_protect,
    /// carbuncle_shell, fenrir_earth, fenrir_wind, atomos, phoenix, leviathan, odin, madeen,
    /// bahamut, arc.
    pub summons: [bool; 16],
    pub reserved: Vec<i32>,
}

impl Achievement98000 {
    pub const RESERVED: usize = 123;
    /// Written key of each summon flag, in `summons` order.
    pub const SUMMON_KEYS: [&'static str; 16] = [
        "00002_summon_shiva",
        "00003_summon_ifrit",
        "00004_summon_ramuh",
        "00005_summon_carbuncle_reflector",
        "00006_summon_carbuncle_haste",
        "00007_summon_carbuncle_protect",
        "00008_summon_carbuncle_shell",
        "00009_summon_fenrir_earth",
        "000010_summon_fenrir_wind",
        "000011_summon_atomos",
        "000012_summon_phoenix",
        "000013_summon_leviathan",
        "000014_summon_odin",
        "000015_summon_madeen",
        "000016_summon_bahamut",
        "000017_summon_arc",
    ];
    fn blank() -> Self {
        Self {
            abnormal_status: 0,
            summons: [false; 16],
            reserved: vec![0; Self::RESERVED],
        }
    }
    fn pair(&self) -> Result<Pair, E> {
        let mut fields = vec![("00001_abnormal_status", p_u32(self.abnormal_status))];
        for (key, flag) in Self::SUMMON_KEYS.into_iter().zip(self.summons) {
            fields.push((key, p_bool(flag)));
        }
        fields.push((
            "99999_ReservedBuffer",
            p_i32s(&self.reserved, Self::RESERVED, "98000 ReservedBuffer")?,
        ));
        Ok(p_cls(fields))
    }
    fn read(n: &D) -> Result<Self, E> {
        let mut summons = [false; 16];
        for (slot, key) in summons.iter_mut().zip(Self::SUMMON_KEYS) {
            *slot = r_bool(get(n, key)?)?;
        }
        Ok(Self {
            abnormal_status: r_u32(get(n, "00001_abnormal_status")?)?,
            summons,
            reserved: r_i32s(
                get(n, "99999_ReservedBuffer")?,
                Self::RESERVED,
                "98000 ReservedBuffer",
            )?,
        })
    }
}

/// `94000_Common`: nine UInt32 `player_bonus.cap` values plus 119 reserved Int32
/// (`JsonParser.cs:51-70`).
#[derive(Debug, Clone, PartialEq)]
pub struct Common94000 {
    pub player_bonus: [u32; 9],
    pub reserved: Vec<i32>,
}

/// `95000_Setting`: Double `00001_time` plus 126 reserved Int32 (`JsonParser.cs:71-80`).
#[derive(Debug, Clone, PartialEq)]
pub struct Setting95000 {
    pub time: f64,
    pub reserved: Vec<i32>,
}

/// The whole base `Data` tree (`JsonParser.cs:33-227`).
///
/// `reserved_91000` .. `reserved_93000`, `sound_96000`, `world_97000` are 128 Int32 each and
/// `other_99000` is 384 Int32; they carry no named fields in the C# and are kept as raw values.
#[derive(Debug, Clone, PartialEq)]
pub struct Ff9SaveTree {
    pub state: StateSection,
    pub event: EventSection,
    pub mini_game: MiniGameSection,
    pub common: CommonSection,
    pub setting: SettingSection,
    /// `60000_Sound.auto_save_bgm_id` (Int32; `JsonParser.cs:1530-1546`).
    pub auto_save_bgm_id: i32,
    pub world: WorldSection,
    pub achievement: AchievementSection,
    pub reserved_91000: Vec<i32>,
    pub reserved_92000: Vec<i32>,
    pub reserved_93000: Vec<i32>,
    pub common_94000: Common94000,
    pub setting_95000: Setting95000,
    pub sound_96000: Vec<i32>,
    pub world_97000: Vec<i32>,
    pub achievement_98000: Achievement98000,
    pub other_99000: Vec<i32>,
}

impl Ff9SaveTree {
    /// Exact payload length after the `SAVE` magic, computed from the schema (17,966 bytes);
    /// it fits the 18,432-byte `DataReservedSize` (`SharedDataBytesStorage.cs:1435`).
    pub const PAYLOAD_BYTES: usize = 17_966;

    /// Valid all-zero tree (empty card/item/ability slots use the writer's empty markers).
    pub fn blank() -> Self {
        Self {
            state: StateSection::default(),
            event: EventSection::default(),
            mini_game: MiniGameSection::blank(),
            common: CommonSection::blank(),
            setting: SettingSection::blank(),
            auto_save_bgm_id: -1,
            world: WorldSection::default(),
            achievement: AchievementSection::blank(),
            reserved_91000: vec![0; 128],
            reserved_92000: vec![0; 128],
            reserved_93000: vec![0; 128],
            common_94000: Common94000 {
                player_bonus: [0; 9],
                reserved: vec![0; 119],
            },
            setting_95000: Setting95000 {
                time: 0.0,
                reserved: vec![0; 126],
            },
            sound_96000: vec![0; 128],
            world_97000: vec![0; 128],
            achievement_98000: Achievement98000::blank(),
            other_99000: vec![0; 384],
        }
    }

    fn pair(&self) -> Result<Pair, E> {
        let c94 = &self.common_94000;
        let s95 = &self.setting_95000;
        let data = p_cls(vec![
            ("10000_State", self.state.pair()),
            ("20000_Event", self.event.pair()),
            ("30000_MiniGame", self.mini_game.pair()?),
            ("40000_Common", self.common.pair()?),
            ("50000_Setting", self.setting.pair()?),
            (
                "60000_Sound",
                p_cls(vec![("auto_save_bgm_id", p_i32(self.auto_save_bgm_id))]),
            ),
            ("70000_World", self.world.pair()),
            ("80000_Achievement", self.achievement.pair()?),
            (
                "91000_State",
                p_i32s(&self.reserved_91000, 128, "91000_State")?,
            ),
            (
                "92000_Event",
                p_i32s(&self.reserved_92000, 128, "92000_Event")?,
            ),
            (
                "93000_MiniGame",
                p_i32s(&self.reserved_93000, 128, "93000_MiniGame")?,
            ),
            (
                "94000_Common",
                p_cls(vec![
                    (
                        "00001_player_bonus",
                        p_arr(c94.player_bonus.iter().map(|x| p_u32(*x)).collect()),
                    ),
                    (
                        "99999_ReservedBuffer",
                        p_i32s(&c94.reserved, 119, "94000 ReservedBuffer")?,
                    ),
                ]),
            ),
            (
                "95000_Setting",
                p_cls(vec![
                    ("00001_time", p_f64(s95.time)),
                    (
                        "99999_ReservedBuffer",
                        p_i32s(&s95.reserved, 126, "95000 ReservedBuffer")?,
                    ),
                ]),
            ),
            (
                "96000_Sound",
                p_i32s(&self.sound_96000, 128, "96000_Sound")?,
            ),
            (
                "97000_World",
                p_i32s(&self.world_97000, 128, "97000_World")?,
            ),
            ("98000_Achievement", self.achievement_98000.pair()?),
            (
                "99000_Other",
                p_i32s(&self.other_99000, 384, "99000_Other")?,
            ),
        ]);
        Ok(p_cls(vec![("Data", data)]))
    }

    /// Flattens the tree to the payload bytes that follow the `SAVE` magic.
    pub fn encode_payload(&self, order: KeyOrder) -> Result<Vec<u8>, E> {
        let (schema, data) = self.pair()?;
        let mut types = Vec::new();
        flatten_types(&schema, order, &mut types);
        let mut values = Vec::with_capacity(types.len());
        flatten_values(&data, &schema, order, &mut values)?;
        BaseSavePayload { values }.encode(&types)
    }

    /// Inverse of [`Self::encode_payload`]; the payload must match the schema size exactly.
    pub fn decode_payload(payload: &[u8], order: KeyOrder) -> Result<Self, E> {
        let (schema, _) = Self::blank().pair()?;
        let mut types = Vec::new();
        flatten_types(&schema, order, &mut types);
        let decoded = BaseSavePayload::decode(payload, &types)?;
        let data = unflatten(&schema, order, &mut decoded.values.into_iter())?;
        let root = get(&data, "Data")?;
        Ok(Self {
            state: StateSection::read(get(root, "10000_State")?)?,
            event: EventSection::read(get(root, "20000_Event")?)?,
            mini_game: MiniGameSection::read(get(root, "30000_MiniGame")?)?,
            common: CommonSection::read(get(root, "40000_Common")?)?,
            setting: SettingSection::read(get(root, "50000_Setting")?)?,
            auto_save_bgm_id: r_i32(get(get(root, "60000_Sound")?, "auto_save_bgm_id")?)?,
            world: WorldSection::read(get(root, "70000_World")?)?,
            achievement: AchievementSection::read(get(root, "80000_Achievement")?)?,
            reserved_91000: r_i32s(get(root, "91000_State")?, 128, "91000_State")?,
            reserved_92000: r_i32s(get(root, "92000_Event")?, 128, "92000_Event")?,
            reserved_93000: r_i32s(get(root, "93000_MiniGame")?, 128, "93000_MiniGame")?,
            common_94000: {
                let n = get(root, "94000_Common")?;
                let bonus = items(get(n, "00001_player_bonus")?)?;
                check_len("00001_player_bonus", bonus.len(), 9)?;
                let mut player_bonus = [0u32; 9];
                for (slot, node) in player_bonus.iter_mut().zip(bonus) {
                    *slot = r_u32(node)?;
                }
                Common94000 {
                    player_bonus,
                    reserved: r_i32s(get(n, "99999_ReservedBuffer")?, 119, "94000 ReservedBuffer")?,
                }
            },
            setting_95000: {
                let n = get(root, "95000_Setting")?;
                Setting95000 {
                    time: r_f64(get(n, "00001_time")?)?,
                    reserved: r_i32s(get(n, "99999_ReservedBuffer")?, 126, "95000 ReservedBuffer")?,
                }
            },
            sound_96000: r_i32s(get(root, "96000_Sound")?, 128, "96000_Sound")?,
            world_97000: r_i32s(get(root, "97000_World")?, 128, "97000_World")?,
            achievement_98000: Achievement98000::read(get(root, "98000_Achievement")?)?,
            other_99000: r_i32s(get(root, "99000_Other")?, 384, "99000_Other")?,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// MemoriaExtraData sidecar (SimpleJSON binary tree).
// ---------------------------------------------------------------------------------------------

type N = SimpleJsonNode;

/// Leaf written the way `JSONData.Serialize` picks a tag for an integer's decimal text
/// (`JSONData.cs:55-88`): Int32 when it round-trips, else Double when it does, else string.
/// The `|v| < 1e15` bound for Double is inferred from .NET `Double.ToString()` and is not
/// verified against a real sidecar.
fn leaf_int(value: i64) -> N {
    if let Ok(v) = i32::try_from(value) {
        N::Int(v)
    } else if value.unsigned_abs() < 1_000_000_000_000_000 {
        N::Double(value as f64)
    } else {
        N::String(value.to_string())
    }
}

fn leaf_u64(value: u64) -> N {
    i64::try_from(value).map_or_else(|_| N::String(value.to_string()), leaf_int)
}

/// `Boolean.ToString()` text; `AsBool` never round-trips it, so it is stored as a string leaf.
fn leaf_bool(value: bool) -> N {
    N::String(if value { "True" } else { "False" }.to_owned())
}

fn node_i64(node: &N) -> Result<i64, E> {
    match node {
        N::Int(v) => Ok(i64::from(*v)),
        N::String(s) => s
            .trim()
            .parse()
            .map_err(|_| err(format!("`{s}` is not an integer"))),
        N::Double(v) if v.fract() == 0.0 && v.abs() < 9.0e15 => Ok(*v as i64),
        N::Float(v) if v.fract() == 0.0 && v.abs() < 1.0e7 => Ok(*v as i64),
        other => Err(err(format!("expected an integer leaf, found {other:?}"))),
    }
}

fn node_bool(node: &N) -> Result<bool, E> {
    match node {
        N::Bool(v) => Ok(*v),
        N::String(s) if s == "True" || s.eq_ignore_ascii_case("true") => Ok(true),
        N::String(s) if s == "False" || s.eq_ignore_ascii_case("false") => Ok(false),
        other => Err(err(format!("expected a boolean leaf, found {other:?}"))),
    }
}

fn node_f64(node: &N) -> Result<f64, E> {
    match node {
        N::Double(v) => Ok(*v),
        N::Float(v) => Ok(f64::from(*v)),
        N::Int(v) => Ok(f64::from(*v)),
        N::String(s) => s
            .trim()
            .parse()
            .map_err(|_| err(format!("`{s}` is not a number"))),
        other => Err(err(format!("expected a numeric leaf, found {other:?}"))),
    }
}

fn node_str(node: &N) -> Result<String, E> {
    match node {
        N::String(s) => Ok(s.clone()),
        other => Err(err(format!("expected a string leaf, found {other:?}"))),
    }
}

/// Strict key reader over a class node: every key must be consumed, so unknown keys are errors.
struct Fields<'a> {
    fields: &'a [(String, N)],
    seen: std::cell::Cell<usize>,
    path: String,
}

impl<'a> Fields<'a> {
    fn new(node: &'a N, path: &str) -> Result<Self, E> {
        match node {
            N::Class(fields) => Ok(Self {
                fields,
                seen: std::cell::Cell::new(0),
                path: path.to_owned(),
            }),
            _ => Err(err(format!("{path} is not a class"))),
        }
    }
    fn has(&self, key: &str) -> bool {
        self.fields.iter().any(|(k, _)| k == key)
    }
    fn node(&self, key: &str) -> Result<&'a N, E> {
        let found = self
            .fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .ok_or_else(|| err(format!("{}: missing key `{key}`", self.path)))?;
        self.seen.set(self.seen.get() + 1);
        Ok(found)
    }
    fn int(&self, key: &str) -> Result<i64, E> {
        node_i64(self.node(key)?).map_err(|e| err(format!("{}.{key}: {}", self.path, e.context)))
    }
    fn array(&self, key: &str) -> Result<&'a [N], E> {
        match self.node(key)? {
            N::Array(v) => Ok(v),
            _ => Err(err(format!("{}.{key} is not an array", self.path))),
        }
    }
    fn class(&self, key: &str) -> Result<Fields<'a>, E> {
        Fields::new(self.node(key)?, &format!("{}.{key}", self.path))
    }
    fn finish(self) -> Result<(), E> {
        if self.seen.get() == self.fields.len() {
            Ok(())
        } else {
            Err(err(format!("{}: unexpected extra keys", self.path)))
        }
    }
}

fn cls(items: Vec<(&str, N)>) -> N {
    N::Class(items.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn id_count_array(entries: &[(i64, i64)], a: &str, b: &str) -> N {
    N::Array(
        entries
            .iter()
            .map(|(x, y)| cls(vec![(a, leaf_int(*x)), (b, leaf_int(*y))]))
            .collect(),
    )
}

fn read_pairs(array: &[N], a: &str, b: &str, path: &str) -> Result<Vec<(i64, i64)>, E> {
    array
        .iter()
        .map(|n| {
            let f = Fields::new(n, path)?;
            let out = (f.int(a)?, f.int(b)?);
            f.finish()?;
            Ok(out)
        })
        .collect()
}

/// Sidecar `20000_Event` (`JsonParser.cs:575-633`, new format).
#[derive(Debug, Clone, PartialEq)]
pub struct MemoriaEvent {
    pub step_count: i32,
    pub event_global: EventGlobal,
    /// `(BattleAbilityId, count)` in dictionary order.
    pub ability_usage: Vec<(i32, i32)>,
    /// `(id, entries)`.
    pub script_vector: Vec<(i32, Vec<i32>)>,
    /// `(id, [(key, value)])`.
    pub script_dictionary: Vec<(i32, Vec<(i32, i32)>)>,
}

/// Sidecar `30000_MiniGame`: only the cards that exist, no 100-slot padding.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoriaMiniGame {
    pub s_win: i16,
    pub s_lose: i16,
    pub s_draw: i16,
    pub cards: Vec<CardSlot>,
}

/// Player block of the new-format `40000_Common` (`JsonParser.cs:787-913`). Numbers are decimal
/// text in the tree, so they are kept as `i64` (`status` as `u64`).
#[derive(Debug, Clone, PartialEq)]
pub struct MemoriaPlayer {
    pub name: String,
    pub category: i64,
    pub level: i64,
    pub exp: i64,
    /// hp, mp, at, at_coef, capa.
    pub cur: [i64; 5],
    pub max: [i64; 5],
    pub trance: i64,
    pub web_bone: i64,
    /// dex, str, mgc, wpr.
    pub elem: [i64; 4],
    /// p_def, p_ev, m_def, m_ev.
    pub defence: [i64; 4],
    /// max_hp, max_mp, dex, str, mgc, wpr.
    pub basis: [i64; 6],
    /// slot_no, serial_no, row, win_pose, party, menu_type.
    pub info: [i64; 6],
    pub sub_replaced: bool,
    pub status: u64,
    pub equip: [i64; 5],
    /// dex, str, mgc, wpr, cap.
    pub bonus: [i64; 5],
    pub abilities: MemoriaAbilities,
    pub max_hp_limit: i64,
    pub max_mp_limit: i64,
    pub max_damage_limit: i64,
    pub max_mp_damage_limit: i64,
    pub basis_capa: i64,
    pub basis_max_capa: i64,
}

/// `pa`/`sa` (plain) or `pa_extended`/`sa_extended` (`JsonParser.cs:873-902`).
#[derive(Debug, Clone, PartialEq)]
pub enum MemoriaAbilities {
    Plain { pa: Vec<i64>, sa: Vec<i64> },
    Extended { pa: Vec<(i64, i64)>, sa: Vec<i64> },
}

/// New-format `40000_Common` of the sidecar (`JsonParser.cs:765-1017`).
#[derive(Debug, Clone, PartialEq)]
pub struct MemoriaCommon {
    pub players: Vec<MemoriaPlayer>,
    pub slot: Vec<i64>,
    pub escape_no: i64,
    pub summon_flag: i64,
    pub gil: i64,
    pub frog_no: i64,
    pub steal_no: i64,
    pub kills_per_category: Vec<(i64, i64)>,
    pub kills_per_model: Vec<(i64, i64)>,
    pub items: Vec<(i64, i64)>,
    /// `(id, obtained, used)`.
    pub rare_items_ex: Vec<(i64, bool, bool)>,
    pub battle_no: i64,
}

/// Typed `MemoriaExtraData` sidecar (`JsonParser.cs:229-238`). Sections are written in the
/// order `95000_Setting`, `20000_Event`, `40000_Common`, `30000_MiniGame`.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoriaExtra {
    /// `95000_Setting.00001_time` leaf (kept as read so the tag byte survives).
    pub setting_time: N,
    pub event: MemoriaEvent,
    pub common: MemoriaCommon,
    pub mini_game: MemoriaMiniGame,
}

fn int_array(values: &[i64]) -> N {
    N::Array(values.iter().map(|v| leaf_int(*v)).collect())
}

fn read_ints(array: &[N], path: &str) -> Result<Vec<i64>, E> {
    array
        .iter()
        .map(|n| node_i64(n).map_err(|e| err(format!("{path}: {}", e.context))))
        .collect()
}

fn fixed<const L: usize>(values: Vec<i64>, path: &str) -> Result<[i64; L], E> {
    values
        .try_into()
        .map_err(|v: Vec<i64>| err(format!("{path} has {} elements, expected {L}", v.len())))
}

fn keyed<const L: usize>(f: &Fields<'_>, keys: [&str; L]) -> Result<[i64; L], E> {
    let mut out = [0i64; L];
    for (slot, key) in out.iter_mut().zip(keys) {
        *slot = f.int(key)?;
    }
    Ok(out)
}

fn keyed_node<const L: usize>(keys: [&str; L], values: [i64; L]) -> N {
    cls(keys
        .into_iter()
        .zip(values)
        .map(|(k, v)| (k, leaf_int(v)))
        .collect())
}

const CUR_KEYS: [&str; 5] = ["hp", "mp", "at", "at_coef", "capa"];
const STAT_KEYS: [&str; 4] = ["dex", "str", "mgc", "wpr"];
const DEF_KEYS: [&str; 4] = ["p_def", "p_ev", "m_def", "m_ev"];
const BASIS_KEYS: [&str; 6] = ["max_hp", "max_mp", "dex", "str", "mgc", "wpr"];
const INFO_KEYS: [&str; 6] = [
    "slot_no",
    "serial_no",
    "row",
    "win_pose",
    "party",
    "menu_type",
];
const BONUS_KEYS: [&str; 5] = ["dex", "str", "mgc", "wpr", "cap"];

impl MemoriaPlayer {
    fn to_node(&self) -> N {
        let mut info: Vec<(&str, N)> = INFO_KEYS
            .into_iter()
            .zip(self.info)
            .map(|(k, v)| (k, leaf_int(v)))
            .collect();
        info.push(("sub_replaced", leaf_bool(self.sub_replaced)));
        let mut fields = vec![
            ("name", N::String(self.name.clone())),
            ("category", leaf_int(self.category)),
            ("level", leaf_int(self.level)),
            ("exp", leaf_int(self.exp)),
            ("cur", keyed_node(CUR_KEYS, self.cur)),
            ("max", keyed_node(CUR_KEYS, self.max)),
            ("trance", leaf_int(self.trance)),
            ("web_bone", leaf_int(self.web_bone)),
            ("elem", keyed_node(STAT_KEYS, self.elem)),
            ("defence", keyed_node(DEF_KEYS, self.defence)),
            ("basis", keyed_node(BASIS_KEYS, self.basis)),
            ("info", cls(info)),
            ("status", leaf_u64(self.status)),
            ("equip", int_array(&self.equip)),
            ("bonus", keyed_node(BONUS_KEYS, self.bonus)),
        ];
        match &self.abilities {
            MemoriaAbilities::Plain { pa, sa } => {
                fields.push(("pa", int_array(pa)));
                fields.push(("sa", int_array(sa)));
            }
            MemoriaAbilities::Extended { pa, sa } => {
                fields.push(("pa_extended", id_count_array(pa, "id", "cur")));
                fields.push(("sa_extended", int_array(sa)));
            }
        }
        fields.extend([
            ("maxHpLimit", leaf_int(self.max_hp_limit)),
            ("maxMpLimit", leaf_int(self.max_mp_limit)),
            ("maxDamageLimit", leaf_int(self.max_damage_limit)),
            ("maxMpDamageLimit", leaf_int(self.max_mp_damage_limit)),
            ("basis_capa", leaf_int(self.basis_capa)),
            ("basis_max_capa", leaf_int(self.basis_max_capa)),
        ]);
        cls(fields)
    }

    fn from_node(node: &N, path: &str) -> Result<Self, E> {
        let f = Fields::new(node, path)?;
        let sub = |key: &str| f.class(key);
        let cur = sub("cur")?;
        let cur_v = keyed(&cur, CUR_KEYS)?;
        cur.finish()?;
        let max = sub("max")?;
        let max_v = keyed(&max, CUR_KEYS)?;
        max.finish()?;
        let elem = sub("elem")?;
        let elem_v = keyed(&elem, STAT_KEYS)?;
        elem.finish()?;
        let defence = sub("defence")?;
        let defence_v = keyed(&defence, DEF_KEYS)?;
        defence.finish()?;
        let basis = sub("basis")?;
        let basis_v = keyed(&basis, BASIS_KEYS)?;
        basis.finish()?;
        let info = sub("info")?;
        let info_v = keyed(&info, INFO_KEYS)?;
        let sub_replaced = node_bool(info.node("sub_replaced")?)?;
        info.finish()?;
        let bonus = sub("bonus")?;
        let bonus_v = keyed(&bonus, BONUS_KEYS)?;
        bonus.finish()?;
        let abilities = if f.has("pa") {
            MemoriaAbilities::Plain {
                pa: read_ints(f.array("pa")?, "pa")?,
                sa: read_ints(f.array("sa")?, "sa")?,
            }
        } else {
            MemoriaAbilities::Extended {
                pa: read_pairs(f.array("pa_extended")?, "id", "cur", "pa_extended")?,
                sa: read_ints(f.array("sa_extended")?, "sa_extended")?,
            }
        };
        let status = {
            let n = f.node("status")?;
            match n {
                N::String(s) => s.parse::<u64>().map_err(|_| err("status is not a u64"))?,
                other => u64::try_from(node_i64(other)?).map_err(|_| err("status is negative"))?,
            }
        };
        let out = Self {
            name: node_str(f.node("name")?)?,
            category: f.int("category")?,
            level: f.int("level")?,
            exp: f.int("exp")?,
            cur: cur_v,
            max: max_v,
            trance: f.int("trance")?,
            web_bone: f.int("web_bone")?,
            elem: elem_v,
            defence: defence_v,
            basis: basis_v,
            info: info_v,
            sub_replaced,
            status,
            equip: fixed(read_ints(f.array("equip")?, "equip")?, "equip")?,
            bonus: bonus_v,
            abilities,
            max_hp_limit: f.int("maxHpLimit")?,
            max_mp_limit: f.int("maxMpLimit")?,
            max_damage_limit: f.int("maxDamageLimit")?,
            max_mp_damage_limit: f.int("maxMpDamageLimit")?,
            basis_capa: f.int("basis_capa")?,
            basis_max_capa: f.int("basis_max_capa")?,
        };
        f.finish()?;
        Ok(out)
    }
}

impl MemoriaExtra {
    /// Builds the sidecar `Class` node in the writer's key order.
    pub fn to_node(&self) -> N {
        let ev = &self.event;
        let event = cls(vec![
            ("gStepCount", leaf_int(i64::from(ev.step_count))),
            ("gEventGlobal", N::String(ev.event_global.to_base64())),
            (
                "gAbilityUsage",
                id_count_array(
                    &ev.ability_usage
                        .iter()
                        .map(|(a, b)| (i64::from(*a), i64::from(*b)))
                        .collect::<Vec<_>>(),
                    "id",
                    "count",
                ),
            ),
            (
                "gScriptVector",
                N::Array(
                    ev.script_vector
                        .iter()
                        .map(|(id, entries)| {
                            cls(vec![
                                ("id", leaf_int(i64::from(*id))),
                                (
                                    "entries",
                                    int_array(
                                        &entries.iter().map(|x| i64::from(*x)).collect::<Vec<_>>(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "gScriptDictionary",
                N::Array(
                    ev.script_dictionary
                        .iter()
                        .map(|(id, entries)| {
                            cls(vec![
                                ("id", leaf_int(i64::from(*id))),
                                (
                                    "entries",
                                    id_count_array(
                                        &entries
                                            .iter()
                                            .map(|(a, b)| (i64::from(*a), i64::from(*b)))
                                            .collect::<Vec<_>>(),
                                        "id",
                                        "value",
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]);
        let c = &self.common;
        let common = cls(vec![
            (
                "players",
                N::Array(c.players.iter().map(MemoriaPlayer::to_node).collect()),
            ),
            ("slot", int_array(&c.slot)),
            ("escape_no", leaf_int(c.escape_no)),
            ("summon_flag", leaf_int(c.summon_flag)),
            ("gil", leaf_int(c.gil)),
            ("frog_no", leaf_int(c.frog_no)),
            ("steal_no", leaf_int(c.steal_no)),
            (
                "kills_per_category",
                id_count_array(&c.kills_per_category, "id", "count"),
            ),
            (
                "kills_per_model",
                id_count_array(&c.kills_per_model, "id", "count"),
            ),
            ("items", id_count_array(&c.items, "id", "count")),
            (
                "rareItemsEx",
                N::Array(
                    c.rare_items_ex
                        .iter()
                        .map(|(id, obtained, used)| {
                            cls(vec![
                                ("id", leaf_int(*id)),
                                ("obtained", leaf_bool(*obtained)),
                                ("used", leaf_bool(*used)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("battle_no", leaf_int(c.battle_no)),
        ]);
        let m = &self.mini_game;
        let mini = cls(vec![
            ("sWin", leaf_int(i64::from(m.s_win))),
            ("sLose", leaf_int(i64::from(m.s_lose))),
            ("sDraw", leaf_int(i64::from(m.s_draw))),
            (
                "MiniGameCard",
                N::Array(
                    m.cards
                        .iter()
                        .map(|c| {
                            cls(vec![
                                ("id", leaf_int(i64::from(c.id))),
                                ("side", leaf_int(i64::from(c.side))),
                                ("atk", leaf_int(i64::from(c.atk))),
                                ("type", leaf_int(i64::from(c.card_type))),
                                ("pdef", leaf_int(i64::from(c.pdef))),
                                ("mdef", leaf_int(i64::from(c.mdef))),
                                ("cpoint", leaf_int(i64::from(c.cpoint))),
                                ("arrow", leaf_int(i64::from(c.arrow))),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]);
        cls(vec![
            (
                "95000_Setting",
                cls(vec![("00001_time", self.setting_time.clone())]),
            ),
            ("20000_Event", event),
            ("40000_Common", common),
            ("30000_MiniGame", mini),
        ])
    }

    /// Serializes with the existing SimpleJSON binary writer.
    pub fn encode(&self) -> Result<Vec<u8>, E> {
        super::SimpleJsonDocument::encode(&self.to_node())
    }

    /// Reads a sidecar blob; every section key the writer emits must be present and no extra
    /// keys are accepted (the in-memory `MetaData*` keys are never stored in the file).
    pub fn decode(bytes: &[u8]) -> Result<Self, E> {
        let document = super::SimpleJsonDocument::decode(bytes)?;
        if document.trailing_bytes != 0 {
            return Err(err("sidecar has trailing bytes"));
        }
        Self::from_node(&document.root)
    }

    pub fn from_node(root: &N) -> Result<Self, E> {
        let top = Fields::new(root, "MemoriaExtraData")?;
        let setting = top.class("95000_Setting")?;
        let setting_time = setting.node("00001_time")?.clone();
        setting.finish()?;

        let ev = top.class("20000_Event")?;
        let step_count =
            i32::try_from(ev.int("gStepCount")?).map_err(|_| err("gStepCount range"))?;
        let event_global = EventGlobal::from_base64(&node_str(ev.node("gEventGlobal")?)?)?;
        let to_i32 = |v: i64| i32::try_from(v).map_err(|_| err("Int32 out of range"));
        let ability_usage = read_pairs(ev.array("gAbilityUsage")?, "id", "count", "gAbilityUsage")?
            .into_iter()
            .map(|(a, b)| Ok((to_i32(a)?, to_i32(b)?)))
            .collect::<Result<_, E>>()?;
        let script_vector = ev
            .array("gScriptVector")?
            .iter()
            .map(|n| {
                let f = Fields::new(n, "gScriptVector")?;
                let id = to_i32(f.int("id")?)?;
                let entries = read_ints(f.array("entries")?, "entries")?
                    .into_iter()
                    .map(to_i32)
                    .collect::<Result<_, _>>()?;
                f.finish()?;
                Ok((id, entries))
            })
            .collect::<Result<_, E>>()?;
        let script_dictionary = ev
            .array("gScriptDictionary")?
            .iter()
            .map(|n| {
                let f = Fields::new(n, "gScriptDictionary")?;
                let id = to_i32(f.int("id")?)?;
                let entries = read_pairs(f.array("entries")?, "id", "value", "entries")?
                    .into_iter()
                    .map(|(a, b)| Ok((to_i32(a)?, to_i32(b)?)))
                    .collect::<Result<_, E>>()?;
                f.finish()?;
                Ok((id, entries))
            })
            .collect::<Result<_, E>>()?;
        let event = MemoriaEvent {
            step_count,
            event_global,
            ability_usage,
            script_vector,
            script_dictionary,
        };
        ev.finish()?;

        let c = top.class("40000_Common")?;
        let players = c
            .array("players")?
            .iter()
            .map(|n| MemoriaPlayer::from_node(n, "players"))
            .collect::<Result<_, _>>()?;
        let rare_items_ex = c
            .array("rareItemsEx")?
            .iter()
            .map(|n| {
                let f = Fields::new(n, "rareItemsEx")?;
                let out = (
                    f.int("id")?,
                    node_bool(f.node("obtained")?)?,
                    node_bool(f.node("used")?)?,
                );
                f.finish()?;
                Ok(out)
            })
            .collect::<Result<_, E>>()?;
        let common = MemoriaCommon {
            players,
            slot: read_ints(c.array("slot")?, "slot")?,
            escape_no: c.int("escape_no")?,
            summon_flag: c.int("summon_flag")?,
            gil: c.int("gil")?,
            frog_no: c.int("frog_no")?,
            steal_no: c.int("steal_no")?,
            kills_per_category: read_pairs(
                c.array("kills_per_category")?,
                "id",
                "count",
                "kills_per_category",
            )?,
            kills_per_model: read_pairs(
                c.array("kills_per_model")?,
                "id",
                "count",
                "kills_per_model",
            )?,
            items: read_pairs(c.array("items")?, "id", "count", "items")?,
            rare_items_ex,
            battle_no: c.int("battle_no")?,
        };
        c.finish()?;

        let m = top.class("30000_MiniGame")?;
        let byte = |v: i64| u8::try_from(v).map_err(|_| err("card byte out of range"));
        let cards = m
            .array("MiniGameCard")?
            .iter()
            .map(|n| {
                let f = Fields::new(n, "MiniGameCard")?;
                let card = CardSlot {
                    id: byte(f.int("id")?)?,
                    side: byte(f.int("side")?)?,
                    atk: byte(f.int("atk")?)?,
                    card_type: to_i32(f.int("type")?)?,
                    pdef: byte(f.int("pdef")?)?,
                    mdef: byte(f.int("mdef")?)?,
                    cpoint: byte(f.int("cpoint")?)?,
                    arrow: byte(f.int("arrow")?)?,
                };
                f.finish()?;
                Ok(card)
            })
            .collect::<Result<_, E>>()?;
        let short = |v: i64| i16::try_from(v).map_err(|_| err("Int16 out of range"));
        let mini_game = MemoriaMiniGame {
            s_win: short(m.int("sWin")?)?,
            s_lose: short(m.int("sLose")?)?,
            s_draw: short(m.int("sDraw")?)?,
            cards,
        };
        m.finish()?;
        top.finish()?;
        Ok(Self {
            setting_time,
            event,
            common,
            mini_game,
        })
    }

    /// Numeric value of the stored `time` leaf.
    pub fn setting_time_seconds(&self) -> Result<f64, E> {
        node_f64(&self.setting_time)
    }

    /// A `time` leaf as `Double` (the C# type); the writer may choose Float when the text
    /// round-trips as one, which is not reproduced here.
    pub fn time_leaf(seconds: f64) -> SimpleJsonNode {
        N::Double(seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert!(base64_decode("Zm9vY").is_err());
        assert!(base64_decode("Zg=A").is_err());
    }

    #[test]
    fn blank_payload_has_the_schema_size() {
        let bytes = Ff9SaveTree::blank()
            .encode_payload(KeyOrder::Ordinal)
            .unwrap();
        assert_eq!(bytes.len(), Ff9SaveTree::PAYLOAD_BYTES);
        assert!(bytes.len() + 4 <= super::super::BASE_SAVE_BLOCK_BYTES);
    }

    #[test]
    fn culture_order_differs_only_for_mixed_case_keys() {
        let a = Ff9SaveTree::blank()
            .encode_payload(KeyOrder::Ordinal)
            .unwrap();
        let b = Ff9SaveTree::blank()
            .encode_payload(KeyOrder::CultureApprox)
            .unwrap();
        assert_eq!(a.len(), b.len());
        assert!(
            KeyOrder::CultureApprox
                .compare("abilities", "AteCheckArray")
                .is_lt()
        );
        assert!(
            KeyOrder::Ordinal
                .compare("abilities", "AteCheckArray")
                .is_gt()
        );
    }
}
