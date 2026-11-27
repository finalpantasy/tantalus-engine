//! Scripted play window (`play --harness DIR`): the real play loop of [`crate::live`] with a
//! timed button script instead of the keyboard, a virtual display clock, a hidden window, a
//! PNG every few frames and one JSON line a frame.
//!
//! The window is never shown: it is created hidden and its frames are read back with Bevy's
//! screenshot. The clock is virtual (`--harness-hz`, default the frame cap or 60): every frame
//! advances the game by exactly `1 / hz` seconds whatever the wall clock did, so two runs of
//! the same script give the same logic and only the drawing differs between frame-rate
//! settings. The wall-clock time each frame took is logged beside it (`wall_ms`).
//!
//! Sound runs in-process without a device: the cues the game raises go to an
//! [`AudioDirector`], rendered for the frame's virtual time, and the song it plays is logged.
//!
//! Script: `FROM-TO:HEXMASK,...` (or `@FILE` with one entry a line, `#` comments), times in
//! 1/30 s of virtual time (field ticks); the masks held over each range are or-ed.

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use bevy::prelude::*;
use tantalus_tools::play::{PlayGame, pad};
use tantalus_tools::play_audio::{AudioCue, AudioDirector};
use tantalus_tools::sound_bank::SoundBank;

/// The harness settings from the `play` flags.
#[derive(Debug, Clone)]
pub struct HarnessSettings {
    pub out: PathBuf,
    /// Held ranges in 1/30 s of virtual time: (from, to, mask).
    pub script: Vec<(u64, u64, i32)>,
    /// Press confirm once a dialog has been open this many frames (0: never).
    pub auto_confirm: u64,
    /// Frames to run before exiting.
    pub frames: u64,
    /// A PNG every this many frames (0: none).
    pub shot_every: u64,
    /// The virtual display rate.
    pub hz: f64,
    /// What to press in battle instead of the script (`--battle`), see [`BattlePolicy`].
    pub battle: Option<BattlePolicy>,
}

/// What the harness does in battle: each time a member's command menu opens it taps one
/// command, one tap every [`TAP_TICKS`]: the first row (Attack), the second (Steal or the first
/// magic list), the fourth (Item, the first item), the three in turn, L1 and R1 held to flee,
/// or nothing (`idle`, to lose a fight). Confirm taps close the result screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattlePolicy {
    Attack,
    Row2,
    Row4,
    Mix,
    Flee,
    Idle,
}

impl BattlePolicy {
    pub fn parse(text: &str) -> Result<Self, String> {
        Ok(match text {
            "attack" => Self::Attack,
            "row2" => Self::Row2,
            "row4" => Self::Row4,
            "mix" => Self::Mix,
            "flee" => Self::Flee,
            "idle" => Self::Idle,
            _ => {
                return Err(format!(
                    "bad --battle `{text}` (attack|row2|row4|mix|flee|idle)"
                ));
            }
        })
    }

    /// The taps for the `turn`th command menu of the battle.
    fn taps(self, turn: u64) -> &'static [i32] {
        const C: i32 = pad::CONFIRM | pad::CIRCLE;
        const D: i32 = pad::DOWN;
        // Each starts with a release, so a button still held from the last turn is pressed anew.
        const ROWS: [&[i32]; 3] = [&[0, C, C, C], &[0, D, C, C, C], &[0, D, D, D, C, C, C]];
        match self {
            Self::Attack => ROWS[0],
            Self::Row2 => ROWS[1],
            Self::Row4 => ROWS[2],
            Self::Mix => ROWS[(turn % 3) as usize],
            Self::Flee | Self::Idle => &[],
        }
    }

    /// The pad bits `ticks` (1/30 s) into the `turn`th command menu, or into the result screen.
    pub fn held(self, turn: u64, ticks: u64, ended: bool) -> i32 {
        let tap = (ticks / TAP_TICKS) as usize;
        let down = ticks % TAP_TICKS < TAP_TICKS / 2;
        if ended {
            return if down { pad::CONFIRM | pad::CIRCLE } else { 0 };
        }
        if self == Self::Flee {
            return pad::L1 | pad::R1;
        }
        if self == Self::Idle {
            return 0;
        }
        let taps = self.taps(turn);
        // Still in the menu once the pattern is done (an item or spell that cannot be used
        // buzzes): back out to the commands and Attack, after a pause of three taps.
        let fallback: [i32; 9] = [
            pad::CANCEL,
            0,
            pad::CANCEL,
            pad::UP,
            pad::UP,
            pad::UP,
            pad::CONFIRM | pad::CIRCLE,
            pad::CONFIRM | pad::CIRCLE,
            pad::CONFIRM | pad::CIRCLE,
        ];
        let bits = match tap.checked_sub(taps.len() + 3) {
            None => taps.get(tap).copied().unwrap_or(0),
            Some(late) => fallback
                .get(late % (fallback.len() + 3))
                .copied()
                .unwrap_or(0),
        };
        if down { bits } else { 0 }
    }
}

/// Ticks (1/30 s) per battle tap: half pressed, half released.
pub const TAP_TICKS: u64 = 12;

/// Parses a `FROM-TO:HEXMASK,...` list, or `@FILE` with one entry a line.
pub fn parse_script(text: &str) -> Result<Vec<(u64, u64, i32)>, String> {
    let owned;
    let list: Vec<&str> = if let Some(path) = text.strip_prefix('@') {
        owned = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
        owned
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .filter(|line| !line.is_empty())
            .flat_map(|line| line.split(','))
            .collect()
    } else {
        text.split(',').collect()
    };
    list.iter()
        .map(|part| {
            let part = part.trim();
            part.split_once(':')
                .and_then(|(range, mask)| {
                    let (from, to) = range.split_once('-')?;
                    Some((
                        from.trim().parse().ok()?,
                        to.trim().parse().ok()?,
                        i32::from_str_radix(mask.trim().trim_start_matches("0x"), 16).ok()?,
                    ))
                })
                .ok_or_else(|| format!("bad script entry `{part}`, want FROM-TO:HEXMASK"))
        })
        .collect()
}

/// In-process sound for the harness: the window's cues, no device.
pub struct HeadlessSound {
    director: AudioDirector,
    describe: SoundBank,
    /// Sound ticks (1/60 s) owed to the director.
    carry: f64,
}

/// The sample rate the headless director renders at (a multiple of 60; the samples are dropped).
const RATE: u32 = 6_000;

impl HeadlessSound {
    pub fn open(streaming_assets: &std::path::Path, game_data: &std::path::Path) -> Option<Self> {
        let open = || SoundBank::open(streaming_assets, game_data);
        let bank = open()
            .map_err(|error| eprintln!("harness sound: {error}"))
            .ok()?;
        let describe = open().ok()?;
        let director = AudioDirector::new(bank, RATE)
            .map_err(|error| eprintln!("harness sound: {error}"))
            .ok()?;
        Some(Self {
            director,
            describe,
            carry: 0.0,
        })
    }

    pub fn apply(&mut self, cues: &[AudioCue]) -> Vec<String> {
        cues.iter()
            .map(|cue| {
                self.director.apply(cue);
                tantalus_tools::play_audio::describe_cue(cue, &self.describe)
            })
            .collect()
    }

    /// Runs the sound system for `seconds` of play.
    pub fn advance(&mut self, seconds: f64) {
        self.carry += seconds * 60.0;
        let ticks = self.carry.floor();
        if ticks >= 1.0 {
            self.carry -= ticks;
            let _ = self.director.render(ticks as u32);
        }
    }

    pub fn music(&self) -> i32 {
        self.director.system.current_music_id()
    }
}

/// What the harness knows between frames.
#[derive(Resource)]
pub struct Harness {
    pub settings: HarnessSettings,
    pub frame: u64,
    /// Virtual seconds played.
    pub seconds: f64,
    log: std::io::BufWriter<std::fs::File>,
    last_wall: Option<Instant>,
    previous: String,
    /// Frames a dialog has been open.
    open_for: u64,
    last_confirm: bool,
    /// The pad bits this frame's tick used.
    pub held: i32,
    /// The mode the last frame ended in, and virtual seconds spent in battle.
    last_mode: String,
    /// The battle state the last frame ended in, the command menus seen, and virtual seconds
    /// since the current one (or the result screen) opened.
    last_battle: Option<String>,
    turns: u64,
    turn_seconds: f64,
}

impl Harness {
    pub fn new(settings: HarnessSettings) -> Result<Self, String> {
        std::fs::create_dir_all(&settings.out)
            .map_err(|error| format!("{}: {error}", settings.out.display()))?;
        let path = settings.out.join("frames.jsonl");
        let log =
            std::fs::File::create(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Self {
            settings,
            frame: 0,
            seconds: 0.0,
            log: std::io::BufWriter::new(log),
            last_wall: None,
            previous: String::new(),
            open_for: 0,
            last_confirm: false,
            held: 0,
            last_mode: String::new(),
            last_battle: None,
            turns: 0,
            turn_seconds: 0.0,
        })
    }

    /// The script time of this frame, in 1/30 s.
    pub fn script_tick(&self) -> u64 {
        (self.seconds * 30.0 + 1e-6) as u64
    }

    /// The pad bits for this frame: the script's, plus an auto-confirm press.
    pub fn next_held(&mut self) -> i32 {
        let tick = self.script_tick();
        if let Some(policy) = self.settings.battle
            && self.last_mode == "battle"
        {
            let state = self.last_battle.as_deref().unwrap_or("");
            let ended = state.contains("ended true");
            let ticks = (self.turn_seconds * 30.0 + 1e-6) as u64;
            self.held = policy.held(self.turns.saturating_sub(1), ticks, ended);
            return self.held;
        }
        let mut held = self
            .settings
            .script
            .iter()
            .filter(|(from, to, _)| (*from..*to).contains(&tick))
            .fold(0, |mask, (_, _, bits)| mask | bits);
        if self.settings.auto_confirm > 0
            && self.open_for >= self.settings.auto_confirm
            && !self.last_confirm
        {
            held |= pad::CONFIRM | pad::CIRCLE;
            self.open_for = 0;
        }
        self.last_confirm = held & pad::CONFIRM != 0;
        self.held = held;
        held
    }
}

/// One frame's state, read after the frame's tick and draw.
pub struct FrameState<'a> {
    pub mode: &'a str,
    pub game: &'a PlayGame,
    pub position: Option<[f32; 3]>,
    pub battle: Option<String>,
    pub title: &'a str,
    /// The drawn field view center (blended between ticks).
    pub view: Option<[f32; 2]>,
    /// The song playing and the cues raised since the last frame.
    pub song: i32,
    pub cues: Vec<String>,
}

/// Writes the frame's log line, asks for its PNG, and exits after the last frame.
pub fn end_frame(world: &mut World, delta: f64, state: FrameState) {
    let Some(mut harness) = world.remove_resource::<Harness>() else {
        return;
    };
    let now = Instant::now();
    let wall_ms = harness
        .last_wall
        .map_or(0.0, |last| (now - last).as_secs_f64() * 1000.0);
    harness.last_wall = Some(now);
    harness.seconds += delta;
    // A member's command menu opening (or the result screen) starts the next tap pattern.
    let ready = |battle: Option<&String>| {
        battle.is_some_and(|b| {
            (b.contains("unit Some") && b.contains(" Commands ")) || b.contains("ended true")
        })
    };
    let unit = |battle: Option<&String>| {
        battle.and_then(|b| {
            b.split(" unit ")
                .nth(1)?
                .split(' ')
                .next()
                .map(str::to_owned)
        })
    };
    if ready(state.battle.as_ref())
        && (!ready(harness.last_battle.as_ref())
            || unit(state.battle.as_ref()) != unit(harness.last_battle.as_ref()))
    {
        harness.turns += 1;
        harness.turn_seconds = 0.0;
    } else {
        harness.turn_seconds += delta;
    }
    if state.mode != "battle" {
        harness.turns = 0;
    }
    harness.last_battle = state.battle.clone();
    harness.last_mode = state.mode.to_owned();
    let game = state.game;
    let text: Vec<String> = game.open_text().into_iter().map(|(_, line)| line).collect();
    harness.open_for = if text.is_empty() {
        0
    } else {
        harness.open_for + 1
    };
    let menu = game.menu_frame().map(|(_, frame)| {
        frame
            .windows
            .iter()
            .flat_map(|window| window.texts.iter().map(|text| text.text.clone()))
            .take(6)
            .collect::<Vec<_>>()
            .join("|")
            + &format!(" @{:?}", frame.pointer)
    });
    // What a press can change: a state that stays the same while input is held is a stuck input.
    let signature = format!(
        "{}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{:?}",
        state.mode,
        game.map(),
        state.position.map(|p| p.map(|v| v.round() as i32)),
        text,
        menu,
        state.battle,
        game.cursor,
        game.stopped,
    );
    let changed = signature != harness.previous;
    harness.previous = signature;
    let frame = harness.frame;
    let line = serde_json::json!({
        "f": frame,
        "t": harness.script_tick(),
        "wall_ms": (wall_ms * 100.0).round() / 100.0,
        "mode": state.mode,
        // On the world map: the world script's map (9000 to 9012), not the field it left.
        "map": if game.world_play.is_some() {
            game.world_script_map
        } else {
            game.map().unwrap_or(-1)
        },
        "field": game.field_name().unwrap_or("-"),
        "ticks": game.ticks,
        "song": state.song,
        "pos": state.position,
        "held": harness.held,
        "changed": changed,
        "text": text,
        "menu": menu,
        "battle": state.battle,
        "cues": state.cues,
        "view": state.view,
        "stopped": game.stopped,
        "title": state.title,
    });
    let _ = writeln!(harness.log, "{line}");
    let every = harness.settings.shot_every;
    if every > 0 && frame.is_multiple_of(every) {
        use bevy::render::view::screenshot::{Screenshot, save_to_disk};
        let path = harness.settings.out.join(format!("f{frame:06}.png"));
        world
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
    harness.frame += 1;
    // A few frames past the end so the last screenshots are written.
    if harness.frame >= harness.settings.frames + 8 {
        let _ = harness.log.flush();
        // `TANTALUS_BENCH_FRAMES` set: the per-section frame times of the run.
        if crate::live::bench::frames().is_some() {
            crate::live::bench::report();
        }
        std::process::exit(0);
    }
    world.insert_resource(harness);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_entries_parse() {
        assert_eq!(
            parse_script("0-30:0x20, 30-31:4000").unwrap(),
            vec![(0, 30, 0x20), (30, 31, 0x4000)]
        );
        assert!(parse_script("0-30").is_err());
    }

    #[test]
    fn battle_taps_alternate_press_and_release() {
        let policy = BattlePolicy::Row2;
        assert_eq!(policy.held(0, 0, false), 0);
        assert_eq!(policy.held(0, TAP_TICKS, false), pad::DOWN);
        assert_eq!(policy.held(0, TAP_TICKS * 3 / 2, false), 0);
        assert_eq!(
            policy.held(0, TAP_TICKS * 2, false),
            pad::CONFIRM | pad::CIRCLE
        );
        assert_eq!(policy.held(0, TAP_TICKS * 9, false), 0);
        assert_eq!(BattlePolicy::Mix.held(1, TAP_TICKS, false), pad::DOWN);
        // Past the pattern and a pause: Cancel back out to the commands.
        assert_eq!(policy.held(0, TAP_TICKS * 8, false), pad::CANCEL);
        assert_eq!(policy.held(0, TAP_TICKS * 20, false), pad::CANCEL);
        assert_eq!(BattlePolicy::Flee.held(0, 0, false), pad::L1 | pad::R1);
        assert_eq!(
            BattlePolicy::Flee.held(0, 0, true),
            pad::CONFIRM | pad::CIRCLE
        );
    }
}
