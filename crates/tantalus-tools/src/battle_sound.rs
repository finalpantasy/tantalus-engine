//! Sound effects of a live battle, from the data the reference plays them from.
//!
//! * An enemy's attack sequence (`SeqOp::Sfx`, `btlseq.SeqExecSfx`) names the effect and the
//!   frame it plays on: `btl_util.SetBattleSfx(regist, SfxNum, SfxVol)`.
//! * A dying enemy plays effects 121 and 120 as its fade starts (`btl_mot.cs:318`).
//! * A party member's weapon attack plays its swing and hit sounds from the character's weapon
//!   sound pair (`BattleParameters.csv`, `FF9Snd.ff9battleSoundWeaponSndEffect02`): the swing
//!   as the attack starts, the hit when the damage lands (not on a miss, `SFX.cs:1331`).
//! * A spell or ability plays the sounds its effect file's sequence starts (`PLAY_SOUND`),
//!   named by the effect's entry in `SpecialEffectMetaData.txt`, ahead of the figure point.
//! * The command menu's own sounds ([`menu_cue`]): the cursor, backing out, a refusal, and the
//!   chime when a party member's turn comes up (`BattleHUD.cs:1285`).
//!
//! A spell's sounds are the plugin's own sound commands (host callback 32, `SFX.SoundPlay` and
//! `SoundPlayChant`): the effect is played once on the translated plugin and each command keeps
//! its plugin frame (one battle tick each), so a sound lands that many ticks after the effect
//! starts, as the view starts it. The sound number goes through `SFX.AdjustSoundIndex` and then
//! indexes the resident list or the effect's list. What is not exact: the pitch changes
//! (`soundFPS`) are not played, and the effect starts a fixed `CAST_LEAD` after the command
//! where the machine uses each character's own lead. Effects the plugin cannot play fall back to
//! the effect file's sequence, ahead of the figure point ([`EFFECT_TICKS`]).

use std::collections::BTreeMap;
use std::path::Path;

use tantalus_assets::pc::battle_sequence::{InterpretOptions, MarkerKind, SequenceFile, interpret};
use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
use tantalus_assets::pc::special_effect::EffectFile;

use crate::battle_effects::{CAST_LEAD, MIN_EFFECT_TICKS};
use crate::battle_stage::{BattleStage, StageEvent, StageModel};
use crate::play_audio::AudioCue;

/// A sequence sound: the sequence frame it plays on (one battle tick each), effect id, volume.
pub type SequenceSfx = (u32, u16, u8);

/// `btl_mot.cs:318`: the effects an enemy's death fade starts with, in play order.
const ENEMY_DEATH_EFFECTS: [u16; 2] = [121, 120];
const FULL_VOLUME: u8 = 127;
/// `BattleAbilityId.Attack`: a party member's weapon attack.
/// `BattleCommandId.Item`.
const ITEM_COMMAND: u32 = 14;
const ATTACK_ABILITY: i32 = 176;
/// Ticks from a command starting to its effect landing, as the battle view times it
/// (`battle_motion::EFFECT_TICKS`).
pub const EFFECT_TICKS: u32 = 40;
/// Ticks from a weapon attack starting to its swing sound (the run up to the target).
const SWING_TICKS: u32 = 10;

/// `FF9Snd.ff9battleSoundWeaponSndEffect02` by `CharacterSerialNumber` 0 to 18: the swing and
/// the hit sound effect ids.
const WEAPON_SOUNDS: [(u16, u16); 19] = [
    (899, 21),
    (900, 83),
    (904, 22),
    (904, 22),
    (906, 898),
    (904, 22),
    (906, 898),
    (902, 70),
    (902, 70),
    (901, 20),
    (904, 22),
    (906, 898),
    (901, 20),
    (905, 897),
    (900, 22),
    (902, 63),
    (902, 63),
    (902, 63),
    (900, 70),
];

/// The sounds of each sequence of a scene's `.raw17`, by sequence number.
pub fn sequence_sounds(bytes: &[u8]) -> Vec<Vec<SequenceSfx>> {
    let Ok(file) = SequenceFile::parse(bytes, "raw17") else {
        return Vec::new();
    };
    (0..file.slots.len())
        .map(|index| {
            let Some(sequence) = file.sequence(index) else {
                return Vec::new();
            };
            let options = InterpretOptions {
                base_anim: file.slots[index].base_anim,
                ..InterpretOptions::default()
            };
            interpret(sequence, &options)
                .markers
                .into_iter()
                .filter_map(|marker| match marker.kind {
                    MarkerKind::SfxTrigger { id, volume } => Some((marker.frame, id, volume)),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// The sounds of the spell and ability effects: ability to effect (`Actions.csv`), and for each
/// effect the sound files its sequence starts with how long before its figure point.
#[derive(Debug, Default)]
pub struct EffectSounds {
    /// Ability id to `AnimationId` (`AA_DATA.Info.VfxIndex`).
    effect_of: BTreeMap<i32, i32>,
    /// Item id to the `AnimationId` of its `ItemEffects.csv` row (the Item command's effect).
    item_effect_of: BTreeMap<i32, i32>,
    /// Effect id to `(ticks before the figure point, sound resource)`.
    timelines: BTreeMap<i32, Vec<(u32, String)>>,
    /// Effect id to its sound list (`SpecialEffectMetaData.txt`).
    lists: BTreeMap<i32, Vec<String>>,
    /// The resident sound list (`ResidentSpecialEffectMetaData.txt`): sound numbers 0 to 11.
    resident: Vec<String>,
    /// The translated plugin, whose sound commands give the exact frames.
    plugin: Option<std::sync::Arc<crate::battle_effects::PluginTiming>>,
    /// Each party model's own cast lead by serial number, as the battle machine uses it.
    cast_leads: std::sync::Arc<BTreeMap<u8, tantalus_legacy::battle::machine::CastLead>>,
}

/// A sound the plugin asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginSound {
    /// A sound resource and its pitch (1 is the sound's own).
    Resource(String, f32),
    /// The Reflect chime (`SFX.REFLECT_SOUND_ID`).
    Reflect,
}

/// `SFX.REFLECT_SOUND_ID`.
const REFLECT_SOUND_ID: u16 = 350;
/// Sounds the resident list holds (`SfxSoundPlayer.GetResidentSoundCount`).
const RESIDENT_SOUNDS: i32 = 12;

/// `SFX.AdjustSoundIndex`: the sound number `SoundPlay(dno, _, sce)` plays on effect `effect`
/// after `calls` earlier plays, or `None` when the call is skipped.
fn adjust_sound_index(effect: i32, dno: i32, sce: i32, calls: u32, fps: &mut i32) -> Option<i32> {
    const PHOENIX_REBIRTH_FLAME: i32 = 225;
    const MADEEN_FULL: i32 = 251;
    const ARK_FULL: i32 = 381;
    const ARK_SHORT: i32 = 447;
    const BAHAMUT_FULL: i32 = 227;
    const LUCKY_SEVEN: i32 = 312;
    const ROULETTE: i32 = 237;
    const LEVIATHAN_FULL: i32 = 179;
    const EXPLOSION: i32 = 31;
    const VENOM_POWDER: i32 = 10;
    const ODIN_FULL: i32 = 261;
    let dno = dno + 12;
    if sce != 0 {
        match effect {
            PHOENIX_REBIRTH_FLAME => return Some(dno + 12),
            MADEEN_FULL => return Some(dno + 11),
            ARK_FULL => {
                return Some(match calls {
                    28.. => dno + 36,
                    20..=27 => dno + 20,
                    12..=19 => dno + 16,
                    _ => dno + 8,
                });
            }
            ARK_SHORT => return Some(dno + 4),
            BAHAMUT_FULL => return Some(dno + 14),
            _ => {}
        }
    }
    match effect {
        ARK_SHORT if calls >= 8 => Some(dno + 8),
        ARK_FULL if calls >= 28 => Some(dno + 28),
        ARK_FULL if calls >= 12 => Some(dno + 12),
        // The first sound of Ark and Odin and the fifth of Bahamut change the pitch
        // (`soundFPS` -1, -3 and -2).
        ARK_FULL if calls == 0 => {
            *fps = -1;
            Some(dno)
        }
        ODIN_FULL if calls == 0 => {
            *fps = -3;
            Some(dno)
        }
        LUCKY_SEVEN => Some(dno - 7),
        ROULETTE if dno != 12 => Some(dno - 3),
        BAHAMUT_FULL => {
            if calls == 4 {
                *fps = -2;
            }
            Some(dno - 2)
        }
        LEVIATHAN_FULL => (dno != 15).then_some(dno),
        EXPLOSION => (dno == 12).then_some(dno),
        VENOM_POWDER => Some(dno - 3),
        _ => Some(dno),
    }
}

/// `SFX.soundFPS` the pitch starts at (`SFX.Play`): a pitch of 1.
const DEFAULT_SOUND_FPS: i32 = 4;
/// `SpecialEffect.Odin__Short`: every sound plays at 1.25.
const ODIN_SHORT: i32 = 424;

/// The pitch `SFX.SoundPlay` gives a sound for `soundFPS` `fps` on effect `effect`.
fn sound_pitch(effect: i32, fps: i32) -> f32 {
    if effect == ODIN_SHORT {
        return 1.25;
    }
    match fps + 3 {
        0 => 0.725,
        1 => 0.85,
        2 => 0.76,
        6 => 0.75,
        _ => 1.0,
    }
}

/// The sounds an effect's plugin commands play, by plugin frame (`SFX.SoundPlay` for mode 0 and
/// `SoundPlayChant` for mode 1; the other modes stream or stop).
pub fn plugin_sounds(
    effect: i32,
    calls: &[crate::battle_effects::SoundCall],
    list: &[String],
    resident: &[String],
) -> Vec<(u32, PluginSound)> {
    let mut out = Vec::new();
    let mut played = 0_u32;
    let mut chant = 0_i32;
    let mut fps = DEFAULT_SOUND_FPS;
    let by_number = |num: i32, pitch: f32| -> Option<PluginSound> {
        let (names, index) = if num < RESIDENT_SOUNDS {
            (resident, num)
        } else {
            (list, num - RESIDENT_SOUNDS)
        };
        let index = usize::try_from(index).ok()?;
        Some(PluginSound::Resource(names.get(index)?.clone(), pitch))
    };
    for &(frame, [mode, dno, _, sce]) in calls {
        match mode {
            0 => {
                let Some(num) = adjust_sound_index(effect, dno, sce, played, &mut fps) else {
                    continue;
                };
                played += 1;
                let pitch = sound_pitch(effect, fps);
                out.extend(by_number(num, pitch).map(|sound| (frame, sound)));
            }
            1 => {
                let kind = dno & 0xFF_FFFF;
                if kind == 3 {
                    if chant % 3 == 0 {
                        out.push((frame, PluginSound::Reflect));
                    }
                } else {
                    out.extend(by_number(kind * 3 + chant, 1.0).map(|sound| (frame, sound)));
                }
                chant += 1;
            }
            // `soundFPS` (callback 125).
            125 => fps = dno,
            _ => {}
        }
    }
    out
}

/// Effect id to its sound resources, in rank order.
pub type SoundLists = BTreeMap<i32, Vec<String>>;

/// The effect sound lists and the resident list, straight from `resources.assets`.
pub fn effect_sound_lists(game_data: &Path) -> Result<(SoundLists, Vec<String>), String> {
    let mut file = StandaloneSerialized::open(&game_data.join("resources.assets"))
        .map_err(|error| error.to_string())?;
    let assets = file.text_assets().map_err(|error| error.to_string())?;
    let mut lists = BTreeMap::new();
    let mut resident = Vec::new();
    for (name, object) in &assets {
        if name == "ResidentSpecialEffectMetaData.txt" {
            let text = file.read_text_asset(object).map_err(|e| e.to_string())?;
            resident = parse_resident_list(&text.script);
        } else if name == "SpecialEffectMetaData.txt" {
            let text = file.read_text_asset(object).map_err(|e| e.to_string())?;
            lists = parse_sound_lists(&text.script)?;
        }
    }
    Ok((lists, resident))
}

impl EffectSounds {
    /// Reads the effect files and their sound lists from `<game_data>/resources.assets` and the
    /// ability table from `Actions.csv`.
    pub fn load(game_data: &Path, actions: &Path) -> Result<Self, String> {
        let mut file = StandaloneSerialized::open(&game_data.join("resources.assets"))
            .map_err(|error| error.to_string())?;
        let assets = file.text_assets().map_err(|error| error.to_string())?;
        let mut lists: BTreeMap<i32, Vec<String>> = BTreeMap::new();
        let mut resident = Vec::new();
        let mut effects = Vec::new();
        for (name, object) in &assets {
            if name == "ResidentSpecialEffectMetaData.txt" {
                let text = file.read_text_asset(object).map_err(|e| e.to_string())?;
                resident = parse_resident_list(&text.script);
            } else if name == "SpecialEffectMetaData.txt" {
                let text = file.read_text_asset(object).map_err(|e| e.to_string())?;
                lists = parse_sound_lists(&text.script)?;
            } else if let Some(id) = name
                .strip_prefix("ef")
                .filter(|digits| digits.len() == 3)
                .and_then(|digits| digits.parse::<i32>().ok())
            {
                let asset = file.read_text_asset(object).map_err(|e| e.to_string())?;
                effects.push((id, name.clone(), asset.script));
            }
        }
        let mut timelines = BTreeMap::new();
        for (id, name, bytes) in &effects {
            let Some(list) = lists.get(id) else { continue };
            if let Some(timeline) = effect_timeline(bytes, name, list) {
                timelines.insert(*id, timeline);
            }
        }
        let text = std::fs::read_to_string(actions)
            .map_err(|error| format!("cannot read {}: {error}", actions.display()))?;
        // `Items.csv` EffectId (column 3) picks the `ItemEffects.csv` row whose AnimationId
        // (column 4) is the effect.
        let items_dir = actions
            .parent()
            .and_then(Path::parent)
            .map(|data| data.join("Items"));
        let item_effect_of = items_dir
            .and_then(|dir| {
                Some(parse_item_effects(
                    &std::fs::read_to_string(dir.join("Items.csv")).ok()?,
                    &std::fs::read_to_string(dir.join("ItemEffects.csv")).ok()?,
                ))
            })
            .unwrap_or_default();
        Ok(Self {
            effect_of: parse_actions(&text),
            item_effect_of,
            timelines,
            lists,
            resident,
            plugin: None,
            cast_leads: Default::default(),
        })
    }

    /// Uses the translated plugin's sound commands for the spells it can play.
    pub fn with_plugin(
        mut self,
        plugin: Option<std::sync::Arc<crate::battle_effects::PluginTiming>>,
    ) -> Self {
        self.plugin = plugin;
        self
    }

    /// Each party model's cast motions (`BattleConfig::cast_leads`), which move the effect, and
    /// so its sounds, later than the default lead.
    pub fn with_cast_leads(
        mut self,
        leads: std::sync::Arc<BTreeMap<u8, tantalus_legacy::battle::machine::CastLead>>,
    ) -> Self {
        self.cast_leads = leads;
        self
    }

    /// Battle frames from the command's start to `PlaySFX` for the party member with serial
    /// number `serial`, as the machine times it (`CastLead::ticks`); the default lead without
    /// that model's motions.
    fn cast_lead(&self, serial: u8, item: bool, many: bool, step: bool) -> u32 {
        self.cast_leads
            .get(&serial)
            .map_or(CAST_LEAD, |lead| lead.ticks(item, many, step))
    }

    /// The plugin's sounds for ability `id` (or Item command item `id`), as `(plugin frame,
    /// sound)`, and the effect's `hit` frame; `None` when the plugin does not play that effect.
    fn plugin_play(&self, id: i32, item: bool) -> Option<(Vec<(u32, PluginSound)>, u32)> {
        let effect = if item {
            *self.item_effect_of.get(&id)?
        } else {
            *self.effect_of.get(&id)?
        };
        let plugin = self.plugin.as_ref()?;
        let calls = plugin.sounds(effect, item)?;
        let hit = plugin.timing(effect, item)?.hit;
        let list = self.lists.get(&effect).map_or(&[][..], Vec::as_slice);
        Some((plugin_sounds(effect, &calls, list, &self.resident), hit))
    }

    /// The sounds of ability `ability`'s effect.
    pub fn for_ability(&self, ability: i32) -> &[(u32, String)] {
        self.effect_of
            .get(&ability)
            .and_then(|effect| self.timelines.get(effect))
            .map_or(&[], Vec::as_slice)
    }
}

/// The sounds of the Item command's effect for `item`.
impl EffectSounds {
    pub fn for_item(&self, item: i32) -> &[(u32, String)] {
        self.item_effect_of
            .get(&item)
            .and_then(|effect| self.timelines.get(effect))
            .map_or(&[], Vec::as_slice)
    }
}

/// Item id to effect id: `Items.csv` column 3 into `ItemEffects.csv` column 4.
fn parse_item_effects(items: &str, effects: &str) -> BTreeMap<i32, i32> {
    let animation: BTreeMap<i32, i32> = effects
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            let mut fields = line.split(';');
            let id = fields.next()?.trim().parse().ok()?;
            let effect = fields.nth(3)?.trim().parse().ok()?;
            Some((id, effect))
        })
        .collect();
    items
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            let mut fields = line.split(';');
            let item = fields.next()?.trim().parse().ok()?;
            let effect_id: i32 = fields.nth(2)?.trim().parse().ok()?;
            Some((item, *animation.get(&effect_id)?))
        })
        .collect()
}

/// `ResidentSpecialEffectMetaData.txt`: `{"data": {"0": ["<resource>", ..]}}`, which carries
/// trailing commas, so the quoted names are read directly.
fn parse_resident_list(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let Some(start) = text.find('\x5b') else {
        return Vec::new();
    };
    let body = &text[start..];
    let body = &body[..body.find('\x5d').unwrap_or(body.len())];
    body.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// `SpecialEffectMetaData.txt`: `{"data": {"<effect>": ["<resource>", ..]}}`.
fn parse_sound_lists(bytes: &[u8]) -> Result<BTreeMap<i32, Vec<String>>, String> {
    let text = String::from_utf8_lossy(bytes);
    let value: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("SpecialEffectMetaData.txt: {error}"))?;
    let mut lists = BTreeMap::new();
    for (key, list) in value["data"].as_object().into_iter().flatten() {
        let Ok(id) = key.parse::<i32>() else { continue };
        let names = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect();
        lists.insert(id, names);
    }
    Ok(lists)
}

/// `Actions.csv` (`Comment;Id;Display;Targets;DefaultAlly;ForDead;DefaultDead;DefaultCamera;
/// AnimationId;..`): ability id to its effect id.
fn parse_actions(text: &str) -> BTreeMap<i32, i32> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            let mut fields = line.split(';');
            let ability = fields.nth(1)?.trim().parse().ok()?;
            let effect = fields.nth(6)?.trim().parse().ok()?;
            Some((ability, effect))
        })
        .collect()
}

/// The sounds an effect file starts and when, as ticks before its figure point.
///
/// `PLAY_SOUND`'s sound number is the rank of its small file among the effect's sound files
/// (`SFX.SoundPlay` indexes the effect's list from 0; 341 of the 372 effect files have exactly
/// as many list entries as distinct sound files).
fn effect_timeline(bytes: &[u8], name: &str, list: &[String]) -> Option<Vec<(u32, String)>> {
    let mut file = EffectFile::parse(bytes, name).ok()?;
    file.resolve_small_files(bytes, name).ok()?;
    let files: BTreeMap<usize, ()> = file
        .sequence_summary
        .sound_refs
        .iter()
        .filter_map(|reference| Some((reference.file?, ())))
        .collect();
    let file_of: BTreeMap<usize, Option<usize>> = file
        .sequence_summary
        .sound_refs
        .iter()
        .map(|reference| (reference.op_offset, reference.file))
        .collect();
    let mut frame = 0_u32;
    let mut figure = None;
    let mut starts = Vec::new();
    for op in &file.sequence {
        match op.code {
            // WAIT: a plain wait counts frames; the others wait on the engine (an animation).
            0x01 if op.arg1 == 0 => frame += u32::from(op.arg2),
            // EFFECT_POINT: `arg2` 1 is the figure point.
            _ if op.name == Some("EFFECT_POINT") && op.arg2 == 1 && figure.is_none() => {
                figure = Some(frame);
            }
            0x2D => {
                let Some(file_index) = file_of.get(&op.offset).copied().flatten() else {
                    continue;
                };
                let rank = files.keys().position(|&known| known == file_index)?;
                starts.push((frame, rank));
            }
            _ => {}
        }
    }
    let figure = figure.unwrap_or(frame);
    Some(
        starts
            .into_iter()
            .filter_map(|(at, rank)| Some((figure.saturating_sub(at), list.get(rank)?.clone())))
            .collect(),
    )
}

/// What the command menu looked like, for [`menu_cue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuSnapshot {
    /// The party member being asked (machine unit index).
    pub unit: Option<usize>,
    /// 0 the command window, 1 the ability or item list, 2 picking a target.
    pub depth: u8,
    /// The cursor row at that depth.
    pub cursor: usize,
}

/// `BattleHUD.SwitchPlayer`: a party member's turn comes up.
const TURN_UP_SOUND: i32 = 1044;
/// `ButtonGroupState` cursor move and a confirmed pick (`BattleHUD.Scene.cs`).
const CURSOR_SOUND: i32 = 103;
/// Backing out of a list or the target cursor.
const BACK_SOUND: i32 = 101;
/// A pick that is refused.
const REFUSED_SOUND: i32 = 102;

/// The menu sound for one frame of input. `sent` is whether the frame chose a command.
pub fn menu_cue(
    before: MenuSnapshot,
    after: MenuSnapshot,
    confirm: bool,
    cancel: bool,
    sent: bool,
) -> Option<AudioCue> {
    let effect = |id| Some(AudioCue::Effect { id });
    if after.unit.is_some() && after.unit != before.unit {
        return effect(TURN_UP_SOUND);
    }
    if sent {
        return effect(CURSOR_SOUND);
    }
    before.unit?;
    if after.depth > before.depth {
        return effect(CURSOR_SOUND);
    }
    if after.depth < before.depth {
        return effect(BACK_SOUND);
    }
    if after.cursor != before.cursor {
        return effect(CURSOR_SOUND);
    }
    if confirm && after == before {
        return effect(REFUSED_SOUND);
    }
    let _ = cancel;
    None
}

#[derive(Debug, Clone, PartialEq)]
enum Due {
    /// A battle sound effect id (`SetBattleSfx`).
    Effect(u16, u8),
    /// A sound resource an effect file names.
    Resource(String, f32),
}

/// Follows a battle's stage frames and hands out the sounds that fall due.
#[derive(Debug, Default)]
pub struct BattleSounds {
    seen: usize,
    tick: u64,
    due: Vec<(u64, Due)>,
    /// Party units whose command in progress is a weapon attack.
    attacking: Vec<usize>,
}

impl BattleSounds {
    /// Reads the stage frames added since the last call and returns the sounds due now.
    pub fn step(
        &mut self,
        stage: &BattleStage,
        party_len: usize,
        effects: Option<&EffectSounds>,
    ) -> Vec<AudioCue> {
        if stage.frames.len() < self.seen {
            *self = Self::default();
        }
        for frame in &stage.frames[self.seen..] {
            for event in &frame.events {
                self.schedule(event, stage, party_len, effects, &frame.rows);
            }
        }
        self.seen = stage.frames.len();
        self.tick += 1;
        let tick = self.tick;
        let mut cues = Vec::new();
        self.due.retain(|(at, sound)| {
            let ready = *at <= tick;
            if ready {
                cues.push(match sound {
                    Due::Effect(id, volume) => AudioCue::BattleEffect {
                        id: i32::from(*id),
                        volume: i32::from(*volume),
                    },
                    Due::Resource(resource, pitch) => AudioCue::EffectSound {
                        resource: resource.clone(),
                        pitch: *pitch,
                    },
                });
            }
            !ready
        });
        cues
    }

    fn add(&mut self, after: u32, sound: Due) {
        self.due.push((self.tick + u64::from(after), sound));
    }

    /// Queues the plugin's own sound commands for a spell, item or ability. They fall due their
    /// plugin frame after the effect starts. Returns whether the plugin played the effect.
    fn schedule_plugin(
        &mut self,
        effects: Option<&EffectSounds>,
        id: i32,
        item: bool,
        lead: u32,
    ) -> bool {
        let Some((sounds, hit)) = effects.and_then(|effects| effects.plugin_play(id, item)) else {
            return false;
        };
        // The view starts the effect `hit` frames before the HP change, which the machine puts
        // at `CAST_LEAD` + `hit` ticks (and no sooner than `MIN_EFFECT_TICKS`).
        // The caster's own lead (`lead`) moves both by its difference from the default lead,
        // as `BattleMachine` moves the HP change.
        let start = CAST_LEAD.max(MIN_EFFECT_TICKS.saturating_sub(hit));
        let start = (i64::from(start) + i64::from(lead) - i64::from(CAST_LEAD)).max(0) as u32;
        for (frame, sound) in sounds {
            self.add(
                start + frame,
                match sound {
                    PluginSound::Resource(resource, pitch) => Due::Resource(resource, pitch),
                    PluginSound::Reflect => Due::Effect(REFLECT_SOUND_ID, FULL_VOLUME),
                },
            );
        }
        true
    }

    fn schedule(
        &mut self,
        event: &StageEvent,
        stage: &BattleStage,
        party_len: usize,
        effects: Option<&EffectSounds>,
        rows: &[u8],
    ) {
        // The party caster's lead before `PlaySFX` (`BattleMachine::command_ticks`): the step
        // forward from the back row at enemies, the chant (twice for many targets) and the cast.
        let lead_of = |unit: usize, item: bool, targets: &[usize]| -> u32 {
            let Some(StageModel::Player { serial, .. }) =
                stage.units.get(unit).map(|unit| &unit.model)
            else {
                return CAST_LEAD;
            };
            let at_enemies = !targets.is_empty() && targets.iter().all(|&t| t >= party_len);
            let step = rows.get(unit) == Some(&0) && at_enemies;
            effects.map_or(CAST_LEAD, |effects| {
                effects.cast_lead(*serial, item, targets.len() > 1, step)
            })
        };
        match *event {
            StageEvent::Command { unit, sub_no, .. } if unit >= party_len => {
                let sounds = usize::try_from(sub_no)
                    .ok()
                    .and_then(|sub| stage.sfx.get(sub));
                for &(frame, id, volume) in sounds.into_iter().flatten() {
                    self.add(frame, Due::Effect(id, volume));
                }
            }
            StageEvent::Command {
                unit,
                cmd_no,
                sub_no,
                ref targets,
                ..
            } if cmd_no == ITEM_COMMAND => {
                if self.schedule_plugin(effects, sub_no, true, lead_of(unit, true, targets)) {
                    return;
                }
                for (before, resource) in effects.into_iter().flat_map(|e| e.for_item(sub_no)) {
                    self.add(
                        EFFECT_TICKS.saturating_sub(*before),
                        Due::Resource(resource.clone(), 1.0),
                    );
                }
            }
            StageEvent::Command { unit, sub_no, .. } if sub_no == ATTACK_ABILITY => {
                self.attacking.push(unit);
                if let Some((swing, _)) = weapon_sounds(stage, unit) {
                    self.add(SWING_TICKS, Due::Effect(swing, FULL_VOLUME));
                }
            }
            StageEvent::Command {
                unit,
                sub_no,
                ref targets,
                ..
            } => {
                self.attacking.retain(|&attacker| attacker != unit);
                if self.schedule_plugin(effects, sub_no, false, lead_of(unit, false, targets)) {
                    return;
                }
                for (before, resource) in effects.into_iter().flat_map(|e| e.for_ability(sub_no)) {
                    self.add(
                        EFFECT_TICKS.saturating_sub(*before),
                        Due::Resource(resource.clone(), 1.0),
                    );
                }
            }
            StageEvent::Effect {
                caster,
                damage,
                target: _,
            } if caster < party_len && damage != 0 && self.attacking.contains(&caster) => {
                if let Some((_, hit)) = weapon_sounds(stage, caster) {
                    self.add(0, Due::Effect(hit, FULL_VOLUME));
                }
            }
            StageEvent::Died { unit } if unit >= party_len => {
                for id in ENEMY_DEATH_EFFECTS {
                    self.add(0, Due::Effect(id, FULL_VOLUME));
                }
            }
            _ => {}
        }
    }
}

/// The swing and hit sound ids of party unit `unit`'s weapon.
fn weapon_sounds(stage: &BattleStage, unit: usize) -> Option<(u16, u16)> {
    match stage.units.get(unit)?.model {
        StageModel::Player { serial, .. } => WEAPON_SOUNDS.get(usize::from(serial)).copied(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_fall_due_after_their_frame() {
        let mut sounds = BattleSounds::default();
        sounds.due.push((2, Due::Effect(77, 100)));
        let stage = BattleStage::default();
        assert!(sounds.step(&stage, 1, None).is_empty());
        let cues = sounds.step(&stage, 1, None);
        assert_eq!(cues.len(), 1);
    }

    #[test]
    fn plugin_sound_numbers_follow_adjust_sound_index() {
        let list: Vec<String> = ["a", "b", "c"].map(str::to_owned).to_vec();
        let resident: Vec<String> = (0..12).map(|n| format!("r{n}")).collect();
        let res = |name: &str| PluginSound::Resource(name.to_owned(), 1.0);
        // A plain effect: sound number n plays the effect's list entry n.
        let calls = [(3, [0, 1, 0, 0]), (9, [0, 0, 0, 0]), (12, [1, 1, 0, 0])];
        assert_eq!(
            plugin_sounds(1, &calls, &list, &resident),
            [(3, res("b")), (9, res("a")), (12, res("r3"))]
        );
        // Explosion (31) skips every call but the first sound; Lucky Seven (312) shifts by 7.
        assert_eq!(
            plugin_sounds(
                31,
                &[(0, [0, 1, 0, 0]), (1, [0, 0, 0, 0])],
                &list,
                &resident
            ),
            [(1, res("a"))]
        );
        assert_eq!(adjust_sound_index(312, 7, 0, 0, &mut 4), Some(12));
        // Odin and Bahamut change the pitch (`soundFPS`) on their first and fifth sounds.
        let mut fps = 4;
        assert_eq!(adjust_sound_index(261, 0, 0, 0, &mut fps), Some(12));
        assert_eq!(fps, -3);
        assert_eq!(sound_pitch(261, fps), 0.725);
        assert_eq!(sound_pitch(424, 4), 1.25);
        assert_eq!(sound_pitch(1, 4), 1.0);
        // A pitch command (125) applies to the sounds after it.
        let pitched = [(0, [0, 0, 0, 0]), (1, [125, -3, 0, 0]), (2, [0, 0, 0, 0])];
        assert_eq!(
            plugin_sounds(1, &pitched, &list, &resident)
                .into_iter()
                .map(|(frame, sound)| (
                    frame,
                    matches!(sound, PluginSound::Resource(_, p) if p == 1.0)
                ))
                .collect::<Vec<_>>(),
            [(0, true), (2, false)]
        );
        // Reflect's chant sound plays on every third chant call.
        let chant = [(0, [1, 3, 0, 0]), (1, [1, 3, 0, 0]), (2, [1, 3, 0, 0])];
        assert_eq!(
            plugin_sounds(1, &chant, &list, &resident),
            [(0, PluginSound::Reflect)]
        );
        assert_eq!(
            parse_resident_list(
                b"{
 \"data\": {\"0\": [\"x/a\",
	\"x/b\",
 ]}}"
            ),
            ["x/a", "x/b"]
        );
    }

    /// Runs only with `FFIX_GAME_DATA` (the install's `FF9_Data`) and `FFIX_ACTIONS_CSV` set.
    #[test]
    fn real_cure_plays_its_three_sounds_before_the_figure() {
        let (Some(data), Some(actions)) = (
            std::env::var_os("FFIX_GAME_DATA"),
            std::env::var_os("FFIX_ACTIONS_CSV"),
        ) else {
            return;
        };
        let sounds = EffectSounds::load(Path::new(&data), Path::new(&actions)).unwrap();
        let cure = sounds.for_ability(1);
        println!("CURE {cure:?}");
        assert_eq!(cure.len(), 3);
        assert!(cure.iter().all(|(_, name)| name.starts_with("Sounds0")));
        assert!(!sounds.for_ability(25).is_empty(), "Fire has sounds");
        // Items: Potion (236) and Phoenix Down (249) each play an effect with sounds.
        let potion = sounds.for_item(236);
        println!("POTION {potion:?}");
        assert!(!potion.is_empty(), "the Potion's effect has sounds");
    }

    /// Runs only with `FFIX_GAME_DATA` and `FFIX_ACTIONS_CSV` set.
    #[test]
    fn real_spells_play_their_sounds_on_plugin_frames() {
        let (Some(data), Some(actions)) = (
            std::env::var_os("FFIX_GAME_DATA"),
            std::env::var_os("FFIX_ACTIONS_CSV"),
        ) else {
            return;
        };
        let mut table = crate::battle_effects::EffectTable::default();
        table.load_plugin_timing(Path::new(&data));
        let sounds = EffectSounds::load(Path::new(&data), Path::new(&actions))
            .unwrap()
            .with_plugin(table.plugin.clone());
        // Cure 1, Fire 25, Ifrit 51.
        for ability in [1, 25, 51] {
            let (played, hit) = sounds.plugin_play(ability, false).expect("plugin plays it");
            println!("ability {ability}: hit {hit} sounds {played:?}");
            assert!(!played.is_empty(), "ability {ability} plays sounds");
            assert!(played.iter().all(|(_, sound)| matches!(
                sound,
                PluginSound::Resource(..) | PluginSound::Reflect
            )));
        }
    }

    fn stage_with(events: Vec<Vec<StageEvent>>) -> BattleStage {
        use crate::battle_stage::{StageFrame, StageUnit};
        let unit = |model| StageUnit {
            model,
            position: [0.0; 3],
            turn: 0.0,
            max_hp: 100,
            max_mp: 0,
            type_no: None,
            name: None,
        };
        BattleStage {
            units: vec![
                unit(StageModel::Player {
                    character: 0,
                    serial: 0,
                }),
                unit(StageModel::Enemy {
                    geo: 0,
                    idle: 0,
                    hit: 0,
                    death: 0,
                }),
            ],
            frames: events
                .into_iter()
                .map(|events| StageFrame {
                    events,
                    ..StageFrame::default()
                })
                .collect(),
            ..BattleStage::default()
        }
    }

    #[test]
    fn a_dagger_attack_swings_then_hits_and_a_dying_enemy_is_heard() {
        let attack = StageEvent::Command {
            unit: 0,
            cmd_no: 1,
            sub_no: ATTACK_ABILITY,
            targets: vec![1],
            short_summon: false,
        };
        let hit = StageEvent::Effect {
            caster: 0,
            target: 1,
            damage: 9,
        };
        let stage = stage_with(vec![
            vec![attack],
            vec![],
            vec![hit],
            vec![StageEvent::Died { unit: 1 }],
        ]);
        let mut sounds = BattleSounds::default();
        let mut heard = Vec::new();
        for _ in 0..16 {
            for cue in sounds.step(&stage, 1, None) {
                if let AudioCue::BattleEffect { id, .. } = cue {
                    heard.push(id);
                }
            }
        }
        // Zidane's dagger: swing 899, hit 21; the enemy's death effects 121 and 120.
        assert_eq!(heard, vec![21, 121, 120, 899]);
    }

    #[test]
    fn actions_csv_gives_each_abilitys_effect() {
        let text = "#! Include\n# header\nCure;1;Hp(1);ManyAny(3);1;1;0;0;9;8;10;16;;0;71;0;6;1;255;# Cure\n";
        assert_eq!(parse_actions(text).get(&1), Some(&9));
    }

    #[test]
    fn menu_sounds_follow_the_reference() {
        let idle = MenuSnapshot::default();
        let ready = MenuSnapshot {
            unit: Some(0),
            ..idle
        };
        let id = |cue: Option<AudioCue>| match cue {
            Some(AudioCue::Effect { id }) => id,
            _ => 0,
        };
        // A turn coming up chimes, moving the cursor ticks, going deeper confirms, backing out
        // cancels, and confirming on nothing is refused.
        assert_eq!(id(menu_cue(idle, ready, false, false, false)), 1044);
        let moved = MenuSnapshot { cursor: 1, ..ready };
        assert_eq!(id(menu_cue(ready, moved, false, false, false)), 103);
        let deeper = MenuSnapshot { depth: 1, ..ready };
        assert_eq!(id(menu_cue(ready, deeper, true, false, false)), 103);
        assert_eq!(id(menu_cue(deeper, ready, false, true, false)), 101);
        assert_eq!(id(menu_cue(ready, ready, true, false, false)), 102);
        assert!(menu_cue(ready, ready, false, false, false).is_none());
    }
}
