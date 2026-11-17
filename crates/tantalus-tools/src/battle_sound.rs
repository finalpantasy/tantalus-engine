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
//! What is not exact: the effect plugin's own script is not run, so a sound's time is its offset
//! from the effect's figure point in the sequence, laid back from where the view puts the effect
//! ([`EFFECT_TICKS`]), and waits that depend on animations count as zero.

use std::collections::BTreeMap;
use std::path::Path;

use tantalus_assets::pc::battle_sequence::{InterpretOptions, MarkerKind, SequenceFile, interpret};
use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
use tantalus_assets::pc::special_effect::EffectFile;

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
}

impl EffectSounds {
    /// Reads the effect files and their sound lists from `<game_data>/resources.assets` and the
    /// ability table from `Actions.csv`.
    pub fn load(game_data: &Path, actions: &Path) -> Result<Self, String> {
        let mut file = StandaloneSerialized::open(&game_data.join("resources.assets"))
            .map_err(|error| error.to_string())?;
        let assets = file.text_assets().map_err(|error| error.to_string())?;
        let mut lists: BTreeMap<i32, Vec<String>> = BTreeMap::new();
        let mut effects = Vec::new();
        for (name, object) in &assets {
            if name == "SpecialEffectMetaData.txt" {
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
        })
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
    Resource(String),
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
                self.schedule(event, stage, party_len, effects);
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
                    Due::Resource(resource) => AudioCue::EffectSound {
                        resource: resource.clone(),
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

    fn schedule(
        &mut self,
        event: &StageEvent,
        stage: &BattleStage,
        party_len: usize,
        effects: Option<&EffectSounds>,
    ) {
        match *event {
            StageEvent::Command { unit, sub_no, .. } if unit >= party_len => {
                let sounds = usize::try_from(sub_no)
                    .ok()
                    .and_then(|sub| stage.sfx.get(sub));
                for &(frame, id, volume) in sounds.into_iter().flatten() {
                    self.add(frame, Due::Effect(id, volume));
                }
            }
            StageEvent::Command { cmd_no, sub_no, .. } if cmd_no == ITEM_COMMAND => {
                for (before, resource) in effects.into_iter().flat_map(|e| e.for_item(sub_no)) {
                    self.add(
                        EFFECT_TICKS.saturating_sub(*before),
                        Due::Resource(resource.clone()),
                    );
                }
            }
            StageEvent::Command { unit, sub_no, .. } if sub_no == ATTACK_ABILITY => {
                self.attacking.push(unit);
                if let Some((swing, _)) = weapon_sounds(stage, unit) {
                    self.add(SWING_TICKS, Due::Effect(swing, FULL_VOLUME));
                }
            }
            StageEvent::Command { unit, sub_no, .. } => {
                self.attacking.retain(|&attacker| attacker != unit);
                for (before, resource) in effects.into_iter().flat_map(|e| e.for_ability(sub_no)) {
                    self.add(
                        EFFECT_TICKS.saturating_sub(*before),
                        Due::Resource(resource.clone()),
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
