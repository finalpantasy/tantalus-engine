# New-game chain (milestone G3)

Goal: from a new game (`fldMapNo` 70, zeroed state), follow each field change until the first interactive field. Source: `Global/Event/Engine/EventEngine.Initialize.cs:51-64` (`NewGame` sets `fldMapNo = 70`, zeroes `gEventGlobal`), `Global/ff9/FF9DBAll.Events.cs` (map number to event archive name).

## Tool

`tantalus-trace chain --bundle <install>/StreamingAssets/p0data7.bin --event-db .reference/Memoria/Assembly-CSharp/Global/ff9/FF9DBAll.Events.cs --atan-table <ratan_tbl.bin> --max-fields 15 --ticks 4000`

It runs each field's archive under the orchestrated `FieldDirector` (each entry's tag-0 function, requested tags spawned on demand, as in `tantalusctl --orchestrate`), stops a field at its first `MAPJUMP` (`world.field_transitions`), and carries `FieldWorld` into the next field. The EventDB table is read from the pinned Memoria source at run time and is not copied into the repository. `tantalus-trace disasm --bundle <p0data7.bin> --event <NAME>.eb [--entry N]` prints decoded instructions.

## Result on 2026-11-18, ninth run (battles fought)

`chain --battles audit/battle-scene-ids.tsv --character-data <Memoria Data/Characters>` now fights each `StartBattle` with `tantalus_tools::field_battle` instead of skipping it. `tantalus-trace fight --battle N [--party 0,9,10]` runs one battle and prints the report as JSON. Each battle runs the scene's enemy scripts (`EnemyAiHooks`) against the legacy battle machine. The party is the new-game level-1 stats with each character's default equipment (`CharacterParameters.csv` -> `DefaultEquipment.csv`, weapon power and armor defence; stat bonuses are not applied), and every party member attacks the first living enemy.

| Battle | Scene | Outcome | Ticks |
|---|---|---|---|
| 336 | `EVT_BATTLE_TH_E001` | Victory | 346 |
| 338 | `EVT_BATTLE_TH_E002` | Victory | 490 |
| 337 | `EVT_BATTLE_TH_E003` | Script ends it with result 5 (`BTL_RESULT_INTERRUPTION`) | 375 |
| 335 | `EVT_BATTLE_TH_E004` | Victory | 2227 |
| 334 | `EVT_BATTLE_TH_E005` | Victory | 359 |
| 302 | `EVT_BATTLE_EF_E006` | Victory | 2021 |
| 301 | `EVT_BATTLE_EF_E007` | Victory | 790 |
| 295 | `EVT_BATTLE_BR_E008` | Victory | 365 |

Before these fixes, all eight ran to the 20,000-tick cap. The changes:

1. **Event phase.** `Phase::Event` and `Phase::MenuOn`, plus `MenuOffSeq::Event`, port `PHASE_EVENT`, `PHASE_MENU_ON` and `SEQ_MENU_OFF_EVENT` (`battle.cs:307-320`, `426-441`). `Battle::battle_code` applies `SetBattleData` 32 to 35 (`btl_scrp.cs:774-825`). Scenes with header flag 1 (`SpecialStart`) start in the event phase.
2. **Scripts every frame.** `BattleHooks::frame` runs the battle scripts once per machine tick, before `BattleMain`, as `ServiceEvents` does. Before, they ran only when an enemy's gauge filled, so the tag-1 loops that end battles hardly ran.
3. **Load timing.** `SceneOverlay::load_frames` reads `load_status` as 0 for the first frames. `BattleLoadLoop` sets `LOAD_INITNPC` and `LOAD_INITCHR` after the scripts start. Objects that entry 0 spawns then pass their load waits in the same frame as entry 0. EF_E006's HP watcher on entry 0 `Return`s for good (`ip = nil`) if entry 2 has not registered its unit yet.
4. **Unit identity.** `unit_view` gives enemies `slot_no = 4 + index` and `enemy_id = index` (`btl_id = 16 << index`). TH_E001's end check reads member 70 and never fired while every enemy read 0.
5. **SFX callback.** `QueueSpecialAttack` sets `gEventGlobal[199] |= 16`, which stands in for SFX callback 112. TH_E003 waits on it.
6. **Script unit writes.** `BattleHooks::unit_writes` applies the members the scripts wrote right after the script pass, before `BattleMain`, as `SetCharacterData` writes into `BTL_DATA`. A write to any member marks the whole unit view dirty, so applying it after the tick restored the HP from before that tick's damage. In EF_E006 that undid nearly every hit on the cage. The runner drives `LegacyBattleAdapter` directly, as `battle_run` does.
7. **Member extraction.** `EventEngine.OperatorExtract` always takes the top three stack entries. A mask that is not a list, or an operand that is a member, reads the last member target. The VM rejected `SL3.m57 == SL1.m57` (a dead unit's loop, which now runs every frame). On main at `a0c773a`, the `--battle-run-all` sweep gives 474 victories, 34 defeats and 54 errors; with these changes it gives 510, 38 and 0. Of the 18 scenes that no longer end in victory there, 9 friendly-monster scenes (WM_14xx to WM_16xx) now end as their scripts say, with `SetBattleData` 33 = 7 (enemy flee). TH_E005 and WM_9900 to WM_9903 stall at the tick cap, and four CW_R scenes end in defeat. That harness leaves `load_status` at 0 and does not start special scenes in the event phase; TH_E005 disables the ATB and then waits on the load bits.

Still open: equipment stat bonuses; battle global memory starts fresh instead of sharing the field's `gEventGlobal`; the win-pose path (`SEQ_MENU_OFF_VICTORY_WITH_ENEMY`) is not ported; battle results do not feed back into field HP or rewards.

## Result on 2026-11-18, eighth run (created objects, random, prompts)

Same command as the seventh run, with `--ticks 12000`, `--max-fields 110`, `--hold 115:16,207:16` and `--choices 52:66=1,64:127=1,202:242=1,202:242=0@2100`.

The new game runs 110 fields (the cap) with zero VM errors. After the second act it plays the sword fight, the cockpit scenes and King Leo's finale, then the airship crash: Evil Forest, the crash site rooms, and the ATE scenes in between. Scenario goes from 1155 to 2100; the next step is Baku's fight on map 202.

| Map | Archive | Ticks | Next | Scenario |
|---|---|---|---|---|
| 64 | `EVT_ALEX1_TS_SWD_BTL` | 2478 | 150 | 1600 (button prompts, 49 rounds) |
| 68/64/67 | `TS_ROYAL_1` / `TS_SWD_BTL` / `TS_STAGE_2C` | 350 / 960 / 3451 | 64, 67, 60 | 1600 (battle 335 skipped) |
| 60 | `EVT_ALEX1_TS_COCKPIT` | 825 / 769 | 60, 69 | 1600 (battle 334 skipped) |
| 69 | `EVT_ALEX1_TS_ROYAL_2` | 1750 | 209 | 1900 |
| 209/201/206 | `EVT_DOWNSHIP_BT_CF1_0` / `CCP_0` / `FRT_0` | 1136 / 538 / 1153 | 201, 206, 251 | 1900 to 2005 (ATE tutorial) |
| 250/251 | `EVT_EFOREST1_EF_ENT_0` / `FR1_0` | 326 / 1413 | 251, 207 | 2010 to 2030 (battles 302, 301 skipped) |
| 200-208 | `EVT_DOWNSHIP_BT_*` | 62 to 1413 | | 2040 to 2100 |

Engine fixes behind this run, each from the Memoria source:

- Only entry 0 starts at field load. Every other entry starts when `NEW`, `NEW2` or `NEW3` creates it; `DirectorStep::initialized` reports those sources. Before this, map 64 at entrance 327 registered exits that its script creates only in another branch.
- System variable 0 is `Comn.random8()`. It read 0, and map 64 entry 20's retry loop for a new prompt never ended.
- `FieldWorld::begin_field` also clears actors, the control actor and exit arrivals, as a field change destroys every object. Old actors stayed talkable on the next field.

Explorer changes in `tantalus-trace`:

- Button prompts: `PROMPT_KEYS` maps (map, text) to the key the script checks. Map 64's "Press [n]!" is text 112 + n for byte 46 = n (Left, Right, Triangle, Down, Cross, Up, Circle, Square), and map 206's ATE tutorial holds text 352 until Select is held. A prompt window is answered with its key, never closed with Confirm.
- Choices can depend on the story step: `MAP:TEXT=N@SCENARIO` applies from that scenario on. Baku's "Anytime!" starts his fight only from scenario 2100; before that the answer is "Hold on".
- An exit that takes control three times and keeps the player in the field (map 206 to the forest: "too tough on my own") is dropped for that visit, and the explorer tries another.

Open items: battles are still skipped; name text variables are empty; the tick limit is 12000 because the sword fight runs about 2400 ticks.

## Result on 2026-11-17, seventh run (object order, exit position, floors)

Same command as the sixth run, with `--route` gaining `153>150`, `--waypoints` gaining `154@-807:-5081`, and `--max-fields 60`.

The new game now runs 40 fields with zero VM errors, past the castle guard (map 150), through the castle and the Pluto Knight towers, then back to the theater ship: Garnet joins, the Steiner chase plays out, and the party reaches the second act on stage (map 65). Scenario goes from 1155 to 1600. Acts 2A and 2B play through (maps 65, 59, 66) to the royal seats (68). After that the explorer returns to the castle and loops there, with no scenario write until `TS_ROYAL_2` (map 69, 1900), which is reached from the cockpit (60).

| Map | Archive | Ticks | Ops | Next | Scenario |
|---|---|---|---|---|---|
| 153/150 | `AC_H2F` / `AC_GUARD` | 304 / 776 | | 150, 153 | 1155 (guard scene) |
| 154 | `EVT_ALEX1_AC_ENT_2F` | 258 | 12,538 | 158 | 1155 (ground floor exit) |
| 158-160 | `AC_OUT_E` / `OUT_C` / `OUT_L` | 62 / 438 / 482 | | 159, 160, 162 | 1155 |
| 162-166 | `AC_TOWER_L1` to `L5` | 62 to 662 | | 163 to 55 | 1155 (spiral tower, height-gated exit) |
| 55 | `EVT_ALEX1_TS_ORCHE` | 670 | 17,131 | 54 | 1400 |
| 54 | `EVT_ALEX1_TS_UPPER` | 933 | 21,820 | 51 | 1420 |
| 51 | `EVT_ALEX1_TS_CARGO_3` | 2919 | 148,002 | 53 | 1500 (Garnet) |
| 53 | `EVT_ALEX1_TS_MEET_1` | 1304 | 36,939 | 56 | 1500 |
| 56/57 | `TS_ENGIN_UP` / `TS_ENGIN` | 443 / 318 | | 57, 58 | 1500 |
| 58 | `EVT_ALEX1_TS_STORAGE` | 1116 | 45,587 | 65 | 1600 (battle 337 skipped) |
| 65 | `EVT_ALEX1_TS_STAGE_2A` | 1080 | | 59 | 1600 (waits for Cancel or Special) |
| 59 | `EVT_ALEX1_TS_UNDER` | 602 | | 66 | 1600 |
| 66 | `EVT_ALEX1_TS_STAGE_2B` | 3841 | | 68 | 1600 (waits for an idle frame) |
| 68 | `EVT_ALEX1_TS_ROYAL_1` | 350 | | 64 | 1600 |

Engine fixes behind this run, each from the Memoria source:

- Objects step in creation order. `Obj` appends to the tail of `activeObj` (`Obj.cs:32-45`) and `ProcessCode` walks from the head, so an object created by `InitializeObject` runs after the ones already alive. `FieldScene` now keeps a rank per object and steps in rank order. Map 150 had read byte 26 before the entry that sets it, and its stage stuck at 15.
- `MJPOS` (`DoEventCode.cs:2200`) is implemented. The player's position is projected onto the region's first edge, clamped to 0..256, and stored in `sMapJumpX/Z` (system variables 10 and 11). Since the VM hands it to the host at once, the facing test that follows in the same function reads the new values. Before this, every exit script that compares the player's facing with the exit direction read 0 and refused.
- `AFRAME` (0x3D) sets an actor's first and last animation frame, `StartTurn` resets them to 0/255, and an animation that ends holds its last frame. The rope climb in map 115 depends on it.
- A field start clears regions, windows, pending requests, shared scripts, object members and deleted objects from the last field.
- Idle animations loop. With no animation playing, member 7 (`animFrame`) counts up and wraps at the default clip length, as the reference's idle clip restarts. Map 66 entry 2 waits for frame 13 before its part of the act. Real clip lengths are not loaded yet.

Explorer changes in `tantalus-trace` (they drive the player; the game rules stay in the engine):

- Facing: toward an exit's `MJPOS` point while in the region, and toward each target the player jumps to.
- Exit choice: the explorer records where each exit led and picks the one whose destination has the fewest visits. This ends the 153⇄154 and 153⇄64 loops.
- Floors: walkmesh heights now apply, and a jump keeps the actor on its own floor. A target that only another floor covers is reached by walking the mesh triangle by triangle (breadth-first over shared edges), so map 154's ground floor and its exit to 158 are reachable from the upper floor. An actor standing past the mesh edge starts from the nearest triangle.
- An exit region that does not fire after its dwell is retried on each other floor under it. Map 164's spiral tower leaves upward only above height 12000.
- Region targets use a point `IsInQuad` actually contains. Map 164 region 2's vertex average falls outside it.
- After 300 ticks with no control and no new text, the cutscene taps add Cancel and Special to Confirm. Western builds test script key masks raw (`ETb.KeyOn` swaps Confirm and Cancel only for the Japanese layout), and map 65 entry 3 waits for `KeyOn(0x10000)` or `KeyOn(0x80000)`.

Open items: name text variables are empty ("Captain , do you climb..."); battles are skipped; scene object order is not saved in snapshots yet; the walk facing in `world.rs` (`128 + atan2(dx,-dz)`) has not been checked against `Angle2` (0 = −z, 64 = west).

## Result on 2026-11-17, sixth run (object lifecycle, sequences, walkmeshes)

`tantalus-trace chain ... --character-data <Memoria>/Memoria.Patcher/StreamingAssets/Data/Characters --game-data <install>/x64/FF9_Data --mes-map <Memoria>/Assembly-CSharp/Global/Event/Engine/EventEngineUtils.cs --start 70 --ticks 4000 --explore --confirm-after 30 --route 100>101,101>102,102>103,103>105,105>106,106>115 --skip-talk 103:8 --hold 115:16 --choices 52:66=1 --waypoints 116@2000:-137/1400:-137/800:-137/-339:1000/-339:1800/-339:2500/1000:5000/2500:8000/3300:10800/3400:11000 --max-fields 20`

The new game now runs 20 fields with zero VM errors, from the opening through the Alexandria streets, the steeple and the rooftops, backstage, the play, and the sword fight, into the castle (map 153 to 154). It stops at the field limit, not a stall.

| Map | Archive | Ticks | Ops | Next | Scenario |
|---|---|---|---|---|---|
| 70 | `EVT_ALEX1_TS_OPENING` | 1290 | 3,889 | 50 | 0 |
| 50 | `EVT_ALEX1_TS_CARGO_0` | 1511 | 106,745 | 52 | 1000 (battle 336 skipped) |
| 52 | `EVT_ALEX1_TS_MEET_0` | 1470 | 98,242 | 100 | 1000 |
| 100 | `EVT_ALEX1_AT_STREET_A` | 1974 | 144,868 | 101 | 1000 |
| 101 | `EVT_ALEX1_AT_STREET_B` | 693 | 34,114 | 102 | 1000 |
| 102 | `EVT_ALEX1_AT_STREET_C` | 318 | 8,805 | 103 | 1000 |
| 103/104 | `CENTER` / `TICKET` | 812 / 569 | | 104, 103 | 1000 to 1150 (ticket stamped; two harmless return trips) |
| 103 | `EVT_ALEX1_AT_CENTER` | 303 | 45,639 | 105 | 1150 |
| 105 | `EVT_ALEX1_AT_BACK_STR` | 1798 | 52,071 | 106 | 1152 (Puck) |
| 106 | `EVT_ALEX1_AT_TSS` | 422 | 17,112 | 115 | 1152 |
| 115 | `EVT_ALEX1_AT_SENTOU` | 2372 | 141,529 | 116 | 1155 (ladder climb, rope) |
| 116 | `EVT_ALEX1_AT_ROOF` | 3119 | 337,300 | 61 | 1155 |
| 61 | `EVT_ALEX1_TS_STAGE_BK` | 2762 | 70,459 | 62 | 1155 |
| 62 | `EVT_ALEX1_TS_STAGE_1A` | 941 | 37,497 | 63 | 1155 (battle 338 skipped) |
| 63 | `EVT_ALEX1_TS_STAGE_1B` | 1751 | 88,580 | 64 | 1155 |
| 64 | `EVT_ALEX1_TS_SWD_BTL` | 62 | 3,187 | 153 | 1155 |
| 153 | `EVT_ALEX1_AC_H2F` | 62 | 6,989 | 154 | 1155 |

Fixes, each checked against Memoria:

1. **Object lifecycle (`DELETE` 0x1C, `NEW` 0x07).** `TerminateEntry` disposes the whole object, so every function of that entry stops, not only the caller. `FieldWorld` keeps a deleted-object set (cleared per field). `FieldScene::step` skips VMs whose object is deleted, and `NEW` restores it. `NEW` with uid 0 means the entry's own id. Before this, map 105 stopped with "set position: unknown actor 6".
2. **Sequences (`STARTSEQ` 0x43, `WAITSEQ` 0x44, `ENDSEQ` 0x45).** A shared entry now runs as its own VM against the caller's object (`FieldScene::add_shared_script`, `VM::with_own_object`), and a new sequence replaces the old one. Inside a sequence, `DELETE 255` deletes the sequence itself, not the object it moves.
3. **Local uids 250 and 255 in requests.** uid 250 resolves to the control actor (`LegacyEventHost::control_object`). A sync request waits on its target, not on the caller (the old wait deadlocked). A request for a function that already ran to its end reruns it, and a request for an object that never spawns releases the caller.
4. **Axis order of `POS3`/`DPOS3` (0xA1/0xAD) and `SETVY3` (0xE2).** The second word is negated height, the third is floor z (`SetActorPosition(po, a1, -a2, a3)`, `SetupJump`). The VM had them swapped.
5. **Same-step `WaitAnimation`.** A `PlayAnimation` and a `WaitAnimation` in the same VM step now wait, where before the wait fell through. Map 115 ran out of instruction budget at entry 4356 byte 460 before this.
6. **Walkmeshes.** The chain reads `EventEngineUtils.eventIDToFBGID` at run time and loads each field's `<background>.bgi` from `p0data11`-`p0data19` (`bundled_field::load_field_navigation`). Map 116 polls `BGI` triangles of the player (entries 4 to 15) and failed on its first tick without one.
7. **Scripted player.** `--hold MAP:MASK` holds pad bits for a field (map 115's ladder reads up = 16). `--waypoints MAP@X:Z/...` places the player at floor points in order once regions and talks are done, snapped to the nearest walkmesh triangle when off the mesh. On map 116 the scene waits for Vivi's x <= 900 (stage 4), z >= 2300 (stage 9), and x > 3000 with z > 10300 (entry 16). The explorer also talks to objects again after the scenario counter changes.

Known gaps: Puck's line "So your name's , huh?" shows an empty name (the text variable for Vivi's name is not filled). Battles are still skipped. The control actor's height reads 32768 in several fields; this matches the reference, because `POS3` stores the negated -32768.

## Result on 2026-11-17, fifth run (self-object facing)

Same command as the fourth run. Movie frame counts (Kimi, `d9c6c41`) now make movies last their real length, so fields 70 and 52 take more ticks.

| Map | Archive | Ticks | Ops | Outcome |
|---|---|---|---|---|
| 70 | `EVT_ALEX1_TS_OPENING` | 1,290 | 3,889 | Movie, then `ChangeField(50)` |
| 50 | `EVT_ALEX1_TS_CARGO_0` | 1,549 | 116,186 | Battle 336 skipped, `ChangeField(52)` |
| 52 | `EVT_ALEX1_TS_MEET_0` | 1,502 | 99,946 | Baku's quiz answered, `ChangeField(100)` |
| 100 | `EVT_ALEX1_AT_STREET_A` | 1,491 | 88,705 | Opening scene finishes; player control; the explorer steps into the exit region to 107 |
| 107 | `EVT_ALEX1_AT_GATE` | 154 | 19,553 | Explorer steps into the exit back to 100 |
| 100 | `EVT_ALEX1_AT_STREET_A` | 6,000 | 899,319 | Free roam, no scripted progression without a route |

The map 100 stall was entry 19's turn loop (offsets 1039-1061): `DIRE(getvobj(255, 3) + 2)`, wait one frame, repeat while `getvobj(255, 3) < 162`. It turns the object itself; it is not a player-input gate. Two bugs kept it from ending:

1. **Uid 255.** `EBin` reads object members through `GetObjByUID`, and `GetUIDFromLocalUID` maps 255 to the running object (`gCur`), 250 to the controlled object and 251-254 to the party. The VM passed 255 through, and the host created an empty actor 255, so the loop always read 0. The VM now replaces 255 with its own entity; the field host maps 250 to the controlled actor.
2. **Facing scale.** `DIRE`/`TURNA` store `angle << 4` (12-bit fixed point) and `getvobj` member 3 returns `fixed >> 4 & 255`. The host stored and returned the shifted value. `ActorState::facing` is now the 0-255 scale everywhere, which matches the walking code.

With both fixed, the turn completes, Map byte 26 reaches 0, and the street sequencer runs to user control. Map 100 has three exit regions (tag 2 `ChangeField` 101, 107, 114). From here the new game is free roam, so continuing the chain needs a route: which exits to take and which characters to talk to.

Known gap: `FieldHost` applies a VM step's host events after the step, so a read later in the same step sees the state before the step. The reference applies each opcode at once. Scripts that write and read in one frame without a wait would differ.

## Result on 2026-11-16, fourth run (dialog signals and choices)

`tantalus-trace chain ... --character-data <Memoria Data/Characters> --game-data <install>/x64/FF9_Data --mes-map .reference/Memoria/Assembly-CSharp/Global/Event/Engine/EventEngineUtils.cs --choices 52:66=1 --confirm-after 30 --explore --ticks 6000`

| Map | Archive | Ticks | Ops | Outcome |
|---|---|---|---|---|
| 70 | `EVT_ALEX1_TS_OPENING` | 35 | 124 | Movie, then `ChangeField(50)` |
| 50 | `EVT_ALEX1_TS_CARGO_0` | 1,549 | 116,186 | Dialogue, lantern region, battle 336 skipped, `ChangeField(52)` |
| 52 | `EVT_ALEX1_TS_MEET_0` | 828 | 48,722 | Baku's quiz answered, `ChangeField(100)` |
| 100 | `EVT_ALEX1_AT_STREET_A` | 6,000 | 358,330 | First Alexandria field; runs without errors, next stall not diagnosed yet |

Field 52 stalled twice before reaching its exit:

1. **Dialog signals.** Entry 3 shows text 67 with `WindowAsync` and polls system variable 8 (`GetDialogProgression`, `ETb.gMesSignal`) until it is non-zero. The reference sets it from `[SIGL=n]` tags as the text displays (`DialogBoxSymbols.OnSignal`: a negative value increments, any other replaces it, at most one per event tick) and from `SETSIGNAL` (0xE3). `FieldWorld` now holds `mes_signal`, answers system variable 8 from it, and applies `SETSIGNAL`. When a window shows a text, the text's signals (`FieldWorld::dialog_signals`, filled by the caller) are queued and applied one per tick. Approximation: the reference raises each signal when the typewriter reaches its tag, so text speed, `[WAIT]` and page breaks are not modeled.
2. **Choices.** Text 66 is Baku's question (`[CHOO]`, two options). Answer 0 ("kidnap Queen Brahne") sends the scene into a joke loop; answer 1 ("kidnap Princess Garnet") moves on. System variable 9 (`GetDialogChoice`, `ETb.GetChoose()`) now reads `FieldWorld::dialog_choice`. `--choices MAP:TEXT=N` sets the answer when the scripted confirm closes that choice window (default 0, the initial cursor).

Field text comes from the install at run time. Memoria loads `EmbeddedAsset/Text/<lang>/Field/<zone>.mes` with zone `EventEngineUtils.eventIDToMESID[map]` (map 50, 51 and 52 use zone 2). The same `2.mes` name exists once per language in `resources.assets`, so the path is resolved through the Unity `ResourceManager` container in `mainData` (`tantalus_assets::pc::resource_container`, format 15: `vector<pair<string, PPtr>>`). `--language` picks the folder (default `us`).

## Result on 2026-11-16, third run (regions, scripted player, battle yield)

`tantalus-trace chain ... --character-data <Memoria Data/Characters> --confirm-after 30 --explore --ticks 6000`

| Map | Archive | Ticks | Ops | Outcome |
|---|---|---|---|---|
| 70 | `EVT_ALEX1_TS_OPENING` | 35 | 124 | Movie `0x103`, then `ChangeField(50)` |
| 50 | `EVT_ALEX1_TS_CARGO_0` | 1,549 | 116,186 | Dialogue, lantern region + Confirm, battle 336 (skipped as a win), then `ChangeField(52)` |
| 52 | `EVT_ALEX1_TS_MEET_0` | 6,000 | 442,053 | 15 dialog windows closed; stalls (next investigation) |

What changed:

1. **Regions.** `tantalus_game::field::region` ports `SetRegion` polygons and `TreadQuad`/`IsInQuad` (triangles of three consecutive vertices, large-border barycentric test). `FieldHost` stores `DefineRegion` polygons in `FieldWorld::regions`, records `DefinePlayerControl` as `control_actor` and `SetUserControl` as `user_control`, and answers system variable 2 (`IsMovementEnabled`) from it.
2. **Per-frame region requests.** The reference calls `CollisionRequest` for the controlled actor every frame while user control is on, which runs `Request(quad, 1, 2)` for the first region with a tag-2 function containing the actor. Field 50's lantern logic relies on this: region tag 2 sets Map byte 38 every frame, and entry 17's loop clears it every frame unless Confirm is pressed in the same frame. `chain --explore` re-runs the region's tag 2 every tick while the actor is inside (new `FieldDirector::remove_entry`), places the actor at each tag-2 region's centroid in turn (stand-in for walking, 120 ticks per region), and presses Confirm (`EventInput.Confirm` `0x20000`) every 15 ticks.
3. **Battle yield.** `ENCOUNT`/`ENCOUNT2` return `FLOW_STATE_JUMP_BATTLE`; `EBin.EntryLoopDone` halts the event loop with the object's ip past the opcode, and the object resumes there after the battle. The VM used to mark the function `Completed`, which lost entry 13's post-battle code (party HP restore, Map byte 44, the exit). It now yields one frame. With no battle engine yet, `chain` skips each battle as a win and reports it in `battles_skipped`.

## Result on 2026-11-16, second run (shared variables, loop functions, party, scripted confirm)

`tantalus-trace chain ... --character-data .reference/Memoria/Memoria.Patcher/StreamingAssets/Data/Characters --confirm-after 30 --ticks 6000`

| Map | Archive | Ticks | Ops | Outcome |
|---|---|---|---|---|
| 70 | `EVT_ALEX1_TS_OPENING` | 35 | 124 | Movie `0x103`, then `ChangeField(50)` |
| 50 | `EVT_ALEX1_TS_CARGO_0` | 6000 | 542,526 | Opening dialogue plays (2 confirms); reaches the first player-controlled point, no errors |

Four fixes got field 50 there:

1. **Shared variables (G1).** `tantalus_legacy::VariableMemory` (`event/memory.rs`) ports `EBin.GetVariableValueInternal` / `SetVariableValueInternal`: byte-addressed, little-endian, overlapping typed access, bits at `index >> 3` / `index & 7`, both 24-bit types sign-extended on read. `LegacyEventHost::variable_memory(scope)` (default `None`) lets a host supply the memory; the VM routes its one variable read and four typed writes through it and keeps its private per-VM map only when the host returns `None`. `FieldWorld` owns a 2048-byte `global_memory` (`gEventGlobal`) and an 80-byte `map_memory` (`mapvar`); `FieldHost` returns them for Global and Map, Instance stays per VM. `chain` zeroes map memory per field (`EventEngine.cs:556`).
2. **Loop functions.** `EventEngine.Return` (`EventEngine.cs:1178`) moves an object whose init function (tag 0) returns to `stateRunning` and starts its tag 1 at level 7. Field 50's scene logic lives in those tag-1 loops (entry 3 is a stage sequencer on Map byte 24, advanced by Map bit 231). `chain` now starts tag 1 when tag 0 completes (14 loops in field 50). This belongs in `FieldDirector` eventually.
3. **Party records.** Entry 17 reads `B_CURHP` of a character before the party exists. `FF9Play_Init` builds a player record for all 12 characters at level 1 with full HP/MP. `tantalus_game::new_game` ports that (`max_hp = BonusHP[lv-1] * str / 50`, `max_mp = BonusMP[lv-1] * mgc / 100`, level 1 gives Zidane 105 HP / 36 MP). The tables are read at run time from the pinned Memoria `BaseStats.csv` / `Leveling.csv`; nothing is copied into the repository. This is a stopgap until the tables are read from the install.
4. **Scripted confirm.** `MES` flag bit 7 (128) is a dialog window the player closes. `--confirm-after N` closes any window open for N ticks.

## Where field 50 waits now: region triggers and input

Map byte 24 is 1 (stage 1). Entry 17's stage-1 case enables user control and waits for `KeyOn(0x20000) || KeyOn(0x80000)` while Map byte 38 == 1. Map byte 38 is set only by the tag-2 functions of region entries 4 to 7 (entry type 1), which the reference runs while the player stands inside the region polygon. Driving past this point needs (a) region-entry detection for type-1 entries against the player position (`SetRegion` polygons) and (b) a player input tape that walks into the region and presses confirm. The first battle (`StartBattle` 336, entry 13 stage 11) and `ChangeField(52)` follow.

## Result on 2026-11-16, first run

| Map | Archive | Ticks | Ops | Outcome |
|---|---|---|---|---|
| 70 | `EVT_ALEX1_TS_OPENING` | 35 | 122 | Plays movie `0x103` (disc 1, movie 3), then `ChangeField(50)` |
| 50 | `EVT_ALEX1_TS_CARGO_0` | 4000 | 12,246 | Stalls: entry 16 polls Map byte 44, which another entry writes |

Before the movie service, field 70 polled `SyncCinematic` (system variable 15) forever: 6,089 ops over 2,000 ticks and no field change. The new `FieldFmv` (`crates/tantalus-game/src/field/fmv.rs`, ported from `Global/fldfmv.cs`) answers system variables 14 and 15 and handles `FMV` dispatch operations 0 to 5. Movie lengths are not imported yet, so a played movie finishes on its first frame, as if skipped.

## Blocker for field 50: shared variables (milestone G1)

Field 50's entries talk through Map-scope variables, and later fields branch on Global variables (Global Int16 2 is assigned in field 70). Memoria stores Global variables in one 2048-byte `gEventGlobal` and Map variables in one 80-byte `mapvar` shared by every entry and zeroed per field (`EventContext.cs:9`, `EventEngine.cs:556`), with typed, overlapping reads and writes (`EBin.GetVariableValueInternal` / `SetVariableValueInternal`). The Rust VM keeps a private `(scope, index) -> i32` map per VM, so a write in one entry is invisible to the others and to the next field. Field 50 also contains the first battle (`StartBattle` 336, entry 13) and the exit `ChangeField(52)` (entry 15).

## Result on 2026-11-18, tenth run (HP carried between fights)

The chain now carries battle HP and MP back to the field and applies the
field heals the story scripts issue.

1. `field_battle::save_party` ports `btl_sys.SavePlayerData`: each party
   member's HP and MP at the end of a fight go back to the player record,
   clamped to the maximum. A fight that failed to run changes nothing.
2. The field host handles `SetPartyHp` (0xF1) and `SetPartyMp` (0xF2) as
   `EventEngine.DoEventCode` does: the script's old character index maps to
   `CharacterId`, values clamp to the maximum, 9999 HP and 999 MP restore
   fully, and a field heal on Beatrix also heals Cinna, Marcus and Blank by
   the same share.

Before the field heals, the party wiped from TH_E004 onward (chain67). With
them (chain68), every chain battle is a victory except TH_E003, which ends in
its scripted interruption (result 5). The party ends the run at 97/105,
90/90 and 105/105.

Open: `read_character_stat` still reads the record by the script's character
operand directly; it needs the same old-index mapping once a script reads
Beatrix, Cinna, Marcus or Blank.

### Rewards

Victories now pay into the field world (`field_battle::pay_rewards`). Each
enemy death runs `BattleBonus::set_bonus` for that enemy's row
(`btl_sys.SetBonus`), and so does a script write to character member 71
(`btl_scrp.SetCharacterData` case 71), which is how bosses that never reach
0 HP pay. `BattleReward::from_bonus` then gives the gil, the item stacks and
each line's EXP share. Scenes flagged `AfterEvent` (flag 2048) pay nothing,
as in `battle.cs:72`.

In chain70 the TH_E001 to TH_E005, EF_E006 and BR_E008 fights pay nothing:
the party is alive at the end but gets no EXP share, which only an
`AfterEvent` scene gives. EF_E007, the second Prison Cage fight, pays 436 gil and items 240
and 244 through member 71. AP and cards are reported but not applied: the
field world has no ability or card state yet.

### Equipment stat bonuses

Chain party units now add their default set's stat bonuses, the equipment
half of `ff9play.FF9Play_Update`: each item's `BonusId` (`Items.csv`) picks a
`Stats.csv` row, and speed, strength, magic and spirit are summed over the
five slots and capped at 99. The Evil Forest party (Zidane, Marcus, Blank)
has no bonuses on its default gear, so chain71 matches chain70. Freya, Eiko,
Amarant and Beatrix do get them (Beatrix: +1 speed, +2 strength, +3 magic,
+8 spirit).

## Result on 2026-11-18, eleventh run (past Baku, into the Evil Forest)

Command: the tenth run's, with `--max-fields 160`, three more route entries
and one more choice:

```
--route '100>101,101>102,102>103,103>105,105>106,106>115,153>150,204>205@2250,205>202@2250,202>200@2250'
--choices 52:66=1,64:127=1,202:242=1,202:242=0@2100,202:242=1@2200
```

Two changes got the chain past Baku's fight:

1. **Battle return runs main function 10.** Back on the same field after a
   battle, `EventEngine.StartEvents` restores the saved context and requests
   the main object's function 10 at level 0 (`EventEngine.cs:598`). The chain
   now does the same after each fight. On map 202 that function sets the
   scenario to 2200 and starts the scene after Baku's fight; before this, the
   chain stayed at 2100 and wandered the ship until the field cap.
2. **Route entries can wait for a story step.** `--route MAP>DEST@SCENARIO`
   applies only from that scenario on, like `--choices`. The three new entries
   send the explorer from the Gargant room to map 200 once Vivi's magic sword
   scene (2250) is done. `202:242=1@2200` answers "Hold on" to Baku after the
   fight. At 2250, "Anytime!" sets map var 31, but the main loop no longer
   waits on it. Control stays off.

The story then runs 2100, 2200 (Baku's post-fight scene), 2225 (Steiner in
storage), 2250 (magic sword), 2260 (map 200), out through map 206 into the
Evil Forest (maps 250 to 257), 2270, 2280 and 2300. EF_E009 (Plant Brain,
battle 303) is a victory and pays 468 gil.

| Battle | Scene | Result | Party HP after |
|---|---|---|---|
| 303 | EF_E009 | victory | 44/105, 41/90, 16/105 |

Open:

- Map 257 (`EVT_EFOREST1_EF_DEP_0`) stops at text 154 ("What are you
  waitin' for!?") after the Plant Brain fight and holds the field for the
  whole tick budget.
- The battle party is still Zidane, Marcus and Blank. The script's party
  changes (Vivi and Steiner join for Plant Brain) do not reach the list the
  chain builds fighters from.
