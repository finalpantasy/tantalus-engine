//! Which spell effect a party command plays and when it lands, read from
//! Memoria's exported effect sequences (`Data/SpecialEffects/ef<id>/
//! Sequence.seq`, run by `UnifiedBattleSequencer`) and the battle tables.
//!
//! The effect itself is the special-effect plugin's MIPS script, which the
//! client plays (`tantalus_client::battle_effects`); the timing below is
//! the plugin's own where it matches the DLL and `Sequence.seq` otherwise:
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

/// Effects whose plugin run does not match the player's DLL
/// (`tantalus-sfx` `--test plugin sweep`): 212 to 224, where the DLL does
/// not repeat itself, and the sword skills Darkside, Minus Strike, Iai
/// Strike, Thunder Slash (both levels), Bio Sword, Lancer and Photon.
const NOT_MATCHING: [i32; 11] = [188, 189, 190, 191, 221, 299, 401, 409, 410, 411, 412];

/// The effects the plugin plays: those that match the DLL.
pub fn is_drawn(id: i32) -> bool {
    !(212..=224).contains(&id) && !NOT_MATCHING.contains(&id)
}

/// Frames an effect may play before it is cut off.
pub const MAX_FRAMES: u32 = 3000;

/// When effect `id` reaches its Effect and Figure points and its end, in
/// frames from `SFX_Play`, played once on one player (z -1500) against one
/// enemy (z 500). `None` when it cannot play or has no Effect point.
pub fn plugin_timing(
    runtime: &tantalus_sfx::SfxRuntime,
    id: i32,
    package: &[u8],
    item: bool,
) -> Option<EffectTiming> {
    use tantalus_sfx::player::{Setup, Unit};
    let unit = |btl_id: u16, player: bool, z: i16| Unit {
        btl_id,
        player,
        slot: 0,
        serial: 0,
        position: [0, 0, z],
        base: [0, 0, z],
        yaw: if player { 2048 } else { 0 },
        height: 512,
        radius: 256,
        geo_radius: 256,
        tar_bone: 0,
    };
    let setup = Setup {
        units: vec![unit(1, true, -1500), unit(0x10, false, 500)],
        caster: 0,
        targets: vec![1],
        item,
        args: [0; 3],
        put: [0, 0],
    };
    let mut run = runtime.play(id, package, &setup)?;
    // The view's first preset camera, as the PSX camera the plugin reads.
    let camera = tantalus_assets::pc::battle_camera::PsxCamera {
        rotation: [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]],
        translation: [0, 0, 0],
        h: 400,
    };
    while run.frame() < MAX_FRAMES && run.advance(&camera).is_some() {}
    let events = run.events();
    let hit = events.effect_points.first()?.0;
    let figure = events.figure_points.first().map_or(hit, |point| point.0);
    Some(EffectTiming {
        hit,
        figure: figure.max(hit),
        length: run.frame().max(hit),
        dim: None,
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
    /// The translated plugin, which times effects when first asked
    /// ([`Self::load_plugin_timing`]); its timing wins over `timing`.
    pub plugin: Option<std::sync::Arc<PluginTiming>>,
}

/// The translated plugin and the effect packages, timing each effect the
/// first time a command needs it (the long summons take a second each).
pub struct PluginTiming {
    runtime: tantalus_sfx::SfxRuntime,
    file: std::sync::Mutex<tantalus_assets::pc::serialized_inventory::StandaloneSerialized>,
    assets: Vec<(String, tantalus_assets::pc::serialized::SerializedObject)>,
    done: std::sync::Mutex<std::collections::HashMap<(i32, bool), Option<EffectTiming>>>,
}

impl std::fmt::Debug for PluginTiming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginTiming")
    }
}

/// Plugin state is runtime-only: tables compare by their data.
impl PartialEq for PluginTiming {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl PluginTiming {
    /// Effect `id`'s timing, played once per (effect, item).
    pub fn timing(&self, id: i32, item: bool) -> Option<EffectTiming> {
        if !is_drawn(id) {
            return None;
        }
        let mut done = self.done.lock().unwrap_or_else(|e| e.into_inner());
        *done.entry((id, item)).or_insert_with(|| {
            let name = format!("ef{id:03}");
            let (_, object) = self.assets.iter().find(|(asset, _)| *asset == name)?;
            let text = self
                .file
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .read_text_asset(object)
                .ok()?;
            plugin_timing(&self.runtime, id, &text.script, item)
        })
    }
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

    /// Plays every ability and item effect on the translated plugin
    /// (`tantalus_sfx`, from `Plugins/FF9SpecialEffectPlugin.dll` and the
    /// `efNNN` assets of `resources.assets` under `game_data`) and keeps
    /// when each reaches its first Effect point (the HP change: the plugin's
    /// host callback 23, `btl_cmd.ExecVfxCommand`), its Figure point
    /// (damage numbers, callback 24) and its end, as Steam does. The
    /// `Sequence.seq` timing stays for effects the plugin cannot play or
    /// does not play like the DLL ([`is_drawn`]).
    pub fn load_plugin_timing(&mut self, game_data: &Path) {
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        let Some(runtime) = std::fs::read(game_data.join("Plugins/FF9SpecialEffectPlugin.dll"))
            .ok()
            .and_then(tantalus_sfx::SfxRuntime::new)
        else {
            return;
        };
        let Ok(mut file) = StandaloneSerialized::open(&game_data.join("resources.assets")) else {
            return;
        };
        let Ok(assets) = file.text_assets() else {
            return;
        };
        self.plugin = Some(std::sync::Arc::new(PluginTiming {
            runtime,
            file: std::sync::Mutex::new(file),
            assets,
            done: Default::default(),
        }));
    }

    /// An effect's timing: the plugin's when it has one, else `Sequence.seq`.
    pub fn timing_of(&self, id: i32, item: bool) -> Option<EffectTiming> {
        let seq = self.timing.get(&id).copied();
        match self
            .plugin
            .as_ref()
            .and_then(|plugin| plugin.timing(id, item))
        {
            // The scene dim is the sequence's (`SetBackgroundIntensity`).
            Some(timing) => Some(EffectTiming {
                dim: seq.and_then(|seq| seq.dim),
                ..timing
            }),
            None => seq,
        }
    }

    /// The effect a party member's command on `targets` units plays:
    /// `cmd_no` 14 (Item) reads the item table, any other the abilities.
    /// `short_summon` picks `Vfx2` (`btl_vfx.cs:99`).
    pub fn effect_of(
        &self,
        cmd_no: u32,
        sub_no: i32,
        targets: usize,
        short_summon: bool,
    ) -> Option<(i32, EffectTiming)> {
        // Attack plays the weapon's own effect (`GetPlayerAttackVfx`).
        if cmd_no == tantalus_legacy::battle::queue::cmd::ATTACK || sub_no == ABILITY_ATTACK {
            return None;
        }
        let id = if cmd_no == tantalus_legacy::battle::queue::cmd::ITEM {
            *self.items.get(&sub_no)?
        } else {
            let (many, single, many_any) = *self.abilities.get(&sub_no)?;
            if (many_any && targets == 1) || short_summon {
                single
            } else {
                many
            }
        };
        Some((
            id,
            self.timing_of(id, cmd_no == tantalus_legacy::battle::queue::cmd::ITEM)?,
        ))
    }
}

/// Battle frames from a command's `CMD_MODE_LOOP` to its `PlaySFX` that
/// [`EffectTable::command_timings`] assumes; the battle moves each command
/// by its caster's own [`cast_leads`] (`BattleConfig::cast_leads`).
pub const CAST_LEAD: u32 = tantalus_legacy::battle::machine::DEFAULT_CAST_LEAD;

/// `BattlePlayerCharacter.PlayerMotionIndex` of the cast motions.
const MP_IDLE_TO_CHANT: usize = 26;
const MP_CHANT: usize = 27;
const MP_MAGIC: usize = 28;
const MP_STEP_FORWARD: usize = 29;
const MP_ITEM1: usize = 31;

/// Every party model's cast motions by serial (`BattleParameters.csv`
/// motions, frames from `audit/animation-frames.csv`). A model missing a
/// clip length is left out and keeps [`CAST_LEAD`].
pub fn cast_leads() -> BTreeMap<u8, tantalus_legacy::battle::machine::CastLead> {
    use tantalus_assets::pc::battle_models::{
        animation_db_key, player_attack_sequences, player_battle_motions,
    };
    let frames = crate::animation_frames::table();
    let mut out = BTreeMap::new();
    for (serial, _, _) in player_attack_sequences() {
        let Some(motions) = player_battle_motions(serial) else {
            continue;
        };
        let length = |index: usize| -> Option<u32> {
            let key = u16::try_from(animation_db_key(motions[index])?).ok()?;
            frames.get(&key).map(|&frames| u32::from(frames))
        };
        let lead = (|| {
            Some(tantalus_legacy::battle::machine::CastLead {
                step_forward: length(MP_STEP_FORWARD)?,
                idle_to_chant: length(MP_IDLE_TO_CHANT)?,
                chant: length(MP_CHANT)?,
                magic: length(MP_MAGIC)?,
                item: length(MP_ITEM1)?,
            })
        })();
        if let (Some(lead), Ok(serial)) = (lead, u8::try_from(serial)) {
            out.insert(serial, lead);
        }
    }
    out
}

/// The fewest ticks a command takes before its HP change and before it ends
/// (the old stand-in for every command).
pub const MIN_EFFECT_TICKS: u32 = 40;
/// `SpecialEffect.Special_Trance_Activate` and `Special_Trance_End`.
pub const TRANCE_ACTIVATE: i32 = 257;
pub const TRANCE_END: i32 = 489;
pub const MIN_COMMAND_TICKS: u32 = 75;

impl EffectTable {
    /// Each command's wait for its effect, for the battle machine
    /// (`BattleConfig::command_timing`): the HP change at the effect's
    /// first `EffectPoint Type=Effect` (`ExecVfxCommand` from the SFX,
    /// `btl_cmd.cs:1526`) and the end once the whole effect has played
    /// (`WaitSFXDone`, `CheckCommandLoop` `btl_cmd.cs:1441`), both
    /// [`CAST_LEAD`] after the command starts.
    pub fn command_timings(&self) -> tantalus_legacy::battle::machine::CommandTimings {
        let mut out = tantalus_legacy::battle::machine::CommandTimings::new();
        let abilities = self
            .abilities
            .keys()
            .flat_map(|&ability| (0u8..4).map(move |variant| (false, ability, variant)));
        let items = self
            .items
            .keys()
            .flat_map(|&item| (0u8..4).map(move |variant| (true, item, variant)));
        let trance = [true, false].map(tantalus_legacy::battle::machine::sys_trans_key);
        for key in abilities.chain(items).chain(trance) {
            if let Some(value) = self.command_timing_of(&key) {
                out.insert(key, value);
            }
        }
        out
    }

    /// The ticks effect `id` holds a command for: the HP change and the end
    /// ([`CAST_LEAD`] after the command starts).
    pub fn effect_timing_ticks(&self, effect: i32, item: bool) -> Option<(u32, u32)> {
        let timing = self.timing_of(effect, item)?;
        let hit = (CAST_LEAD + timing.hit).max(MIN_EFFECT_TICKS);
        let total = (CAST_LEAD + timing.length).max(hit).max(MIN_COMMAND_TICKS);
        Some((hit, total))
    }

    /// One command's wait (see [`Self::command_timings`]), played on the
    /// plugin only now.
    pub fn command_timing_of(&self, key: &(bool, i32, u8)) -> Option<(u32, u32)> {
        let (item, id, variant) = *key;
        let ticks = |effect: i32, item: bool| {
            let timing = &self.timing_of(effect, item)?;
            let hit = (CAST_LEAD + timing.hit).max(MIN_EFFECT_TICKS);
            let total = (CAST_LEAD + timing.length).max(hit).max(MIN_COMMAND_TICKS);
            Some((hit, total))
        };
        if item {
            return ticks(*self.items.get(&id)?, true);
        }
        let sys_trans = tantalus_legacy::battle::machine::sys_trans_key(true).1;
        if id == sys_trans {
            // `SysTrans` plays the Trance effect from its first frame; the
            // model changes at frame 75 and the command ends with the
            // effect.
            let effect = if variant == 1 {
                TRANCE_ACTIVATE
            } else {
                TRANCE_END
            };
            let timing = self.timing_of(effect, false)?;
            let swap = tantalus_legacy::battle::machine::SYS_TRANS_MODEL_FRAME;
            return Some((swap, timing.length.max(swap)));
        }
        if id == ABILITY_ATTACK {
            return None;
        }
        let &(many, single, many_any) = self.abilities.get(&id)?;
        let alternate = (many_any && variant & 1 != 0) || variant & 2 != 0;
        ticks(if alternate { single } else { many }, false)
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

    /// Zidane (serial 0) chants with `ANH_MAIN_B0_000_200`..`_202` (10, 13
    /// and 18 frames), steps forward in 7 and uses an item in 18.
    #[test]
    fn cast_leads_come_from_the_clip_lengths() {
        let leads = cast_leads();
        let zidane = leads[&0];
        assert_eq!(zidane.ticks(false, false, false), 41);
        assert_eq!(zidane.ticks(false, true, true), 7 + 10 + 26 + 18);
        assert_eq!(zidane.ticks(true, false, false), 18);
        assert!(leads.len() > 10);
    }

    /// Fire (hit 5, 49 frames) keeps the 40-tick HP change and ends at
    /// 35 + 49; a full Shiva (hit 288) waits for its hit, and its short
    /// version (variant bit 1) for the short one's.
    #[test]
    fn command_timings_wait_for_the_effect() {
        let mut table = EffectTable::default();
        table.abilities.insert(25, (0, 0, false));
        table.abilities.insert(49, (38, 407, false));
        let timing = |hit, length| EffectTiming {
            hit,
            figure: hit,
            length,
            dim: None,
        };
        table.timing.insert(0, timing(5, 49));
        table.timing.insert(38, timing(288, 364));
        table.timing.insert(407, timing(60, 120));
        let ticks = table.command_timings();
        assert_eq!(ticks.get(&(false, 25, 1)), Some(&(40, 84)));
        assert_eq!(ticks.get(&(false, 49, 1)), Some(&(323, 399)));
        assert_eq!(ticks.get(&(false, 49, 0)), Some(&(323, 399)));
        assert_eq!(ticks.get(&(false, 49, 3)), Some(&(95, 155)));
        assert_eq!(table.effect_of(16, 49, 1, true).map(|e| e.0), Some(407));
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
        assert_eq!(table.effect_of(22, 25, 1, false).map(|e| e.0), Some(0));
        assert_eq!(table.effect_of(22, 25, 3, false).map(|e| e.0), Some(70));
        assert_eq!(
            table.effect_of(14, 236, 1, false).map(|e| (e.0, e.1.hit)),
            Some((8, 5))
        );
        assert_eq!(table.effect_of(14, 25, 1, false), None);
    }
}
