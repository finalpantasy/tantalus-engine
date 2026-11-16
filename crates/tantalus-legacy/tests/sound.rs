//! Sound state machine tests. All audio is synthetic; expected values are derived by hand from
//! the source arithmetic cited in `src/sound`.

use std::sync::Arc;
use tantalus_contracts::EntityId;
use tantalus_legacy::HostEvent;
use tantalus_legacy::sound::*;

const RATE: u32 = 44_100;
const FRAMES_PER_TICK: usize = 735; // 44100 / 60

fn clip(samples: Vec<i16>) -> PcmClip {
    PcmClip {
        sample_rate: RATE,
        channels: 1,
        samples,
        loop_start: 0,
        loop_end: 0,
        loop_start_alternate: 0,
        loop_end_alternate: 0,
    }
}

fn constant(value: i16, frames: usize) -> Arc<PcmClip> {
    Arc::new(clip(vec![value; frames]))
}

fn ramp(frames: usize, scale: i16) -> Vec<i16> {
    (0..frames).map(|i| (i as i16) * scale).collect()
}

fn entries(list: &[(i32, &str)]) -> Vec<MetaEntry> {
    list.iter().map(|(i, n)| (*i, (*n).to_owned())).collect()
}

fn system() -> SoundSystem {
    let meta = SoundMetadata {
        music: SoundMetadata::index(&entries(&[(10, "bgm10"), (11, "bgm11"), (5, "bgm5")]))
            .unwrap(),
        sound_effect: SoundMetadata::index(&entries(&[
            (100, "se100"),
            (101, "se101"),
            (1966, "se1966"),
            (2295, "se2295"),
            (2728, "se2728"),
        ]))
        .unwrap(),
        song: SoundMetadata::index(&entries(&[(7, "stream7")])).unwrap(),
        ..Default::default()
    };
    let mut sys = SoundSystem::new(Backend::new(RATE).unwrap(), meta);
    for name in [
        "bgm10", "bgm11", "bgm5", "se100", "se101", "se1966", "se2295", "se2728", "stream7",
    ] {
        sys.clips.insert(name.to_owned(), constant(16384, 200_000)); // 0.5
    }
    sys
}

struct NoProjector;
impl SoundProjector for NoProjector {
    fn project_position(&mut self, _: [i32; 3]) -> (i32, i32) {
        (0, 0)
    }
    fn project_object(&mut self, _: EntityId) -> Option<(i32, i32)> {
        None
    }
}

struct FixedProjector;
impl SoundProjector for FixedProjector {
    fn project_position(&mut self, p: [i32; 3]) -> (i32, i32) {
        (p[0], 100)
    }
    fn project_object(&mut self, o: EntityId) -> Option<(i32, i32)> {
        (o.get() == 3).then_some((9, 127))
    }
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}

#[test]
fn tick_arithmetic() {
    // ConvertTickToMillisec: RoundToInt(ticks * 1000 / 60), half to even.
    assert_eq!(convert_tick_to_millisec(60), 1000);
    assert_eq!(convert_tick_to_millisec(1), 17); // 16.67
    assert_eq!(convert_tick_to_millisec(3), 50);
    assert_eq!(convert_tick_to_millisec(0), 0);
    // ConvertMillisecToTick divides by 60 (not 16.67) in the source.
    assert_eq!(convert_millisec_to_tick(600.0), 10);
    close(normalize_volume(127), 1.0);
    close(normalize_volume(200), 1.0);
    close(normalize_volume(-5), 0.0);
    close(normalize_volume(64), 64.0 / 127.0);
    assert_eq!(reverse_normalize_volume(1.0), 127);
    assert_eq!(reverse_normalize_volume(2.0), 127);
    close(normalize_pitch(0), 1.0);
    close(normalize_pitch(255), 3.0);
}

#[test]
fn one_second_fade_reaches_target_at_tick_60() {
    let mut sys = system();
    sys.song_play(10, 127, 0);
    let first = sys.run_ticks(1);
    // volume 1.0, sample 0.5, mono duplicated to both channels
    close(first[0], 0.5);
    close(first[1], 0.5);
    // 60 ticks -> 1000 ms -> 44100 frames of linear fade from 1.0 to 64/127.
    sys.song_vol_intpl(10, 60, 64);
    let out = sys.run_ticks(61);
    let target = 64.0 / 127.0;
    let gain = |f: usize| {
        if f >= 44_100 {
            target
        } else {
            1.0 + (target - 1.0) * (f as f32 / 44_100.0)
        }
    };
    for f in [0, 1, 22_050, 44_099, 44_100, 44_100 + FRAMES_PER_TICK - 1] {
        close(out[2 * f], 0.5 * gain(f));
        close(out[2 * f + 1], 0.5 * gain(f));
    }
    // Midpoint is halfway between the endpoints.
    close(out[2 * 22_050], 0.5 * (1.0 + target) / 2.0);
}

#[test]
fn vol_fade_jumps_to_from_then_fades() {
    let mut sys = system();
    sys.song_play(10, 127, 0);
    sys.run_ticks(1);
    // from 0, to 127 over 30 ticks (500 ms = 22050 frames)
    sys.song_vol_fade(10, 30, 0, 127);
    let out = sys.run_ticks(31);
    close(out[0], 0.0);
    close(out[2 * 11_025], 0.5 * 0.5);
    close(out[2 * 22_050], 0.5);
}

#[test]
fn loop_wraps_at_loop_end() {
    let mut sys = system();
    let mut c = clip(ramp(10, 1000));
    c.loop_start = 2;
    c.loop_end = 6;
    sys.clips.insert("bgm10".into(), Arc::new(c));
    sys.song_play(10, 127, 0);
    let out = sys.run_ticks(1);
    // positions: 0 1 2 3 4 5, then 6 wraps to 2: 2 3 4 5 2 3
    let expect = [0, 1, 2, 3, 4, 5, 2, 3, 4, 5, 2, 3];
    for (f, idx) in expect.iter().enumerate() {
        close(out[2 * f], (*idx as f32) * 1000.0 / 32768.0);
    }
}

#[test]
fn clip_without_loop_ends_and_frees_the_voice() {
    let mut sys = system();
    sys.clips
        .insert("se100".into(), Arc::new(clip(ramp(5, 100))));
    sys.sndeffect_play(100, 0, 0, 127);
    assert_eq!(sys.sfx_channels().len(), 1);
    let out = sys.run_ticks(1);
    close(out[2 * 4], 400.0 / 32768.0);
    close(out[2 * 5], 0.0);
    // Update on the next tick drops the finished channel.
    sys.run_ticks(1);
    assert!(sys.sfx_channels().is_empty());
}

#[test]
fn jumppoint_switches_to_alternate_loop() {
    let mut sys = system();
    let mut c = clip(ramp(20, 1000));
    c.loop_start = 2;
    c.loop_end = 6;
    c.loop_start_alternate = 10;
    c.loop_end_alternate = 12;
    sys.clips.insert("bgm10".into(), Arc::new(c));
    sys.song_play(10, 127, 0);
    sys.song_jumppoint();
    let out = sys.run_ticks(1);
    // Alternate region 10..12 replaces the loop immediately; playback runs 0..11 then wraps to 10.
    let expect = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 10, 11];
    for (f, idx) in expect.iter().enumerate() {
        close(out[2 * f], (*idx as f32) * 1000.0 / 32768.0);
    }
}

#[test]
fn two_voices_sum() {
    let mut sys = system();
    sys.clips.insert("se100".into(), constant(8192, 50_000)); // 0.25
    sys.clips.insert("se101".into(), constant(4096, 50_000)); // 0.125
    sys.sndeffect_play(100, 0, 0, 127);
    sys.sndeffect_play(101, 0, 0, 127);
    let out = sys.run_ticks(1);
    close(out[0], 0.375);
    close(out[1], 0.375);
    // Sound effect volume 64/127 scales a single voice.
    sys.sndeffect_vol(100, 0, 64);
    let out = sys.run_ticks(1);
    close(out[0], 0.25 * 64.0 / 127.0 + 0.125);
}

#[test]
fn stereo_clip_keeps_channels_apart() {
    let mut sys = system();
    sys.clips.insert(
        "se100".into(),
        Arc::new(PcmClip {
            sample_rate: RATE,
            channels: 2,
            samples: [8192, 16384].repeat(1000),
            loop_start: 0,
            loop_end: 0,
            loop_start_alternate: 0,
            loop_end_alternate: 0,
        }),
    );
    sys.sndeffect_play(100, 0, 0, 127);
    let out = sys.run_ticks(1);
    close(out[0], 0.25);
    close(out[1], 0.5);
}

#[test]
fn pitch_three_reads_every_third_frame() {
    let mut sys = system();
    sys.clips
        .insert("bgm10".into(), Arc::new(clip(ramp(1000, 10))));
    sys.song_play(10, 127, 0);
    sys.song_pitch(10, 255); // normalize_pitch(255) = 3.0
    let out = sys.run_ticks(1);
    for f in 0..6 {
        close(out[2 * f], (3 * f) as f32 * 10.0 / 32768.0);
    }
}

#[test]
fn clip_rate_ratio_advances_position() {
    let mut sys = system();
    let mut c = clip(ramp(1000, 10));
    c.sample_rate = RATE / 2; // half rate: position advances 0.5 per output frame
    sys.clips.insert("bgm10".into(), Arc::new(c));
    sys.song_play(10, 127, 0);
    let out = sys.run_ticks(1);
    close(out[0], 0.0);
    close(out[2 * 2], 10.0 / 32768.0); // position 1.0
    close(out[2], 5.0 / 32768.0); // position 0.5, linear interpolation
}

#[test]
fn song_switch_fades_old_song_over_500_ms() {
    let mut sys = system();
    sys.song_play(10, 127, 0);
    sys.run_ticks(1);
    let old = sys.profile(10, ProfileType::Music).unwrap().sound_id;
    sys.song_play(11, 127, 0);
    assert_eq!(sys.current_music_id(), 11);
    // 500 ms = 22050 frames = 30 ticks; counted from the stop call.
    sys.run_ticks(29);
    assert!(sys.backend.is_exist(old));
    sys.run_ticks(1);
    assert!(!sys.backend.is_exist(old));
}

#[test]
fn stop_has_a_100_ms_minimum_fade() {
    let mut backend = Backend::new(RATE).unwrap();
    let bank = backend.add_data(constant(16384, 100_000), "x", ProfileType::SoundEffect);
    let id = backend.create_sound(bank);
    backend.start(id, 0);
    backend.stop(id, 0); // below 100 ms -> 100 ms = 4410 frames
    backend.mix_frames(4409);
    assert!(backend.is_exist(id));
    backend.mix_frames(1);
    assert!(!backend.is_exist(id));
}

#[test]
fn third_copy_of_a_bank_fades_out_the_oldest() {
    let mut backend = Backend::new(RATE).unwrap();
    let bank = backend.add_data(constant(16384, 100_000), "x", ProfileType::SoundEffect);
    let a = backend.create_sound(bank);
    backend.start(a, 0);
    backend.mix_frames(FRAMES_PER_TICK); // 16.7 ms > 10 ms
    let b = backend.create_sound(bank);
    backend.start(b, 0);
    backend.mix_frames(FRAMES_PER_TICK);
    let c = backend.create_sound(bank);
    backend.start(c, 0); // two others are audible -> oldest (a) fades over 100 ms
    assert_eq!(backend.voice_count(), 3);
    backend.mix_frames(4409);
    assert!(backend.is_exist(a));
    backend.mix_frames(1);
    assert!(!backend.is_exist(a));
    assert!(backend.is_exist(b) && backend.is_exist(c));
}

#[test]
fn restart_within_10_ms_stops_the_unpaused_copy() {
    let mut backend = Backend::new(RATE).unwrap();
    let bank = backend.add_data(constant(16384, 100_000), "x", ProfileType::SoundEffect);
    let a = backend.create_sound(bank);
    backend.start(a, 0);
    backend.mix_frames(100); // 2.3 ms
    let b = backend.create_sound(bank);
    backend.start(b, 0);
    assert!(!backend.is_exist(a));
    assert!(backend.is_exist(b));
}

#[test]
fn pause_stack_needs_matching_resumes() {
    let mut backend = Backend::new(RATE).unwrap();
    let bank = backend.add_data(constant(16384, 100_000), "x", ProfileType::Music);
    let id = backend.create_sound(bank);
    backend.set_pause(id, 1, 0);
    backend.set_pause(id, 1, 0);
    backend.set_pause(id, 0, 0);
    backend.start(id, 0); // pause stack still 1: stays paused
    assert!(backend.is_paused(id));
    backend.set_pause(id, 0, 0);
    assert!(!backend.is_paused(id));
}

#[test]
fn volume_is_clamped_to_five() {
    let mut backend = Backend::new(RATE).unwrap();
    let bank = backend.add_data(constant(16384, 1000), "x", ProfileType::Music);
    let id = backend.create_sound(bank);
    backend.set_volume(id, 9.0, 0);
    close(backend.get_volume(id), 5.0);
    backend.set_volume(id, -1.0, 0);
    close(backend.get_volume(id), 0.0);
}

#[test]
fn rate_must_be_a_multiple_of_60() {
    assert!(Backend::new(44_100).is_ok());
    assert!(Backend::new(48_000).is_ok());
    assert!(Backend::new(44_101).is_err());
    assert_eq!(Backend::new(48_000).unwrap().frames_per_tick(), 800);
}

#[test]
fn suspend_and_restore_song() {
    let mut sys = system();
    sys.song_play(10, 127, 0);
    sys.run_ticks(1);
    sys.song_suspend(10, false);
    assert_eq!(sys.current_music_id(), -1);
    assert_eq!(sys.suspend_song_id(), 10);
    let silent = sys.run_ticks(1);
    assert!(silent.iter().all(|s| *s == 0.0));
    sys.song_restore();
    assert_eq!(sys.current_music_id(), 10);
    assert_eq!(sys.suspend_song_id(), -1);
    let out = sys.run_ticks(1);
    close(out[0], 0.5);
}

#[test]
fn song_load_starts_paused_then_play_resumes() {
    let mut sys = system();
    sys.game_mode = 1;
    sys.dispatch_mode = DispatchMode::Field;
    sys.ff9_sound(code::SONG_LOAD, 10, 0, 0, 0);
    assert_eq!(sys.current_music_id(), 10);
    assert!(sys.run_ticks(1).iter().all(|s| *s == 0.0));
    sys.ff9_sound(code::SONG_PLAY, 10, 0, 0, 0);
    close(sys.run_ticks(1)[0], 0.5);
}

#[test]
fn sfx_limit_drops_the_oldest_channel() {
    let mut sys = system();
    sys.sndeffect_play(1966, 0, 0, 127);
    sys.run_ticks(1);
    sys.sndeffect_play(1966, 0, 0, 127);
    sys.run_ticks(1);
    sys.sndeffect_play(1966, 0, 0, 127);
    assert_eq!(sys.sfx_channels().len(), 3);
    sys.run_ticks(1);
    // Fourth play: three already counted, so the oldest is removed before the new one is added.
    sys.sndeffect_play(1966, 0, 0, 127);
    assert_eq!(sys.sfx_channels().len(), 3);
    assert!(
        sys.sfx_channels()
            .windows(2)
            .all(|w| w[0].start_play_time < w[1].start_play_time)
    );
    // 2295 keeps exactly one channel.
    sys.sndeffect_play(2295, 0, 0, 127);
    sys.run_ticks(1);
    sys.sndeffect_play(2295, 0, 0, 127);
    assert_eq!(
        sys.sfx_channels()
            .iter()
            .filter(|c| c.obj_no == 2295)
            .count(),
        1
    );
}

#[test]
fn sfx_stop_all_skips_environment_effects() {
    let mut sys = system();
    sys.metadata.sound_effect.insert(642, "se642".into());
    sys.clips.insert("se642".into(), constant(1000, 1000));
    sys.sndeffect_play(100, 0, 0, 127);
    sys.sndeffect_play(642, 0, 0, 127); // 642 is an environment effect id
    sys.sndeffect_stop_all(None);
    let left: Vec<i32> = sys.sfx_channels().iter().map(|c| c.obj_no).collect();
    assert_eq!(left, vec![642]);
    assert!(ext_env_obj_no(643) == 642 && ext_env_obj_no(5) == -1);
}

#[test]
fn resident_slot_replaces_and_suspends() {
    let mut sys = system();
    sys.fld_map_no = 5;
    sys.sndeffectres_play(0, 100, 0, 0, 127);
    assert_eq!(sys.sfx_res_slot(0).unwrap().obj_no, 100);
    // Same id, not suspended: map id refreshed, nothing restarted.
    sys.fld_map_no = 6;
    sys.sndeffectres_play(0, 100, 0, 0, 127);
    assert_eq!(sys.sfx_res_slot(0).unwrap().prev_snd_effect_map_id, 6);
    // A different id replaces the slot.
    sys.sndeffectres_play(0, 101, 0, 0, 127);
    assert_eq!(sys.sfx_res_slot(0).unwrap().obj_no, 101);
    // Suspend then restore on the same map replays it.
    sys.sndeffectres_suspend(0);
    assert!(sys.sfx_res_slot(0).unwrap().is_suspend);
    sys.sndeffectres_restore(0);
    assert!(!sys.sfx_res_slot(0).unwrap().is_suspend);
    sys.sndeffectres_stopcurrent();
    assert!(sys.sfx_res_slot(0).is_none());
}

#[test]
fn dispatch_decodes_codes_and_remaps_ids() {
    let mut sys = system();
    // Song id 93 is remapped to 5 (ParameterChanger, song object type).
    sys.ff9_sound(code::SONG_PLAY, 93, 0, 0, 0);
    assert_eq!(sys.current_music_id(), 5);
    // Slot bit (64) is stripped before matching: SNDEFFECTRES_PLAY with slot 1.
    sys.ff9_sound(code::SNDEFFECTRES_PLAY | 64, 100, 0, 0, 127);
    assert!(sys.sfx_res_slot(1).is_some() && sys.sfx_res_slot(0).is_none());
    // Codes are masked to 16 bits: scripts hand over negative i16 values.
    sys.ff9_sound(-12288, 101, 0, 0, 127);
    assert!(sys.sfx_channels().iter().any(|c| c.obj_no == 101));
    // Stubbed tempo codes do nothing and return 0.
    assert_eq!(sys.ff9_sound(code::SONG_TEMPO, 5, 10, 0, 0), 0);
}

#[test]
fn field_dispatch_counts_songs_and_tracks_effects() {
    let mut sys = system();
    sys.dispatch_mode = DispatchMode::Field;
    sys.ff9_sound(code::SONG_PLAY, 10, 0, 0, 0);
    assert_eq!(sys.bgm_field_song_counter, 1);
    sys.ff9_sound(code::SNDEFFECT_PLAY, 100, 0, 0, 127);
    assert!(sys.played_field_sound_effect.contains(&100));
    sys.stopall_map_sndeffects();
    assert!(sys.sfx_channels().is_empty());
    assert!(sys.played_field_sound_effect.is_empty());
}

#[test]
fn world_dispatch_chains_890_to_2728() {
    let mut sys = system();
    sys.dispatch_mode = DispatchMode::World;
    sys.ff9_sound(code::SNDEFFECT_PLAY, 890, 0, 0, 127);
    assert_eq!(sys.sfx_channels()[0].obj_no, 2728);
}

#[test]
fn song_volume_intplall_respects_cfg_sound() {
    let mut sys = system();
    sys.song_play(10, 127, 0);
    sys.settings.cfg_sound = 1;
    sys.song_vol_intplall(60, 0);
    close(
        sys.profile(10, ProfileType::Music).unwrap().sound_volume,
        1.0,
    );
    sys.settings.cfg_sound = 0;
    sys.song_vol_intplall(60, 0);
    close(
        sys.profile(10, ProfileType::Music).unwrap().sound_volume,
        0.0,
    );
}

#[test]
fn stream_play_replaces_stream() {
    let mut sys = system();
    sys.stream_play(7, 0, 0);
    assert_eq!(sys.current_song_id(), 7);
    close(sys.run_ticks(1)[0], 0.5);
    sys.stream_stop();
    assert_eq!(sys.current_song_id(), -1);
    sys.settings.sound_effect_is_mute = true;
    sys.ff9_sound(code::STREAM_PLAY, 7, 0, 0, 0);
    assert_eq!(sys.current_song_id(), -1);
}

#[test]
fn missing_clip_or_metadata_plays_nothing() {
    let mut sys = system();
    sys.song_play(999, 127, 0); // not in metadata
    assert_eq!(sys.current_music_id(), -1);
    sys.clips.remove("bgm10");
    sys.song_play(10, 127, 0); // loader error
    assert_eq!(sys.current_music_id(), -1);
    assert_eq!(sys.backend.voice_count(), 0);
}

#[test]
fn host_events_drive_the_state_machine() {
    let mut sys = system();
    let mut p = FixedProjector;
    let ev = HostEvent::RunSound {
        code: 0,
        sound: 10,
        arguments: [0, 0, 0],
    };
    assert_eq!(
        sys.apply_host_event(&ev, &mut p),
        EventSoundOutcome::Dispatched
    );
    assert_eq!(sys.current_music_id(), 10);

    let ev = HostEvent::RunSound {
        code: -12288,
        sound: 100,
        arguments: [0, 0, 127],
    };
    sys.apply_host_event(&ev, &mut p);
    assert!(sys.sfx_channels().iter().any(|c| c.obj_no == 100));

    let ev = HostEvent::SetSoundPosition {
        position: [40, 0, 0],
        volume: 64,
    };
    assert_eq!(
        sys.apply_host_event(&ev, &mut p),
        EventSoundOutcome::PositionSet
    );
    assert_eq!((sys.se_pos, sys.se_vol), (40, 50)); // 100 * 64 >> 7

    let ev = HostEvent::SetSoundObjectPosition {
        object: EntityId::new(3),
        volume: 255,
    };
    sys.apply_host_event(&ev, &mut p);
    assert_eq!((sys.se_pos, sys.se_vol), (9, 127)); // 127 * 255 >> 7 = 253, clamped
    let ev = HostEvent::SetSoundObjectPosition {
        object: EntityId::new(4),
        volume: 255,
    };
    assert_eq!(
        sys.apply_host_event(&ev, &mut p),
        EventSoundOutcome::ObjectUnknown
    );

    let ev = HostEvent::SetBattleMusicContinuation { enabled: true };
    sys.apply_host_event(&ev, &mut NoProjector);
    assert!(sys.continue_field_song_after_battle);
    let ev = HostEvent::ShowTimer { show: true };
    assert_eq!(
        sys.apply_host_event(&ev, &mut NoProjector),
        EventSoundOutcome::NotSound
    );
}

#[test]
fn hill_of_despair_doubles_effects() {
    let mut sys = system();
    for id in [2980, 2981, 2982, 2983] {
        sys.metadata.sound_effect.insert(id, format!("se{id}"));
        sys.clips.insert(format!("se{id}"), constant(1000, 1000));
    }
    sys.metadata.sound_effect.insert(2786, "se2786".into());
    sys.clips.insert("se2786".into(), constant(1000, 1000));
    sys.fld_map_no = 2928;
    let ev = HostEvent::RunSound {
        code: -12288,
        sound: 2786,
        arguments: [0, 0, 100],
    };
    sys.apply_host_event(&ev, &mut NoProjector);
    let ids: Vec<i32> = sys.sfx_channels().iter().map(|c| c.obj_no).collect();
    assert_eq!(ids, vec![2786, 2980, 2981]);
}

#[test]
fn metadata_tables() {
    let dup = SoundMetadata::index(&entries(&[(1, "a"), (1, "b")]));
    assert!(dup.is_err());
    let mut meta = SoundMetadata::default();
    meta.set_sound_effects(
        &entries(&[(1, "base1")]),
        &entries(&[(1, "ext1"), (2, "ext2")]),
    )
    .unwrap();
    assert_eq!(meta.sound_effect[&1], "base1"); // base wins
    assert_eq!(meta.sound_effect[&2], "ext2"); // extended fills gaps
    let p = meta.get_profile(2, ProfileType::SoundEffect).unwrap();
    assert_eq!(
        (p.resource_id.as_str(), p.sound_volume, p.state),
        ("ext2", 1.0, ProfileState::Idle)
    );
    assert!(meta.get_profile(2, ProfileType::Sfx).is_none());
    assert!(meta.get_profile(2, ProfileType::Music).is_none());
}

#[test]
fn music_state_graph() {
    use ProfileState::*;
    use Transition::*;
    assert_eq!(music_graph_next(Idle, Create), Some(Created));
    assert_eq!(music_graph_next(Stopped, Create), Some(Created));
    assert_eq!(music_graph_next(Played, Create), None);
    assert_eq!(
        music_graph_next(Created, StartCrossfadeIn),
        Some(CrossfadeIn)
    );
    assert_eq!(
        music_graph_next(CrossfadeIn, CrossfadeInFinish),
        Some(Played)
    );
    assert_eq!(music_graph_next(Played, Pause), Some(Paused));
    assert_eq!(music_graph_next(CrossfadeIn, Pause), Some(Paused));
    assert_eq!(music_graph_next(Paused, Resume), Some(Played));
    assert_eq!(music_graph_next(Played, Resume), None);
    for s in [Paused, Played, CrossfadeIn] {
        assert_eq!(music_graph_next(s, Stop), Some(Stopped));
    }
    assert_eq!(music_graph_next(Idle, Stop), None);
}

#[test]
fn rendering_is_deterministic() {
    let run = || {
        let mut sys = system();
        sys.song_play(10, 100, 0);
        sys.run_ticks(2);
        sys.sndeffect_play(100, 0, 0, 90);
        sys.song_vol_intpl(10, 17, 30);
        sys.song_pitch_intpl(10, 20, 100);
        sys.run_ticks(40)
    };
    let a = run();
    assert_eq!(a, run());
    assert_eq!(a.len(), 40 * FRAMES_PER_TICK * 2);
}

#[test]
fn eviction_unloads_unused_music_profiles() {
    let mut sys = system();
    for id in 20..24 {
        sys.metadata.music.insert(id, format!("m{id}"));
        sys.clips.insert(format!("m{id}"), constant(1000, 20)); // 20 frames: ends within a tick
    }
    for id in 20..24 {
        sys.song_play(id, 127, 0);
        sys.run_ticks(1);
    }
    // Four profiles are loaded and every voice finished; the next miss unloads them first.
    sys.song_play(10, 127, 0);
    assert!(sys.profile(20, ProfileType::Music).is_none());
    assert!(sys.profile(10, ProfileType::Music).is_some());
}
