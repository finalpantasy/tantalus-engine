//! Which spell effect a party command plays and when it lands, read from
//! Memoria's exported effect sequences (`Data/SpecialEffects/ef<id>/
//! Sequence.seq`, run by `UnifiedBattleSequencer`) and the battle tables.
//!
//! The effect itself is the special-effect plugin's MIPS script and is not
//! run; the client draws stand-ins for a few spells
//! (`tantalus_client::battle_effects`) on this timing:
//! - `btl_vfx.GetPlayerCommandSFX`: an item plays its `ItemEffects.csv`
//!   `AnimationId` (column 4); an ability its `Actions.csv` `AnimationId`
//!   (column 8, `VfxIndex`), or `AnimationId2` (column 9, `Vfx2`) when it
//!   can target many (`ManyAny`) but was aimed at one.
//! - In `Sequence.seq`, `Wait: Time=N` waits N battle frames,
//!   `EffectPoint ... Type=Effect` applies the effect (the HP change) and
//!   `Type=Figure` shows the damage numbers. `SetBackgroundIntensity`
//!   dims the scene (`Intensity`, fade `Time`, `HoldDuration`).

use std::collections::BTreeMap;
use std::path::Path;

/// `BattleAbilityId.Attack`.
const ABILITY_ATTACK: i32 = 176;

/// When an effect's parts happen, in battle frames from `PlaySFX`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EffectTiming {
    /// The first `EffectPoint Type=Effect`.
    pub hit: u32,
    /// The first `EffectPoint Type=Figure` (the damage numbers).
    pub figure: u32,
    /// All the waits.
    pub length: u32,
    /// `SetBackgroundIntensity` at the start: intensity, fade frames and
    /// hold frames.
    pub dim: Option<(f32, u32, u32)>,
}

/// `Sequence.seq` -> timing. `None` without an `EffectPoint`.
pub fn parse_effect_sequence(text: &str) -> Option<EffectTiming> {
    let mut at = 0u32;
    let mut hit = None;
    let mut figure = None;
    let mut dim = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        let (op, args) = line.split_once(':').unwrap_or((line, ""));
        let arg = |name: &str| {
            args.split(';').find_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                (key.trim() == name).then(|| value.trim().to_owned())
            })
        };
        let number = |name: &str| arg(name).and_then(|value| value.parse::<f32>().ok());
        match op.trim() {
            "Wait" => at += number("Time").unwrap_or(0.0).max(0.0) as u32,
            "EffectPoint" => match arg("Type").as_deref() {
                Some("Effect") if hit.is_none() => hit = Some(at),
                Some("Figure") if figure.is_none() => figure = Some(at),
                _ => {}
            },
            "SetBackgroundIntensity" if dim.is_none() => {
                if let Some(intensity) = number("Intensity")
                    && intensity < 1.0
                {
                    dim = Some((
                        intensity,
                        number("Time").unwrap_or(0.0) as u32,
                        number("HoldDuration").unwrap_or(0.0) as u32,
                    ));
                }
            }
            _ => {}
        }
    }
    let hit = hit.or(figure)?;
    Some(EffectTiming {
        hit,
        figure: figure.unwrap_or(hit).max(hit),
        length: at.max(hit),
        dim,
    })
}

/// Effect ids of abilities and items, with their timing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectTable {
    /// Ability id -> `VfxIndex`, `Vfx2` and whether it targets `ManyAny`.
    pub abilities: BTreeMap<i32, (i32, i32, bool)>,
    /// Item id -> effect id (`ItemEffects.csv` `AnimationId` through
    /// `Items.csv` `EffectId`).
    pub items: BTreeMap<i32, i32>,
    /// Effect id -> timing, for every `ef<id>/Sequence.seq` found.
    pub timing: BTreeMap<i32, EffectTiming>,
}

impl EffectTable {
    /// Reads every `ef<id>/Sequence.seq` under `special_effects`; missing
    /// files leave the table empty.
    pub fn load_timing(&mut self, special_effects: &Path) {
        let Ok(entries) = std::fs::read_dir(special_effects) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_prefix("ef"))
                .and_then(|id| id.parse::<i32>().ok())
            else {
                continue;
            };
            let Ok(bytes) = std::fs::read(entry.path().join("Sequence.seq")) else {
                continue;
            };
            if let Some(timing) = parse_effect_sequence(&String::from_utf8_lossy(&bytes)) {
                self.timing.insert(id, timing);
            }
        }
    }

    /// The effect a party member's command on `targets` units plays:
    /// `cmd_no` 14 (Item) reads the item table, any other the abilities.
    pub fn effect_of(
        &self,
        cmd_no: u32,
        sub_no: i32,
        targets: usize,
    ) -> Option<(i32, EffectTiming)> {
        // Attack plays the weapon's own effect (`GetPlayerAttackVfx`).
        if cmd_no == tantalus_legacy::battle::queue::cmd::ATTACK || sub_no == ABILITY_ATTACK {
            return None;
        }
        let id = if cmd_no == tantalus_legacy::battle::queue::cmd::ITEM {
            *self.items.get(&sub_no)?
        } else {
            let (many, single, many_any) = *self.abilities.get(&sub_no)?;
            if many_any && targets == 1 {
                single
            } else {
                many
            }
        };
        Some((id, *self.timing.get(&id)?))
    }
}

/// A camera the effect starts (`PLAY_CAMERA`), `frame` battle frames after
/// `PlaySFX`: one of `choices` (an `IsAlternate` play picks among the
/// `SETUP_CAMERA` list), each the bytes of a camera set (the
/// `SFXDataCamera` layout of a scene camera set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectCameraPlay {
    pub frame: u32,
    pub choices: Vec<Vec<u8>>,
}

/// The effect's camera plays, from its sequence stream (`SFXBinaryFile`):
/// `WAIT` with type 0 advances the frame (the loading waits of type 1 do
/// not), `LOAD_CHUNK` picks the small-file group, `SETUP_CAMERA` builds the
/// alternatives (255 clears them) and `PLAY_CAMERA` plays its slot (`arg2`
/// 0) or one of the alternatives (`arg2` 3).
pub fn effect_camera_plays(
    effect: &tantalus_assets::pc::special_effect::EffectFile,
    bytes: &[u8],
) -> Vec<EffectCameraPlay> {
    use tantalus_assets::pc::special_effect::RecordDetail;
    let small_file = |chunk: usize, slot: u8| -> Option<Vec<u8>> {
        effect
            .records
            .iter()
            .filter(|record| record.group == chunk)
            .find_map(|record| match &record.detail {
                RecordDetail::SmallFiles(table) => {
                    let file = usize::try_from(*table.slot_to_file.get(usize::from(slot))?).ok()?;
                    let entry = table.files.get(file)?;
                    let start = table.base_offset + entry.offset;
                    bytes.get(start..start + entry.size).map(<[u8]>::to_vec)
                }
                _ => None,
            })
    };
    let mut frame = 0u32;
    let mut chunk = 0usize;
    let mut setup: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    for op in &effect.sequence {
        match op.code {
            0x00 => break,
            0x01 if op.arg1 == 0 => frame += u32::from(op.arg2),
            0x05 => chunk = usize::from(op.arg1),
            0x23 if op.arg1 == 0xFF => setup.clear(),
            0x23 => setup.push(op.arg1),
            0x29 => {
                let slots = match op.arg2 {
                    0 => vec![op.arg1],
                    3 => setup.clone(),
                    _ => continue,
                };
                let choices: Vec<Vec<u8>> = slots
                    .iter()
                    .filter_map(|slot| small_file(chunk, *slot))
                    .collect();
                if !choices.is_empty() {
                    out.push(EffectCameraPlay { frame, choices });
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thunder_dims_and_lands_after_its_waits() {
        let text = "// Shared sequence of SFX Thunder__Single\n\n\
            SetBackgroundIntensity: Intensity=0.5 ; Time=4 ; HoldDuration=41\n\
            PlaySound: Sound=14\nWait: Time=11\n\
            EffectPoint: Char=AllTargets ; Type=Effect\nWait: Time=14\n\
            EffectPoint: Char=Everyone ; Type=Figure\nWait: Time=16\n\
            SetBackgroundIntensity: Intensity=1 ; Time=8\n";
        assert_eq!(
            parse_effect_sequence(text),
            Some(EffectTiming {
                hit: 11,
                figure: 25,
                length: 41,
                dim: Some((0.5, 4, 41)),
            })
        );
        assert_eq!(parse_effect_sequence("Wait: Time=3\n"), None);
    }

    #[test]
    fn items_and_abilities_resolve_separately() {
        let mut table = EffectTable::default();
        table.abilities.insert(25, (70, 0, true));
        table.items.insert(236, 8);
        table.timing.insert(0, EffectTiming::default());
        table.timing.insert(70, EffectTiming::default());
        table.timing.insert(
            8,
            EffectTiming {
                hit: 5,
                ..EffectTiming::default()
            },
        );
        assert_eq!(table.effect_of(22, 25, 1).map(|e| e.0), Some(0));
        assert_eq!(table.effect_of(22, 25, 3).map(|e| e.0), Some(70));
        assert_eq!(
            table.effect_of(14, 236, 1).map(|e| (e.0, e.1.hit)),
            Some((8, 5))
        );
        assert_eq!(table.effect_of(14, 25, 1), None);
    }
}
