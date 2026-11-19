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
| Start | Title screen (New Game / Continue / Load) | missing | `run_play` goes straight to field 70 (`main.rs` `load_game`, `--start` default 70); no title UI anywhere in `crates/` |
| Start | Continue from a save | partly | Only with `--load N` or `--load menu`; the owner's launch line has neither, so every launch is a new game. In game only game over leads to a load list |
| Start | Naming screen (`MENU(1, ...)`) | missing | `field/host.rs` handles `OpenMenu` 2, 4, 5 only; Memoria `EventService.FF9Menu_Command` case 1 `OpenNameMenu`; default names stay |
| Save | Moogle / save point save | works (plain) | `msave.png`: `--moogle-save` opens the slot list; confirm wrote `saves2/slot2.json` and showed "Saved to slot 2." |
| Save | Save slot screen | partly | 8 slots, rows read "Zidane Lv 8 17901 Gil EVT_LIND1_TN_LB_RFS_0": internal field name, no location name, play time or portraits (Steam: 10 blocks x 15) |
| Save | Save format | partly | JSON in `%APPDATA%/tantalus/saves`; the Steam `SavedData_ww.dat` is read with `--load-save`, saved to `<file>.tantalus.dat` |
| Save | Disc change (`DISCCHANGE`) | works (fixed 2026-11-19) | `PlayGame` takes `world.disc_change`: Memoria's `changetodisc<N>.png` fades in (`SceneDirector`, `DisplayPSXDiscChanges`), waits for a press and fades out, then the save screen opens (a save there starts the next disc), then the next disc's first field. From the end-of-disc-1 save: picture, save screen, field 800. Test `play_flow.rs` `the_end_of_disc_one_shows_the_picture_asks_to_save_and_starts_disc_two`; captures `play-p0/disc-pic.png` (the disc 2 picture), `disc-save.png` (the save screen), `disc-next.png` (field 800) and the slot it wrote (`map` 800) |
| Save | Game over | partly | `bd-end.png`: a "GAME OVER / Confirm: load a saved game" window over the battle, then the load screen (`battle_view.rs:1908`). No game-over screen, no return to the title |
| Field | Backgrounds, player and NPC models | works | `load13.png` (Gargan Roo, Zidane, Vivi), `menu-s2.png` (Lindblum, 7 models), `ng-t300.png` (cargo room) |
| Field | Dialog windows, speaker names, choice cursor | works | `ng-t300.png`, `load13.png`: game font and window art, speaker line; choice cursor in `dialog_view.rs` (no choice captured) |
| Field | `!` / `?` / card / here icons, ATE prompt | partly | `presentation_view.rs` draws the real `Icon Atlas` sprites; its header lists the gaps: no bubble pop-in, ATE prompt place guessed, here-icon trigger missing. ATE selection itself is script-driven and was not captured |
| Field | Widescreen field camera | partly | `widescreen.rs` ports `NarrowMapList`/pillarboxing; captures at 854x480 pillarbox narrow fields. V05-V11 widescreen/fps fixes still todo. Bars are drawn in the clear color (18, 24, 36), not black |
| Field | High frame rate | works (by code) | `smooth.rs` blends the last two logic states (Memoria `SmoothFrameUpdater_*` limits); not judged on screen |
| Field | Walk / run | works | `world.rs:1529` runs unless the walk button is held (`FieldMapActorController.Update`) |
| Menu | Main menu | partly | `m1.png`: plain dialog windows, commands on the left, a diamond pointer; no portraits, no Time/location window, no trance bar. Steam: party left, commands right |
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
| World | Vehicles: Hilda Garde 3, Invincible | missing | `set_ride` ignores control > 7; parked airships draw but cannot be flown |
| World | Sea | partly | Flat alpha layer at opacity 190 (`world_view.rs:30`); no wave or scroll |
| World | Place names, world titles | missing | G58 still open; nothing draws the continent or place name |
| Battle | Party and enemy models, backgrounds | works (replay) | `mb3.png`: Zuu, Zidane, Vivi, Freya, Cleyra trunk background |
| Battle | Live view camera | broken (suspect) | `mb1.png`, `mb2.png`, `mbc.png`: in `--live 1` 3 of 10 frames (list, attack, hit) draw no scene at all and 3 more look at sky or ground; the same battle in replay mode draws fine. Same with `TANTALUS_RASTER_CPU=1`, so not a GPU-capture artifact. Needs a look in the window |
| Battle | Command menu, lists, targeting | works | `mb1.png`: Attack/Steal/Skill/Item/Defend/Change, item list, target name window |
| Battle | ATB bars, HP/MP | works | `mb1.png` |
| Battle | Damage numbers | untested | Drawn by `battle_hud.rs:273`; the `-hit` capture had no scene |
| Battle | Trance | works | `mbt.png`: model change, Trance effect, Dyne in the menu |
| Battle | Flee | works (by code) | Hold L1+R1 (`[` `]` or PageUp/PageDown), as `BattleHUD.Unity.cs:45` |
| Battle | Victory screen | partly | `mb2.png`: one "Victory" window (EXP per member, level up, AP, gil, items, stolen); not the Steam EXP/AP and item screens; G19 captions |
| Movies | FMV | works | `fmv001.png` frame 300 decodes; the window plays movies (`live.rs` `MoviePlayer`), Start skips |
| Audio | Music, effects, PS1 renders | works (by log) | `audio-ng.log`: song 136 and field effects cued; 106 PS1 music and 2343 effect renders found in `%APPDATA%/tantalus/psx-audio`. Not listened to |
| Input | Keyboard | partly | Fixed keys (`live.rs` `KEYS`): arrows/WASD, Enter/Space/Z, Backspace/X, Q, E, Tab, P, `[` `]`; no remapping, no on-screen key help |
| Input | Controller | missing | bevy's gilrs reports "Gamepad connected" in every log, but nothing in `crates/` reads `Gamepad`; no analog movement |

## Gaps ranked

### P0: blocks a normal playthrough

Items 1, 2 and 5 were fixed 2026-11-19 (worker play-p0); 3 and 4 are still open.

1. **Disc change: fixed.** `PlayGame` runs Memoria's disc picture, a save prompt and the next disc's first field (see the table). Left: the picture is drawn over black, not over the next field's fade-in as `SceneDirector` does.
2. **World map turned off by `--world-exits`: fixed.** The play window always walks the world map; `--world-exits` is a trace aid used up first.
3. **No title screen and no Continue: open.** Every launch without `--load` is a new game; in game, only game over opens a load list. Scope: medium. Notes from play-p0 (a draft was written, then left out by a scope change; it is in the coordinator's `local/probe/play-p0/all-items-1-5.patch`): `TitleUI.OnKeyConfirm` has Continue (autosave, sound 1261, refusal 102), New Game (`ReInitStateSystem`, sound 3096, field 70) and Load Game (the load screen, sound 103); Continue can load the slot written last since there is no autosave; draw it through `menu_frame` over black like the disc screen. Files: `crates/tantalus-tools/src/play.rs`, `crates/tantalus-client/src/main.rs`.
4. **Airships cannot be flown: open.** Hilda Garde 3 and the Invincible are parked props (`set_ride` returns for control > 7). Notes from play-p0 (draft in the same patch): `ff9.cs:1659-1706` has their `s_moveCHRControl` (type 1, speed 500, radius 640 and 720) and `VehicleSpec` 8 and 9 exist, so `set_ride` can accept 8 and 9 with `fly()`; landing is `w_movementGetGetoff` cases 8 and 9 (probes at `radius * j / 5`, each within 350 of param 194); the real boarding and landing go through the world script (`w_frameSetParameter` 1 = `w_movementChange` from gEventGlobal 190, 28 = the get-off search), both stubs in `WorldEffects`; `world_view.rs` draws rides above 7 as chocobos (`CHOCOBO_TINTS[ride - 1]` would panic). Files: `crates/tantalus-tools/src/world_play.rs`, `world_script.rs`, `crates/tantalus-client/src/world_view.rs`.
5. **World map menu: fixed in part.** The main menu works on foot. Left: the world map's moogle (Square on foot, `WorldHUD` `button_mog`) and its save, which needs a world-map save and load (G39); Square on foot still calls the yellow chocobo instead.

### P1: badly wrong

6. **Live battle camera shows nothing at times.** 6 of 10 live captures miss the fight (3 empty, 3 sky or ground); replay mode is fine, CPU raster is the same. Needs a check in the window before any fix. Scope: unknown, likely small once found (live camera or scene timing in the `FightView` path). Files: `crates/tantalus-client/src/battle_view.rs`, `battle_camera_play.rs`, `battle_capture.rs`.
7. **No controller.** Steam players use a pad; gilrs is in but nothing reads it. Scope: small to medium (map `Gamepad` buttons and the left stick into the same pad bits; analog walk later). Files: `crates/tantalus-client/src/live.rs` (`KEYS`, input system).
8. **Menus are text windows, not the Steam menus.** Main menu layout reversed, no portraits, no Time/location, no Key Items or Arrange, no help lines, abilities in progress not listed, Status has one page, Card has no art. Everything works, but it does not look or read like FF9. Scope: large (NGUI layouts from the `resources.assets` atlases, screen by screen). Files: `crates/tantalus-tools/src/field_menu.rs`, `ability_menu.rs`, `system_menu.rs`, `crates/tantalus-client/src/menu_view.rs`.
9. **Save screen is a debug list.** 8 slots labelled with internal field names, no play time, location or portraits. Scope: small for the labels (`FF9TextTool` location names, play time), medium for the Steam layout. Files: `crates/tantalus-tools/src/play.rs` `slot_labels`, `field_menu.rs`.
10. **Victory and game over screens are single windows.** Steam has the EXP/AP screen per member, the items screen, the card line, and the game-over picture. Scope: medium. Files: `crates/tantalus-client/src/battle_view.rs` (`results_lines`, `ended`), `battle_hud.rs`.
11. **Naming screen missing.** `MENU(1, character)` is ignored, so no names can be chosen. Scope: small to medium (a text-entry screen over the menu art). Files: `crates/tantalus-game/src/field/host.rs` (`OpenMenu`), `crates/tantalus-tools/src/field_menu.rs`.
12. **Side content unverified.** Jump rope, Hot and Cold, chocobo digging, races, swordplay, Mognet letters and the ending sequence were not driven in the window, and the trace forced 25 battle wins. Scope: one audit pass per minigame with capture scripts. Files: `local/probe/play-audit/cap.sh` as a starting point.

### P2: polish

13. Tetra Master text uses a placeholder bitmap font, 4:3 only (`crates/tantalus-client/src/card_view.rs`, `card_draw.rs`). Scope: small to medium.
14. Config rows that do nothing: Movement, Window color, Vibration, ATB Mode, Movie Volume (`crates/tantalus-tools/src/system_menu.rs`, consumers in `live.rs`, `audio.rs`). Scope: small each.
15. World map: flat sea (`world_view.rs`), no place or continent titles (G58), arrival at the place cell centre instead of the exit spot (`play.rs` `start_world`). Scope: small to medium each.
16. Field presentation: bubble pop-in, ATE prompt position, here-icon trigger (`presentation_view.rs` header); pillarbox bars in the clear color instead of black (`live.rs` `ClearColor`). Scope: small.
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
| G19 missing result-screen captions | yes | the result screen is one window today (gap 10) |
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
| G61 auto-flight, ship flapping and tilt | blocked | airships not flyable (gap 4); Blue Narciss and the gold chocobo do move now |
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
