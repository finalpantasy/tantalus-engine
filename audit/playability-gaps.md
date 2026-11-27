# Play window playability gaps (2026-11-19, main fc958f7)

Question: does the real play window (`tantalus-client play`, the `play-now.exe` build) play like the Steam game for a normal playthrough? This is a player's-eye audit. The new-game trace route reaching the ending is a regression check only; it uses aids the window does not have (`--stub-objects`, `--wall-regions`, `--skip-talk`, `--region-talks`, 25 `--win-battles` forced wins, `--heal-after-battle`), so it does not prove the window can be played through.

## Method

- Binary: `target/debug/play-now.exe` from fc958f7, not rebuilt. All runs were headless: `capture-play`, `capture-battle` (replay and `--live 1`), `capture-world`, `capture-card`, `capture-movie`. No window was opened.
- Story states: the 30 trace worlds `story33-worlds/probe/route/w1..w30.json` wrapped as play-window save slots (`{"format":1,"map":field_map_no,"position":null,"ticks":0,"world":W}`) and loaded with `--save-dir DIR --load N`. Loading works: slot 13 enters Gargan Roo 955 with the party and the dialog drawn.
- Scripted input: `--hold FROM-TO:HEXMASK` (pad bits from `tantalus_tools::play::pad`: START 0x8, UP 0x10, DOWN 0x40, CONFIRM 0x20000), `--auto-confirm N`, `--moogle-save`, `--shop N`, `--enter-trance`, `--party-hp`.
- Captures, logs and the save slots are under `local/probe/play-audit/` (not committed); the PNG names below refer to `local/probe/play-audit/caps/`.
- Where nothing could be captured, the state comes from the code and says so.

## Summary table

| Area | Feature | State | Evidence (short) |
|---|---|---|---|
| Start | Title screen (New Game / Continue / Load) | done (ui3, 2026-11-27) | `TitleUI`: a launch without `--load`, `--load-save` or `--start` opens the title (`title_menu.rs`, `play.rs` `open_title`). The picture is the skin's: PS1 skin the discs' title (resident file 60, ten 224-line strips), Steam skin `title_bg` with `title_logo` over it (`ui_skin.rs` `title_picture`, `title_view.rs`); the Continue / New Game / Load Game window draws over it in both skins. The PS1 skin draws the disc's own title menu (fix2, 2026-11-27): NEW GAME, CONTINUE and the copyright line are TIMs 11 to 13 of the title file (`ui_skin.rs` `ps1_title_words`), CONTINUE opens the save list (the PS1 manual: "A menu of saved game data will appear"); the cursor starts on CONTINUE when there is a save. Left: the PS1 words' places are by eye (centered under the logo, not measured on a PS1); the Steam logo's place is by eye; no title song |
| Start | Continue from a save | done (ui3, 2026-11-27) | Title Continue loads the slot written last (no autosave in the play window; sound 1261, refusal 102 with no save); Load Game opens the load list, cancel goes back to the title. `--load N` / `--load menu` still skip the title |
| Start | Naming screen (`MENU(1, ...)`) | missing | `field/host.rs` handles `OpenMenu` 2, 4, 5 only; Memoria `EventService.FF9Menu_Command` case 1 `OpenNameMenu`; default names stay |
| Save | Moogle / save point save | works (plain) | `msave.png`: `--moogle-save` opens the slot list; confirm wrote `saves2/slot2.json` and showed "Saved to slot 2." |
| Save | Save slot screen | partly (fix2, 2026-11-27) | 8 slots, rows read "3  Zidane Lv 8  L. Castle/Base Level": the game's location name (`loc_name.mes`, or the world map area a save made there kept), the gil of the slot under the cursor beside Load/Save (play-bugs PB9). Left: play time, portraits (Steam: 10 blocks x 15) |
| Save | Save format | partly | JSON in `%APPDATA%/tantalus/saves`; the Steam `SavedData_ww.dat` is read with `--load-save`, saved to `<file>.tantalus.dat` |
| Save | Disc change (`DISCCHANGE`) | works (fixed 2026-11-19) | `PlayGame` takes `world.disc_change`: Memoria's `changetodisc<N>.png` fades in (`SceneDirector`, `DisplayPSXDiscChanges`), waits for a press and fades out, then the save screen opens (a save there starts the next disc), then the next disc's first field. From the end-of-disc-1 save: picture, save screen, field 800. Test `play_flow.rs` `the_end_of_disc_one_shows_the_picture_asks_to_save_and_starts_disc_two`; captures `play-p0/disc-pic.png` (the disc 2 picture), `disc-save.png` (the save screen), `disc-next.png` (field 800) and the slot it wrote (`map` 800) |
| Save | Game over | partly (ui3, 2026-11-27) | `GameOverUI`: a lost fight fades to the game over picture over black (PS1 skin: the discs' own picture, file 11 of the resident group; Steam skin: the install's `gameover` texture), Confirm goes to the title screen (`GoToTitleScene`). A field script's `GAMEOVER` (`FLOW_STATE_GAMEOVER` -> `EventService.OpenGameOver`) opens the same screen since fix2 (2026-11-27): the field stops, the picture fades in over 1 s (`GameOverUI.duration`), Confirm fades it out and the title opens (`play.rs` `tick_game_over`, `game_over_view.rs`); a fight resolved without the live view that loses does the same. Capture: fix2 `hunt/out/go-ps1`, `go-steam` (`--start 1213 --set-global 0:u16:5060`: the Alexandria rescue's timer has run out) |
| Field | Backgrounds, player and NPC models | works | `load13.png` (Gargan Roo, Zidane, Vivi), `menu-s2.png` (Lindblum, 7 models), `ng-t300.png` (cargo room) |
| Field | Dialog windows, speaker names, choice cursor | works | `ng-t300.png`, `load13.png`: game font and window art, speaker line; choice cursor in `dialog_view.rs` (no choice captured) |
| Field | Dialog and battle hand cursor | works (2026-11-19, ui1; skins ui2) | The cursor comes from the UI skin (`crates/tantalus-client/src/ui_skin.rs`): PS1, the white glove of the discs' icon sheet (24 by 14 PS1 pixels, drawn 116 UI units wide, nearest-neighbor); Steam, `cursor_hand_choice` of the Icon Atlas (`FF9UIDataTool.IconSpriteName[1]`) at `PointerManager.PointerSize` (114 by 62). The dialog hand sits where `UIPointer.SetPosition` puts it (label left + width / 4 + `Dialog.DefaultOffset` 36). Captures `local/probe/ui2/`. The battle hand tips are placed by eye, 10 UI units left of the text, not from `BattleHUD` offsets |
| Field | `!` / `?` / card / here icons, ATE prompt | partly | `presentation_view.rs` draws the real `Icon Atlas` sprites; its header lists the gaps: no bubble pop-in, ATE prompt place guessed, here-icon trigger missing. ATE selection itself is script-driven and was not captured |
| Field | Widescreen field camera | partly | `widescreen.rs` ports `NarrowMapList`/pillarboxing; captures at 854x480 pillarbox narrow fields. V05-V11 widescreen/fps fixes still todo. Bars are drawn in the clear color (18, 24, 36), not black |
| Field | High frame rate | works (by code) | `smooth.rs` blends the last two logic states (Memoria `SmoothFrameUpdater_*` limits); not judged on screen |
| Field | Walk / run | works | `world.rs:1529` runs unless the walk button is held (`FieldMapActorController.Update`) |
| Menu | Main menu | partly (ui4, 2026-11-27) | Laid out after `MainMenuUI`: party panel on the left (portrait of the skin, moved 5 PSX pixels right in the back row per `FrontRowPosition`/`BackRowPosition`, half transparent at 0 HP; name, LV, HP and MP as now/max with `DisplayCharacterDetail` colors), command list on the right with the skin's hand cursor, a Time and gil window under it (`DisplayTime`: hh:mm:ss, color per 100 hours; gil), and the place name along the bottom (`LocationInfoPanel`). Both skins use the same layout (party left, commands right). Captures `local/probe/ui4-*/m-ps1.png`, `m-steam.png`. Placement of panels, rows and text columns is by eye in PSX pixels (not measured from the prefab). No trance bar: `MainMenuUI` code never touches one (only `StatusUI` has `_tranceSlider`), so none is drawn; a prefab-only bar was not checked. Left: Key Items and Arrange commands, help line, status icons, the screen-transition tween |
| Menu | Items | partly | `m2.png`: list with counts, use on a member; no Key Items tab, no Arrange, no help line |
| Menu | Abilities | partly | `m2.png`: "Magic / Support" tabs; Zidane at Cleyra lists only Flee; abilities still being learned from gear (AP progress) are not listed |
| Menu | Equip | partly | `m2.png`: slots and stats; no item list preview captured, no portrait or ability info |
| Menu | Status | partly | `m2.png`: stats and gear only; no portrait, abilities, element or status pages |
| Menu | Order | works (plain) | `m3.png`: front/back row and member swap |
| Menu | Card | partly | `m3.png`: text list of the collection; no card art, no card detail view |
| Menu | Config | partly | `m3.png`: 13 settings; music/sound volume, cursor memory, ATB wait, battle speed, battle camera and message speed are wired; Movement, Window color, Vibration, ATB Mode and Movie Volume are shown but read nowhere outside the menu |
| Menu | World map menu | partly (fixed 2026-11-19) | START opens the main menu on the world map on foot only (`WorldHUD.OnKeyMenu`, `ControlNo == 0`), not on a vehicle, over the Chocograph list or while the world script holds the menu off; Items, Abilities, Equip, Status, Order, Card and Config work on the party; Save is refused (`MainMenuUI` has no Save; the world map saves through its moogle, Square on foot, `button_mog`, which is not ported). Test `play_flow.rs` `the_main_menu_opens_on_the_world_map_on_foot`; no capture: `capture-play` cannot start on the world map, and the window draws the menu on the world camera (`menu_view.rs`), which was not seen on screen |
| Shops | Item, weapon, synthesis | partly | `mshop.png`: buy/sell/exit, owned/worn counts and the party comparison, synthesis list with ingredients; text only, no item icons or help. Tables read from Memoria's `Data/Items/*.csv` (`shop_menu.rs:46`), not the install |
| Minigames | Tetra Master | partly | `mcard.png`: real card art and board; placeholder bitmap-font text ("TETRA MASTER - CHOOSE 5 CARDS", "YOU LOSE"), 4:3 only, no coin toss or card-pick screen captured |
| Minigames | Jump rope, Hot and Cold, races, swordplay | untested | Script driven; the Hot and Cold timer UI exists (`presentation_view.rs`); none captured |
| Minigames | Chocobo riding, Chocograph | partly | `world_play.rs` rides 1-5, gold air (6); the Chocograph list opens (`chocograph_menu.rs`) |
| World | Walking, terrain, minimap | works | `mw.png`: terrain, sky, Zidane, minimap; random encounters roll per step (`world_play.rs:647`) |
| World | Reaching the world map with the owner's launch line | works (fixed 2026-11-19) | `play` and `capture-play` always walk the world map; `--world-exits` locations are used in order for the next exits and the world map runs once they are used up. Test `play_flow.rs` `world_exits_run_out_into_the_world_map` |
| World | Arrival point on the world map | partly | `start_world` puts the player at the centre of the first matching place cell, not at the reference exit spot |
| World | Vehicles: Blue Narciss | partly | Boards next to the moored ship (`world_play.rs` `board`) |
| World | Vehicles: Hilda Garde 3, Invincible | partly (ui3, 2026-11-27) | `set_ride` takes 8 and 9: the airships fly with `w_movementPlaneOperation` and their own `VehicleSpec`, Square next to a parked one boards it, Square in the air lands it (`w_movementGetGetoff` cases 8 and 9) and it stays parked where it came down; `world_view.rs` draws the flown ship. Left: boarding and landing are the Square shortcut the Blue Narciss uses, not the world script's (`w_frameSetParameter` 1 and 28 are still stubs); no landing or take-off animation. The ride comes back from gEventGlobal 190 when the world map starts (`w_frameMapConstructor` -> `w_movementChange`), so a save made in an airship or on a chocobo loads riding it (fix2, 2026-11-27; test `play_flow.rs` `a_world_map_save_in_an_airship_loads_flying`) |
| World | Sea | partly | Flat alpha layer at opacity 190 (`world_view.rs:30`); no wave or scroll |
| World | Place names, world titles | missing | G58 still open; nothing draws the continent or place name |
| Battle | Party and enemy models, backgrounds | works (replay) | `mb3.png`: Zuu, Zidane, Vivi, Freya, Cleyra trunk background |
| Battle | Live view camera | fixed 2026-11-19 (ui1) | A command taken during the opening camera plunge ran the action camera before the plunge was done; its snap path gave eye == look == 0, a degenerate camera, so the list, attack and hit frames drew nothing (`lb-*.png`). The C# `PHASE_ENTER` waits until `SFX.GetCameraPhase() == 1` (`SFX.cs:1606`); `FightView` now holds the ATB, the menu and the HUD for the plunge length (`CameraDirector::opening_ticks`, capped at 900 ticks). `lc-*.png`/`ld-*.png` (slot 1, `--live 1`): all 10 frames show the fight. Not verified: in the window; an enemy action mid-track at another moment could reach the then unported anchor 9 stored point (ported 2026-11-27, fix1, play-bugs PB2) |
| Battle | Command menu, lists, targeting | works | `mb1.png`: Attack/Steal/Skill/Item/Defend/Change, item list, target name window |
| Battle | ATB bars, HP/MP | works | `mb1.png` |
| Battle | Damage numbers | untested | Drawn by `battle_hud.rs:273`; the `-hit` capture had no scene |
| Battle | Trance | works | `mbt.png`: model change, Trance effect, Dyne in the menu |
| Battle | Flee | works (by code) | Hold L1+R1 (`[` `]` or PageUp/PageDown), as `BattleHUD.Unity.cs:45` |
| Battle | Victory screen | works (ui3, 2026-11-27) | `BattleResultUI` states: the EXP and AP page with a row per member (portrait from the install's `Face Atlas`, level, EXP, Next Lv), Confirm counts the EXP up at the C# rate (`EXPMinTick`/`EXPDefaultAdd`/`EXPTickMax`) with "Level Up!" and the 683 chime as a level is passed, each learned ability's panel in turn (1043) and 103 at the end; Confirm to the gil page, Confirm counts the gil (one a frame), items, stolen items and the card by name; in both skins. Left: no item icons, status icons or EXP/gil tick loops (105, 109), no item-full dialog, Garnet always has the long-hair portrait, the PS1 skin draws the Steam portraits (PS1 ones not found on the discs) |
| Movies | FMV | works | `fmv001.png` frame 300 decodes; the window plays movies (`live.rs` `MoviePlayer`), Start skips |
| Audio | Music, effects, PS1 renders | works (by log) | `audio-ng.log`: song 136 and field effects cued; 106 PS1 music and 2343 effect renders found in `%APPDATA%/tantalus/psx-audio`. Not listened to |
| Audio | Music across fields, world map and battles | fixed 2026-11-19 (ui1) | The world map had no song of its own (`ff9.w_musicPlay` was not ported), so a field song kept playing there; world fights took their song from the field table, so they played no battle song; and the victory fanfare (it loops) held the field song back until it ended, which it never did. Now: `AudioCue::WorldSong` (`w_musicChoice` over `w_musicSet`, on entering the world map and on each change of ride), `FieldReady` (`HonoluluFieldMain.cs:141-151`: restore or drop the song set aside, stop the world song when the field plays none), world fights use `WldBtlEncountBgmMetaData` (battle 359 on 9000 plays song 0), and `BattleLeft` is sent when the result screen closes (`BattleResultUI.Hide`), stopping the end song at once. Tests: `play_audio` director tests, `play_flow.rs` `the_world_map_plays_its_song_and_a_chocobo_its_own` (real data: song 69 on foot, 100 on a chocobo). `capture-play --audio-log --audio-wav` now prints `now playing music N`. Not verified: by ear, the live result-screen close in a window, and field 852's track (no save near it). The fanfare "sounding wrong" was not found: song 5 is the PS1 render of sequence 1 (26.5 s, loop from 8.5 s); needs an ear check against the Steam clip (`TANTALUS_NO_PSX_AUDIO=1`). Also seen: a JSON slot saved on map 9000 (play-audit slots 5-7) loads 9000 as a field, not the world map, so its fights use the field table |
| Input | Keyboard | partly | Fixed keys (`live.rs` `KEYS`): arrows/WASD, Enter/Space/Z, Backspace/X, Q, E, Tab, P, `[` `]`; no remapping, no on-screen key help |
| Input | Controller | missing | bevy's gilrs reports "Gamepad connected" in every log, but nothing in `crates/` reads `Gamepad`; no analog movement |

## Gaps ranked

### P0: blocks a normal playthrough

Items 1, 2 and 5 were fixed 2026-11-19 (worker play-p0); 3 and 4 are still open.

1. **Disc change: fixed.** `PlayGame` runs Memoria's disc picture, a save prompt and the next disc's first field (see the table). Left: the picture is drawn over black, not over the next field's fade-in as `SceneDirector` does.
2. **World map turned off by `--world-exits`: fixed.** The play window always walks the world map; `--world-exits` is a trace aid used up first.
3. **Title screen and Continue: done (ui3, 2026-11-27).** See the Start rows. `TitleUI.OnKeyConfirm`: Continue (the slot written last, sound 1261, refusal 102), New Game (`ReInitStateSystem`, sound 3096, field 70), Load Game (the load screen, sound 103). Game over goes to the title. Test: `tests/play_flow.rs` `the_title_screen_starts_a_new_game_or_continues`.
4. **Airships fly: partly done (ui3, 2026-11-27).** `world_play.rs` `set_ride` accepts 8 and 9 (flying, gEventGlobal 190 kept in step), `board_airship`, `land_airship` (`w_movementGetGetoff` cases 8 and 9: topographs 45, 46, 52 and the game's kept-clear spots refused; probes at `radius * j / 5`, the first ray's last at `radius`; the spot within 350 of the ground under the ship); `world_view.rs` draws rides 8 and 9 as the ships. Test: `tests/world_play.rs` `airships_board_fly_and_land`. Left: see the Vehicles row; the script's own boarding (`w_frameSetParameter` 1 = `w_movementChange` from gEventGlobal 190, 28 = the get-off search) is still a stub in `WorldEffects`.
5. **World map menu: fixed in part.** The main menu works on foot. Left: the world map's moogle (Square on foot, `WorldHUD` `button_mog`) and its save, which needs a world-map save and load (G39); Square on foot still calls the yellow chocobo instead.

### P1: badly wrong

6. **Live battle camera shows nothing at times.** Fixed 2026-11-19 (ui1): the fight now waits out the opening camera (`PHASE_ENTER`). Anchor 9's stored point (the effect's target centre) is ported and the reaction camera follows each effect's flag word since 2026-11-27 (fix1, play-bugs PB2): Steal and Jump no longer aim at the caster's empty base position. Files: `crates/tantalus-client/src/battle_camera_play.rs`, `crates/tantalus-assets/src/pc/battle_camera.rs`.
7. **No controller.** Steam players use a pad; gilrs is in but nothing reads it. Scope: small to medium (map `Gamepad` buttons and the left stick into the same pad bits; analog walk later). Files: `crates/tantalus-client/src/live.rs` (`KEYS`, input system).
8. **Menus are text windows, not the Steam menus.** Main menu screen: done in part (ui4, 2026-11-27, see the Menu / Main menu row: portraits, Time/gil, place, hand cursor); no Key Items or Arrange, no help lines, abilities in progress not listed, Status has one page, Card has no art. Everything works, but it does not look or read like FF9. Scope: large (NGUI layouts from the `resources.assets` atlases, screen by screen). Files: `crates/tantalus-tools/src/field_menu.rs`, `ability_menu.rs`, `system_menu.rs`, `crates/tantalus-client/src/menu_view.rs`.
9. **Save screen is a debug list.** 8 slots labelled with internal field names, no play time, location or portraits. Scope: small for the labels (`FF9TextTool` location names, play time), medium for the Steam layout. Files: `crates/tantalus-tools/src/play.rs` `slot_labels`, `field_menu.rs`.
10. **Victory and game over screens: done in the main (ui3, 2026-11-27).** Portraits, the EXP count-up, the ability-learned panels, the gil count, the card name and the game over picture, in both skins (`battle_view.rs` `ResultCount`, `results_draw`; `battle_hud.rs` `ResultDraw`; `ui_skin.rs` `faces`, `game_over_picture`). Left: see the Victory screen and Game over rows. Harness proof: `play --battle idle --hurt 1` loses a fight on slot 9.
11. **Naming screen missing.** `MENU(1, character)` is ignored, so no names can be chosen. Scope: small to medium (a text-entry screen over the menu art). Files: `crates/tantalus-game/src/field/host.rs` (`OpenMenu`), `crates/tantalus-tools/src/field_menu.rs`.
12. **Side content unverified.** Jump rope, Hot and Cold, chocobo digging, races, swordplay, Mognet letters and the ending sequence were not driven in the window, and the trace forced 25 battle wins. Scope: one audit pass per minigame with capture scripts. Files: `local/probe/play-audit/cap.sh` as a starting point.
13. **PS1 and Steam UI skins: partly (ui2, 2026-11-19).** Skin plumbing done: `--ui-skin ps1|steam`, `TANTALUS_UI_SKIN` or `ui_skin = ps1|steam` in the `--config` file, PS1 by default; the disc folder from `--psx-discs`, `TANTALUS_PSX_DISCS` or `psx_discs = DIR`. The PS1 art is the discs' resident UI file (`tantalus_psxaudio::ui_art`: dialog font with its glyph table, icon sheet, window sheet), cached as `%APPDATA%/tantalus/psx-ui/ui-art.bin` (`TANTALUS_PSX_UI_DIR`); without cache and discs the Steam art is drawn and one `ui skin:` line is logged. Both skins share one layout path (`DialogAssets` with the skin's glyph source and window sprites), so dialogs, menus, the battle HUD and the hand cursor all switch. Not verified against a real PS1 frame (no PS1 screenshot to compare): the gray window grain palette (row 7 of the window sheet), the frame piece and its 8-pixel corners, the font baseline row (10) and the stand-in apostrophe (left mark of `”`; the font has none). Dialog tails: the Steam skin draws the window atlas pointer (`Dialog.SetTailPosition` sprite names, `dialog_pointer_top*/down*`, a right-side tail drawing `*left`), joined to the frame by eye; the PS1 skin still draws a marker (its tail is a notch drawn into the frame, pieces at x 152-216, y 18-31 of the window sheet, not decoded). Battle HUD: the PS1 skin has its own placement (`battle_hud.rs` `Layout::ps1`, fitted in PS1 pixels to the PS1 font, not measured from a PS1 frame) and the PS1 damage digits and `Miss` of the window sheet; the Steam placement is unchanged. Left: the PS1 tail, `Attack`/`Defend` sit close in the PS1 command window, then victory, level-up, game over, menus, save, shop and title. Scope: large.
14. **Dagger's join level in the Ice Cavern era: rule matches, cause open (ui1, 2026-11-19).** The `JOIN` leveling (`FieldWorld::join_with_leveling`) matches `ff9play.FF9Play_Change` / `FF9Play_GetAvgLevel` (average of the other roster members with `info.party != 0`, never lower). Slot 1 (field 300) has no experience for Dagger, so she is Lv 1 while the others are Lv 5-6; the save comes from the trace route, which skips most fights. Next: check which `JOIN` of Garnet before field 300 passes `enableLeveling` and whether the trace ran it after the others leveled.
15. **Field 852 dark bottom band: fixed (ui3, 2026-11-27).** The field camera now draws into the 224 rows the game shows (`FieldMap.PsxFieldHeightNative`, `PSXCameraAspect.LateUpdate`) of the 240-row frame (`window.rs` `fit_field_frame`, `field_rows`): background, models and effects are cut at the viewport, so nothing draws in the 8 rows above and below it, and those rows and the bars are black (`ClearColor`). Harness: `--start 852`, Zidane walked down; before, a 12-pixel band in the clear color at the bottom; after, black above and below the viewport. Left: at 852's bottom scroll limit the art stops 2 rows short of the viewport (black there too); the C# frame was not compared.

### P2: polish

13. Tetra Master text uses a placeholder bitmap font, 4:3 only (`crates/tantalus-client/src/card_view.rs`, `card_draw.rs`). Scope: small to medium.
14. Config rows that do nothing: Movement, Window color, Vibration, ATB Mode, Movie Volume (`crates/tantalus-tools/src/system_menu.rs`, consumers in `live.rs`, `audio.rs`). Scope: small each.
15. World map: flat sea (`world_view.rs`), no place or continent titles (G58), arrival at the place cell centre instead of the exit spot (`play.rs` `start_world`). Scope: small to medium each.
16. Field presentation: bubble pop-in, ATE prompt position, here-icon trigger (`presentation_view.rs` header); pillarbox bars are black since 2026-11-27 (ui3). Scope: small.
17. Keyboard: no remapping and no key help; L2/R2 have no keys (`live.rs` `KEYS`). Scope: small.
18. Shops: no item icons or help lines; tables come from Memoria CSVs rather than the install (`shop_menu.rs:46`). Scope: small (UI), medium (tables from the install).
19. Stale doc: the `live.rs` header still says actors are drawn as markers and the client has no font. Scope: trivial.

## Memoria bug-fix rows (`audit/memoria-bug-fixes.md`) by player visibility

| Row | Player-visible? | Note |
|---|---|---|
| G15 AutoAttack with replaced Attack | yes, rare | Berserk/Confuse on a character whose Attack is replaced |
| G16 default targeting of single/multi spells | yes | the cursor's starting side on every such spell |
| G17 "Miss" instead of "Death" | yes, rare | Avenger-style commands |
| G18 caster status SA when caster is target | barely | numbers only |
| G19 missing result-screen captions | done (ui2) | English captions on both result pages; other languages not wired |
| G20 Steiner Magic Sword too long | yes | every Magic Sword once Vivi and Steiner are together |
| G22 Petrify/Doom countdown display | yes | the countdown number over the unit |
| G28 Scan vs Zombie | rare | |
| G29 Freya Jump retargeting | no | exploit only |
| G30 flee exception | no | crash path, no Tantalus equivalent seen |
| G31 Eidolon replica mix | yes, rare | Garnet with two Eidolons queued |
| G32 hidden characters in the battle menu | yes | scripted fights with hidden members |
| G35 Cover position | yes | Steiner/Freya Cover |
| G37 stats after the battle menu | maybe | only if Tantalus opens a menu mid-battle |
| G39 world-map save soft-lock | blocked | no world-map save yet: the main menu works on the world map, the moogle save does not (gap 5) |
| G40 friendly Mu after the ore | yes, side quest | no friendly-monster logic on the world map |
| G56 Eiko summons Madeen | yes, story | Madain Sari fight |
| G58 Mist Continent title stuck | blocked | no world titles drawn (gap 15) |
| G59 navi mode reset | yes, small | the minimap now exists, so this row is no longer blocked |
| G60 minimap bugs | yes, small | the minimap is drawn now (`world_minimap.rs`); the row's "no minimap" note is stale |
| G61 auto-flight, ship flapping and tilt | open | airships fly now (gap 4); auto-flight, flapping and tilt are not drawn |
| G62 field teleports (1207, 768, 1554, Fossil Roo) | yes | partial; 768 and 1554 still open |
| G64 Steiner transparent (#896) | yes | not located |
| G66 Tetra Master analog stick | no for now | there is no analog input (gap 7) |
| G69 moogle anim on Mognet letters | yes | |
| G70 text opcode alignment | yes | `[u]`, `[XTAB=0]`, negative `[FEED]` in dialogs |
| V02-V04, V09, V10 | yes | field camera or art on single fields and the ending |
| V05-V08, V11 | yes, widescreen/fps | the window defaults to widescreen and high fps, so these show by default |
| V12 layer order (~45 fields), V13 missing objects, V14 overlaps, V15 SPS depth | yes | most visible of the open rows: wrong-looking fields across the game |
| V23 Mognet machine, jump-rope and chocobo-mount animations | yes | |
| UI rows G21 double input, G57 choice cancel | check | keyboard repeat and fast cancel on choices were not tested |

## Not covered

- Anything that needs ears or a real window: music quality, PS1 versus Steam sound picks, frame pacing at 144 Hz, input feel.
- The ending sequence (fields 3000-3009) and credits in the window.
- Steam save import beyond reading the code path.
