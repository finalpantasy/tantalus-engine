//! The typed object lists inside the `FF9.IMG` files (fields, the resident data, character and
//! monster data): how the PS1 game finds a field's or the menu's sound programs by sound id.
//!
//! Layout, worked out from the discs (no public documentation exists for it):
//! - A cluster starts with `0xDB`, a count byte and two pad bytes, then one `u32` per list: the low
//!   24 bits are the list's offset from that word's own address, the high byte the list's type.
//! - A list starts with its type byte, a count byte and two pad bytes, then `count` `u16` object
//!   ids (padded to a multiple of four bytes), then `count` `u32` offsets, each relative to its own
//!   address. An object ends where the next one starts (the last one where its list ends).
//! - An object can itself be a cluster (for example the field's own list of type `0x1B` holds the
//!   field's clusters), so the walk recurses.
//!
//! List type [`SOUND_PROGRAMS`] holds sound-effect programs keyed by the game's sound id: the same
//! number the field scripts pass to the sound driver and the Steam `SoundEffectMetaData.txt` uses
//! (`FF9Snd.FF9AllSoundDispatch` `ObjNo`). Every field carries the programs its scripts play
//! (field 50, Prima Vista, holds 108, 293, 349, 621, 628, 643, 914 and 923); file 10002 of the
//! first group holds the resident ones (menu cursor 103, confirm 101, cancel 102 and others). Each
//! id has one program on all four discs.

use std::ops::Range;

/// Cluster tag byte.
pub const CLUSTER: u8 = 0xDB;
/// List type of sound-effect programs (`AKAO` entries keyed by sound id).
pub const SOUND_PROGRAMS: u8 = 0x08;
/// List type of sample collections (`AKAO` with articulations, keyed by collection id).
pub const SAMPLE_COLLECTIONS: u8 = 0x09;

/// One object of a typed list: its list type, its id and where its bytes are in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterObject {
    pub list_type: u8,
    pub id: u16,
    pub range: Range<usize>,
}

fn u16le(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// Every object of every list reachable from the cluster at byte 0 of `data` (none when the file
/// is not a cluster). Malformed parts are skipped, not reported.
pub fn objects(data: &[u8]) -> Vec<ClusterObject> {
    let mut out = Vec::new();
    walk(data, 0, data.len(), 0, &mut out);
    out
}

fn walk(data: &[u8], base: usize, end: usize, depth: usize, out: &mut Vec<ClusterObject>) {
    if depth > 6 || data.get(base) != Some(&CLUSTER) {
        return;
    }
    let count = usize::from(data[base + 1]);
    // (type, start) of each list, in order; a list ends where the next one starts.
    let mut lists = Vec::with_capacity(count);
    for i in 0..count {
        let at = base + 4 + 4 * i;
        let Some(word) = u32le(data, at) else {
            return;
        };
        let start = at + (word & 0x00FF_FFFF) as usize;
        let kind = (word >> 24) as u8;
        if start >= end || data.get(start) != Some(&kind) {
            continue;
        }
        lists.push((kind, start));
    }
    for (i, &(kind, start)) in lists.iter().enumerate() {
        let list_end = lists.get(i + 1).map_or(end, |l| l.1).min(end);
        let n = usize::from(data[start + 1]);
        let table = start + 4 + (2 * n).div_ceil(4) * 4;
        let mut starts = Vec::with_capacity(n);
        for k in 0..n {
            let (Some(id), Some(rel)) =
                (u16le(data, start + 4 + 2 * k), u32le(data, table + 4 * k))
            else {
                return;
            };
            starts.push((id, table + 4 * k + rel as usize));
        }
        for (k, &(id, at)) in starts.iter().enumerate() {
            let stop = starts.get(k + 1).map_or(list_end, |s| s.1).min(list_end);
            if at >= stop {
                continue;
            }
            out.push(ClusterObject {
                list_type: kind,
                id,
                range: at..stop,
            });
            if data[at] == CLUSTER {
                walk(data, at, stop, depth + 1, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cluster with one list of type 8 holding two objects, the second of them a nested
    /// cluster with a list of type 9.
    fn sample() -> Vec<u8> {
        let mut d = vec![CLUSTER, 1, 0, 0];
        d.extend_from_slice(&((u32::from(SOUND_PROGRAMS) << 24) | 4).to_le_bytes());
        // List at 8: type 8, two ids (103, 7), offsets at 16 and 20.
        d.extend_from_slice(&[SOUND_PROGRAMS, 2, 0, 0]);
        d.extend_from_slice(&103u16.to_le_bytes());
        d.extend_from_slice(&7u16.to_le_bytes());
        d.extend_from_slice(&8u32.to_le_bytes()); // 16 + 8 = 24
        d.extend_from_slice(&12u32.to_le_bytes()); // 20 + 12 = 32
        d.extend_from_slice(b"AKAOprog");
        // Nested cluster at 32 with a type-9 list at 40.
        d.extend_from_slice(&[CLUSTER, 1, 0, 0]);
        d.extend_from_slice(&((u32::from(SAMPLE_COLLECTIONS) << 24) | 4).to_le_bytes());
        d.extend_from_slice(&[SAMPLE_COLLECTIONS, 1, 0, 0]);
        d.extend_from_slice(&5u16.to_le_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&4u32.to_le_bytes()); // 48 + 4 = 52
        d.extend_from_slice(b"AKAObank");
        d
    }

    #[test]
    fn walks_lists_and_nested_clusters() {
        let d = sample();
        let objs = objects(&d);
        assert_eq!(
            objs,
            vec![
                ClusterObject {
                    list_type: SOUND_PROGRAMS,
                    id: 103,
                    range: 24..32
                },
                ClusterObject {
                    list_type: SOUND_PROGRAMS,
                    id: 7,
                    range: 32..60
                },
                ClusterObject {
                    list_type: SAMPLE_COLLECTIONS,
                    id: 5,
                    range: 52..60
                },
            ]
        );
        assert_eq!(&d[objs[0].range.clone()], b"AKAOprog");
    }

    #[test]
    fn a_file_that_is_not_a_cluster_has_no_objects() {
        assert!(objects(b"AKAO\0\0\0\0").is_empty());
        let mut d = sample();
        d[4] = 0xF0; // list offset pointing past the end
        assert!(objects(&d).is_empty());
    }
}
