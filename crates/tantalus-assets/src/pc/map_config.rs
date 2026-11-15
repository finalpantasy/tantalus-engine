//! Field `mapconfigdata` reader.
//!
//! Source: Memoria `MapConfiguration.DMSParseMapConfFile`, `DMSMapLight.Read`,
//! `DMSMapChar.Read`, `DMSMapConf`. Header: u16 attr, u16 version, u16 bg_no,
//! u8 light_count, light_use, char_count, char_use, evt_count, evt_use
//! (12 bytes). Then `light_count` 12-byte lights and `char_count` 8-byte
//! chars. Memoria's size helper (`DMSGetMapConfSize`) also counts
//! `2 * evt_count` bytes of event data, but the parser never reads them, so
//! anything after the chars is surfaced as raw `trailing` (meaning unknown).

use crate::install::ImportError;
use serde::{Deserialize, Serialize};

pub const MAP_CONFIG_HEADER_SIZE: usize = 12;
pub const MAP_CONFIG_LIGHT_SIZE: usize = 12;
pub const MAP_CONFIG_CHAR_SIZE: usize = 8;

/// `DMSMapLight`. Colors and shadow intensity are stored already shifted
/// left by 3, as Memoria does (`(sbyte) << 3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapLight {
    pub kind: i8,
    pub no: i8,
    pub shadow_intensity: i16,
    pub shadow_radius: i8,
    pub color: [i16; 4],
    pub floor: [i8; 4],
}

/// `DMSMapChar` (colors and shadow intensity shifted left by 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapChar {
    pub geo_no: u16,
    pub shadow_intensity: i16,
    pub shadow_radius: i8,
    pub color: [i16; 3],
    pub shadow_z: i8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapConfig {
    pub attr: u16,
    pub version: u16,
    pub bg_no: u16,
    pub light_count: u8,
    pub light_use: u8,
    pub char_count: u8,
    pub char_use: u8,
    pub evt_count: u8,
    pub evt_use: u8,
    pub lights: Vec<MapLight>,
    pub chars: Vec<MapChar>,
    /// Bytes after the chars that Memoria's parser never reads.
    pub trailing: Vec<u8>,
}

impl MapConfig {
    /// Bytes read by the Memoria parser.
    pub fn consumed_bytes(&self) -> usize {
        MAP_CONFIG_HEADER_SIZE
            + self.lights.len() * MAP_CONFIG_LIGHT_SIZE
            + self.chars.len() * MAP_CONFIG_CHAR_SIZE
    }

    /// Size Memoria's `DMSGetMapConfSize` expects (adds `2 * evt_count`).
    pub fn memoria_size(&self) -> usize {
        self.consumed_bytes() + 2 * usize::from(self.evt_count)
    }

    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        if bytes.len() < MAP_CONFIG_HEADER_SIZE {
            return Err(ImportError::invalid(
                source,
                format!("map config has {} bytes, header needs 12", bytes.len()),
            ));
        }
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let light_count = bytes[6];
        let char_count = bytes[8];
        let needed = MAP_CONFIG_HEADER_SIZE
            + usize::from(light_count) * MAP_CONFIG_LIGHT_SIZE
            + usize::from(char_count) * MAP_CONFIG_CHAR_SIZE;
        if bytes.len() < needed {
            return Err(ImportError::invalid(
                source,
                format!(
                    "map config has {} bytes but {light_count} lights and {char_count} chars need {needed}",
                    bytes.len()
                ),
            ));
        }
        let mut at = MAP_CONFIG_HEADER_SIZE;
        let s = |b: u8| i16::from(b as i8) << 3;
        let mut lights = Vec::with_capacity(light_count.into());
        for _ in 0..light_count {
            let d = &bytes[at..at + MAP_CONFIG_LIGHT_SIZE];
            lights.push(MapLight {
                kind: d[0] as i8,
                no: d[1] as i8,
                shadow_intensity: s(d[2]),
                shadow_radius: d[3] as i8,
                color: [s(d[4]), s(d[5]), s(d[6]), s(d[7])],
                floor: [d[8] as i8, d[9] as i8, d[10] as i8, d[11] as i8],
            });
            at += MAP_CONFIG_LIGHT_SIZE;
        }
        let mut chars = Vec::with_capacity(char_count.into());
        for _ in 0..char_count {
            let d = &bytes[at..at + MAP_CONFIG_CHAR_SIZE];
            chars.push(MapChar {
                geo_no: u16::from_le_bytes([d[0], d[1]]),
                shadow_intensity: s(d[2]),
                shadow_radius: d[3] as i8,
                color: [s(d[4]), s(d[5]), s(d[6])],
                shadow_z: d[7] as i8,
            });
            at += MAP_CONFIG_CHAR_SIZE;
        }
        Ok(Self {
            attr: u16_at(0),
            version: u16_at(2),
            bg_no: u16_at(4),
            light_count,
            light_use: bytes[7],
            char_count,
            char_use: bytes[9],
            evt_count: bytes[10],
            evt_use: bytes[11],
            lights,
            chars,
            trailing: bytes[at..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        // attr 1, version 2, bg 0x1234, 1 light (use 1), 1 char (use 1), 3 events (use 3)
        let mut b = vec![1, 0, 2, 0, 0x34, 0x12, 1, 1, 1, 1, 3, 3];
        b.extend([5, 6, 0xFF, 9, 1, 2, 3, 0x80, 10, 11, 12, 0xF6]);
        b.extend([7, 0, 4, 8, 1, 0xFF, 2, 0xFE]);
        b.extend([0; 6]);
        b
    }

    #[test]
    fn parses_header_lights_and_chars() {
        let c = MapConfig::parse(&fixture(), "t").unwrap();
        assert_eq!((c.attr, c.version, c.bg_no), (1, 2, 0x1234));
        assert_eq!((c.light_use, c.char_count, c.char_use), (1, 1, 1));
        assert_eq!((c.evt_count, c.evt_use), (3, 3));
        let l = &c.lights[0];
        assert_eq!(
            (l.kind, l.no, l.shadow_intensity, l.shadow_radius),
            (5, 6, -8, 9)
        );
        assert_eq!(l.color, [8, 16, 24, -1024]);
        assert_eq!(l.floor, [10, 11, 12, -10]);
        let ch = &c.chars[0];
        assert_eq!(
            (ch.geo_no, ch.shadow_intensity, ch.shadow_radius),
            (7, 32, 8)
        );
        assert_eq!(ch.color, [8, -8, 16]);
        assert_eq!(ch.shadow_z, -2);
        assert_eq!(c.consumed_bytes(), 32);
        assert_eq!(c.memoria_size(), 38);
        assert_eq!(c.trailing.len(), 6);
    }

    #[test]
    fn rejects_short_inputs() {
        assert!(MapConfig::parse(&[0; 11], "t").is_err());
        let bytes = fixture();
        assert!(MapConfig::parse(&bytes[..30], "t").is_err());
    }
}
