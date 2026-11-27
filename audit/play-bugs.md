# Play window bugs (scripted hunt, 2026-11-26)

Found by driving the real play window (`tantalus-client play`) with the play harness
(`play --harness DIR --script ... --battle POLICY`, `crates/tantalus-client/src/play_harness.rs`):
hidden window, virtual clock, a PNG every N frames and one JSON line a frame (mode, map, field,
song, player position, pad bits, menu, dialog text, battle machine state, wall time). Every slot
of the play-audit saves (1 = field 300, 4 = 704, 5-7 = world map 9000, 8-13 = 1006, 1017, 1101,
1103, 1150, 955) ran the same 80-second script twice: 60 Hz with interpolation (the default) and
`--fps-cap 30 --fps-interp off` (vanilla). The script walks all four ways, exits fields, taps
Confirm (talk), opens and closes the main menu; battles use one policy per slot (Attack, second
row = Steal or a spell, fourth row = Item, L1+R1 flee, or the three in turn) and close the
result screen.

Evidence is under the hunt's `local/` (copied to the coordinator's `local/probe/hunt1/`):
`hunt/out/<run>/frames.jsonl` and `f*.png`, runs `p*` (before the fixes) and `x*` (after
fix 1), `e4*` (every frame of slot 4's first second). Scripts: `hunt/run.sh`, `pair.sh`,
`batch.sh`, `field.txt`; checks: `hunt/tools/analyze.py` (timeline, song cues, wall-time
spikes, stuck input, no walking, blank frames), `cmpt.py` (both modes by game tick),
`clear.py` (clear color showing).

Severity: P0 blocks play, P1 clearly wrong, P2 polish. FPS: whether the bug depends on the
frame-rate setting.

## Ranked

| id | sev | status | FPS | bug |
|---|---|---|---|---|
| PB1 | P1 | fixed | yes (interp off) | Vanilla 30 fps judders: about half the frames show no motion and the next jumps two ticks |
| PB2 | P1 | fixed | no | Battle camera looks at empty sky or fills the screen with one model during actions |
| PB7 | P1 | fixed | no | Music stops for good after a story battle that kept the field song (no battle song, no win pose) |
| PB3 | P2 | fixed | no | A button held across a battle or movie end is a new press on the first field tick |
| PB4 | P2 | fixed | partly | First field after launch shows the clear color instead of the background for 5 to 20 frames |
| PB5 | P2 | matches (rates); pacing unverified | no | Random battles take about a minute (one enemy action ran 12 s) |
| PB6 | P2 | fixed | no | Battle HUD (party panel, enemy list) is up during the opening camera |

### PB1: vanilla 30 fps judders (fixed)

- Repro: any slot, `--fps-cap 30 --fps-interp off` in the real window. The harness clock has
  no jitter, so it shows only the rounding half: with the clock in whole nanoseconds every 30 Hz
  tick came one frame late (`hunt/out/s1-lo/frames.jsonl`, `ticks` one behind `t`). The jitter
  half is the unit test `smooth::tests::a_capped_vanilla_display_ticks_once_a_frame`.
- Cause: the fixed-step loop took a tick only when `elapsed >= step`
  (`crates/tantalus-client/src/live.rs:641` on 2cd19aa). `FrameLimiter::wait`
  (`smooth.rs:413`) keeps frames on a fixed 1/30 s grid, so each frame is the tick length plus
  or minus a few microseconds. After anything that empties the bank (`live.elapsed = 0.0`
  after a load hitch at `live.rs:668`, a battle end at 613, a movie end at 585) every frame a
  hair short misses its tick and the next runs two. A simulation of grid-locked frames with
  20 us to 1 ms jitter gives 742 no-tick and 742 two-tick frames in 3000. With interpolation the
  drawn alpha follows the banked time, so the default mode hides it.
- Fix: `smooth::tick_due`: without interpolation the tick goes to the display frame nearest its
  time (up to half a frame early, the debt carried), so 30 fps runs exactly one tick a frame
  and the logic rate is unchanged; with interpolation only 1 us of clock rounding is allowed.
  The battle view's 15 Hz loop (`battle_view.rs:1988`) gets the same rounding slack. The
  harness feeds `1 / hz` as an exact f64 so 60 Hz and 30 Hz runs tick on the same frames.
- Check: with the exact clock both modes give identical logic per game tick up to the first
  battle in every slot (`cmpt.py`: 0 differences in slots 1, 4, 5, 6, 10, 11, 12, 13; slots 7, 8
  and 9 differ by one frame after a battle, where the result screen closed on a different
  display frame).

### PB2: battle camera on empty sky (fixed)

- Repro: slot 5, `--battle row2` (Steal): `hunt/out/p5-hi/f003840.png` ("Stole Tent!" over
  sky only), `f003720.png` (camera under Freya's Jump); slot 1 opening
  `hunt/out/p1-hi/f001620.png` (sky); slot 4 `hunt/out/s4-hi/f005400.png` (Vivi's hat fills
  the right third). Same frames in both modes.
- Cause: not the `0xC0` anchor. A camera trace of the same run (fix1 `hunt/out/c5-hi`,
  stage ticks 521 to 548) showed the reaction camera, set 4, playing for Steal and Jump: eye
  at knee height in front of Zidane's base position (anchor `0xE1`: id 1, base `0x20`,
  height `0xC0`) looking up past his head while the effect had already run him to the enemy.
  `CameraDirector::command` started set 3 or 4 for every party effect. In the plugin
  `SeqStateUpdateStart` (`u0010_SeqStateUpdate.cs`) reads the effect's flag word
  (`unk_18004F020`, copied to `dword_18032318C`): with bit `0x8000` it skips `SfxStateReset`
  and `SfxTriggerReaction` altogether, and `SfxTriggerReaction` (`u0009_EvalOrbitTrack.cs`)
  plays nothing for a code whose high nibble is `0xF` and the plugin's own camera table
  (`unk_180063860`, `+44` Garnet, `+48` the others) for nibbles 1 and 7. Steal (effect 200),
  Jump (393) and the weapon attacks (100 on) are `0x83FF`; 118 of the first 520 effects
  have bit `0x8000`.
- Fix: `battle_camera_play::reaction_camera` applies the flag word: no reaction for `0x8000`
  or nibble `0xF`, the camera gate (`RecomputeGate323268`: off with Battle Camera: Fixed
  unless flag `0x8`), the table sets for nibbles 1 and 7
  (`special_effect::plugin_camera_set`, read from the player's DLL), set 3 or 4 otherwise.
  A `0x8000` effect also draws no side bit. Anchor 9's stored point is now ported: it is the
  request's `trgcpos` (`SfxResolveActors` keeps it at offset 56 of the state block that
  `SfxStateReset` copies to `unk_18021FFA0`, so `word_18021FFD8`/`FFDC`), the mean base x and z
  of the effect's targets (`CameraScript::effect_center`).
- Check: the same run after the fix (`hunt/out/f5-hi`, side by side in
  `hunt/out/pb2-before-after.png`) keeps the default camera through Steal and Jump: Zidane at
  the enemy, then home, with "Stole Tent!"; Freya's Jump seen from the default camera.
  Unit tests `reaction_cameras_follow_the_effects_flag_word`,
  `anchor_9_stands_at_the_effects_target_centre`, `reads_a_camera_set_of_the_built_in_table`.
- Left as data-driven, not a bug: the slot 1 opening starts in the sky and sweeps down to the
  party (set 0 of the scene, `p1-hi` f1560 to f1770); the slot 4 shot over Freya's shoulder
  with her arm filling the right third is the Potion's (effect 8, flags `0x300`) reaction set 3
  (`s4-hi` f5280 to f5430). Not checked against the Steam game. Not modelled: the caster's
  statuses in `SfxTriggerReaction` (Trance takes table `+52`/`+56`; two statuses stop it), the
  scene's `FixedCamera1/2` flags and the context flags at `+64`.

### PB3: held button carried across a battle or movie end (fixed)

- Repro: slot 9, any policy: the Confirm that closes the result screen is still held on the
  first field tick (`hunt/out/x9-lo/frames.jsonl` field tick 136, `held` 0x22000).
- Cause: `PlayGame::tick` makes presses from `held & !self.previous`
  (`crates/tantalus-tools/src/play.rs:1618`) and `previous` is the pad of the last tick before
  the battle, so a button held while the battle view or movie read the pad becomes a new press
  (talk, examine) on return. Unity's `GetKeyDown` sees no new press there.
- Fix: `PlayGame::carry_held`, called by the play window when the battle view finishes and
  when a movie ends or is skipped.

### PB7: field song stopped after a no-win-pose battle (fixed)

- Repro: slot 11, `--battle row4`: battle 297 (`CY_E021B`, flags special, no_win_pose,
  no_run_away, message_event) plays no song of its own, so field song 98 goes on; when the
  result screen closes the song fades out and stays off, through field 1105
  (`hunt/out/y11-lo/frames.jsonl`: `song` -1 from t1841).
- Cause: `AudioDirector::finish_battle_return` (`crates/tantalus-tools/src/play_audio.rs:496`
  on 2cd19aa) stops the current song unless a script set `SONGFLAG`, which is
  `battle.ff9ShutdownStateBattleResult`. It missed the other way `BTL_CONTI_FLD_SONG` is set:
  `battle.cs:416-446` sets it when a battle without the win pose ends (won, ended by its
  script or fled) and `Info.FieldBGM` is true, which `btlseq.SetupBattleScene` copies from
  `IsPlayFieldBGMInCurrentBattle`, true when `RequestPlayBattleEncounterSongForField` found no
  song to play (`BattleSwirl.cs:88-98`). The same branch (`battle.cs:379-381`) runs
  `ManageBattleSong` (the fanfare) only with the win pose; the director played the fanfare for
  every victory.
- Fix: `FightReport::no_win_pose` from `SB2_HEAD` flag 0x10, carried on
  `AudioCue::BattleEnded`; the director keeps `field_song_in_battle` and `keep_field_song` as
  the C# does and plays no fanfare for a victory without the win pose. Unit test
  `a_battle_without_its_own_song_or_win_pose_leaves_the_field_song_playing`.
- Check: the same run after the fix keeps song 98 past the result screen and into field 1105
  in both modes (`hunt/out/z11-hi`, `z11-lo`).

### PB4: first field without its background (fixed)

- Repro: slot 4, every frame (`hunt/out/e4-lo`, `e4p-lo`, `e4i-lo`, all 30 Hz harness): the
  clear color with Zidane drawn, background missing, from frame 5 or 9 to frame 11, 21 or 27.
  At 60 Hz (`e4-hi`) none. Wall-time dependent, so not a logic bug. Reproduced by fix1 on the
  build before the fix (`hunt/out/e4-base`: frames 6 to 9 are 96% clear color).
- Cause: Bevy compiles render pipelines in the background and skips a draw whose pipeline is
  not ready yet (`PipelineCache`); the background layers' material pipelines were not ready
  for the first frames, the actor pipeline was. At 60 Hz the first frame happened to block
  long enough to cover it.
- Fix: the play window sets `RenderPlugin::synchronous_pipeline_compilation`, so a pipeline is
  compiled before the frame that first needs it (`live.rs`). The cost is a one-time stall the
  first time a pipeline is used: frame 3 of the first field took 934 ms instead of a few
  missing frames (`e4-wip3` wall times), inside the load.
- Check: the same 40-frame 30 Hz run three times after the fix (`hunt/out/e4-wip3`,
  `e4-wip3-2`, `e4-wip3-3`): no frame with the clear color (`tools/clear.py`); frame 7 before
  and after in `hunt/out/pb4-before-after.png`.

### PB5: long random battles (ATB rates match; not a bug as far as checked)

- Repro: slot 8 (`hunt/out/x8-lo`, Zuu, Attack only): 63 s to win; turns 16 to 32 s apart. The
  machine log shows the ATB frozen during the opening camera (6 s, by design,
  `PHASE_ENTER`) and one command running 12 s (t1232 to t1592) with the others queued
  (`sel_mode` 1) while Zuu's spell plays.
- Compared with the C# and the Steam defaults (fix1), each the same:
  - Tick rate: one ATB add per battle tick, 15 a second (`HonoluluBattleMain.UpdateBattleFrame`,
    `BattleTPS = 15`); the live view runs 15 battle ticks a second (`battle_view.rs`).
  - Add per tick: `at_coef * ATBSpeedFactor` (`HonoluluBattleMain.cs:459`) with
    `GetATBCoef` 10 for Battle Speed 1 and factor 4 (`atb::atb_step`, `base_atb_coef`,
    `DEFAULT_ATB_SPEED_FACTOR`); the saves keep the Steam defaults (`FF9CFG`: `btl_speed` 1,
    `atb` 1 = Wait, `camera` 0), so 40 a tick, 600 a second.
  - Gauge size: `(60 - Dexterity) * 160` (`btl_para.GetMaxATB`): Zidane's 5760 in slot 8
    (Dexterity 24) fills in 9.6 s, as the machine log shows (`p5760/5760`).
  - Stops: no ATB until the opening camera ends (`IsNativeEnableAtb` reads `_commandEnable`,
    which `FF9BMenu_EnableMenu(true)` sets at the end of `PHASE_MENU_ON`); with Wait, none
    while a member's menu is past the command window; a unit with a queued command
    (`sel_mode != 0`) does not fill.
- So a turn is the 9.6 s fill plus the time its command waits behind the others and plays.
  The commands' own lengths come from the effect and sequence data and were not timed against
  a Steam capture of the same battle; that is what is still unverified.

### PB6: HUD during the opening camera (fixed)

- Repro: slot 4, `hunt/out/s4-hi/f000600.png` (t300, 11 ticks into battle 101): party panel and
  enemy list are drawn while the opening camera sweeps the cave wall. `battle_view.rs:2053`
  holds back only the command menu.
- Cause: in the C# the party panel, the command menu and the enemy (target) list are all
  children of `AllMenuPanel` (`BattleHUD.UI.AllMenuPanel.cs`). `BattleHUD.Show` turns it off
  (`BattleHUD.Scene.cs`), and only `FF9BMenu_EnableMenu(true)` at the end of
  `PHASE_MENU_ON` (`battle.cs:122`) turns it on; `PHASE_MENU_OFF` turns it off again at the
  battle's end (`battle.cs:135`).
- Fix: `HudDraw::panels` (`Overlay::hide_panels`): the panels are not drawn while the opening
  camera plays (`PHASE_ENTER`) nor once the battle has ended (the result screen and its
  banner); damage numbers, the banner and the result window still draw.
- Check: slot 4 after the fix (fix1 `hunt/out/h4-hi`, `hunt/out/pb6-after.png`): f600 to f930
  (the opening) show no HUD; f990 on show the party panel and the enemy list.
  The end-of-battle case was checked in the fix1 rerun: `hunt/out/g9-hi/f004170.png` (result
  screen) and `hunt/out/g7-hi/f002850.png` (escape) show no party panel.

### PB8: empty "Escaped" panel after a flee (fixed)

- Repro: slot 7 (flee), fix1 `hunt/out/g7-hi/f002850.png` (t1425, about 1 s after the flee):
  an "Escaped" title and an empty result window sit over the battle until it fades.
- Cause: in the C# a flee clears the bonus (`BattleHUD.GoToBattleResult` ->
  `btl_sys.ClearBattleBonus`), so `BattleResultUI.Show` takes `InitialNone`: `AllPanel` is
  off and the screen hides itself after 0.3 s. No title and no window are shown.
  `battle_view.rs` `results_lines` gave escapes an "Escaped" page instead.
- Fix: the result overlay is not built for `escape` / `party_flee` outcomes; the battle still
  holds and closes by itself as before.
- Check: unit tests and clippy; not rerun in the harness.

## Checked and fine

- Songs across transitions (play-test notes 2 and 4, fixed in 0f35086): field to world map plays
  the world song (slot 1: 60 then 69); battle song then the fanfare, and the field or world song
  returns when the result screen closes (slots 7, 8, 9); field to field keeps or replaces the
  song (slots 4, 10, 12).
- Field 955 (slot 13) asks for music 136, which the Steam `MusicMetaData` does not have and
  `FF9Snd.ParameterChanger` does not remap (it remaps only 93, 137 and 78): silence matches.
- Battle 297 (slot 11) plays no battle song by its data: the field song goes on.
- Menus open and close on the field and the world map; flee ends the battle and the world song
  returns (slot 7).
- No frame-time spikes outside loads: transitions cost 0.3 to 1.6 s (battle load about 0.7 s,
  world map load about 1.5 s); steady frames p50 3 to 9 ms, p95 under 20 ms at 60 Hz.

## Not covered

- The real window with vsync and the real audio device (the harness has neither).
- Movies, Tetra Master, shops and the naming screen (no slot reached them).
- Battles past the 80-second script (most were still running at the end).
