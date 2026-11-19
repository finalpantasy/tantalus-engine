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

## Twelfth run (out of the Evil Forest)

Three fixes take the chain from the map 257 stall to the world map.

1. **Requests follow the object's script level.** Memoria runs a request
   only when its level is below the target's current level
   (`requestAcceptable`: `lv < p.level`). The field world used to refuse any
   request while the target had one running. It now keeps a stack of the
   levels of each object's running requested functions. A request is
   accepted when the stack is empty or its level is below the lowest entry.
2. **`DRET` (0x97, ReturnEntryFunctions) returns every function.** Memoria
   clears the object's call stack and restarts its loop at the idle level.
   The director now reports these objects and drops their request stacks.
   The chain drops their requested-function entries. On map 257, main
   returns object 16's endless idle loop (tag 24) and then asks it for tag
   26. Before this, that request waited for the whole tick budget.
3. **The chain stops at `WORLD`.** `ChangeWorldMap` ends the run with
   `world_map` set in the field record. The world map is not built yet.

`652:90=11` answers the Mogster tutorial menu with its last line, which
falls to the menu's default branch and closes it.

The story now runs past 2300: Blank is petrified (2350, 2375), the Mogster
tutorial plays on map 652, and at 2400 the spring field (map 262) sends the
party to world map 9000.

Open:

- The battle party is still Zidane, Marcus and Blank (see the eleventh run).
- No world map, so the chain ends at the forest exit.

## Thirteenth run (the party follows the script)

The chain builds fighters from the field party, and scripts now set that
party the way Memoria does.

1. **`partychk` and `partyadd` take the old character index.** Memoria
   maps it with `chr2slot` (Cinna 9, Marcus 10 and Blank 11 become ids 8, 9
   and 10). Values outside 0 to 11 become NONE and the add fails; the field
   used to add `-1` as a member.
2. **`PARTYDELETE` (0xDD) applies at once.** It was a host event handled
   after the VM step, while `partyadd` runs inside it. Map 100 empties the
   party and adds Vivi in one step, so the late removes dropped her. The VM
   now calls a new `remove_party_member` host method (a no-op by default) and
   still reports the event.

| Map | Scene | Party (character ids) | Result |
|---|---|---|---|
| 50 | TH_E001 | 0, 8, 9, 10 (Zidane, Cinna, Marcus, Blank) | victory |
| 67 | TH_E004 | 0, 1, 2, 9 (Zidane, Vivi, Garnet, Marcus) | defeat |
| 251 | EF_E007 | 0, 3 (Zidane, Steiner) | victory |
| 202 | BR_E008 | 0 (Zidane) | victory |
| 257 | EF_E009 | 10, 0, 1, 3 (Blank, Zidane, Vivi, Steiner) | victory |

The forest exit leaves Zidane, Vivi, Steiner and Garnet in the party.

Open:

- TH_E004 is now a defeat. The stand-in policy (the first ready member hits
  the first enemy) loses with this party; it won with the old wrong one. The
  chain still continues, as it does after any fight.
- EF_E006 ends with no outcome, as before.
- Removing a member closes the gap instead of leaving an empty slot, so a
  later add joins at the end rather than in the freed slot.

## Actor tracks on screen

`tantalus-trace chain --actor-track <dir>` writes one file per field visit,
`NNN_<map>.jsonl`. The header line names the map and its background field;
every second tick after that gives the player id, the control flag and each
actor's position and facing. Run 84 wrote 147 files.

`tantalus-client view-track --assets <StreamingAssets> --track <file>` plays
one of these over the field background in a window at 15 frames a second,
looping. `capture-track ... --tick T --output <png>` saves one frame without a
window. Actors are boxes for now (amber for the player, cyan for the others),
placed through the field camera. On `006_103` (Alexandria main street) the
player walks the street between ticks 100 and 600. Some cyan boxes sit on
roofs; those objects are probably hidden or off-stage, and the track does not
record visibility yet.

## Fourteenth run (AP, cards, and past the world map)

Victories now pay AP. Each character's learnable table comes from Memoria's
`Abilities/<Preset>.csv` (preset from `CharacterParameters.csv`), and the
abilities on their default equipment come from the `AbilityIds` column of
`Items.csv` (`AA:X` is X, `SA:X` is 192 + X). `pay_rewards` calls
`FieldWorld::apply_ap` once per equipped item carrying the ability, as
`BattleResultUI.AddAp` does. A card goes to `FieldWorld::cards` while the
collection holds fewer than 100. In run 85, Zidane earned AP toward Flee
(40 AP to learn) from the Evil Forest fights. No fight in the slice dropped a
card, so the card path is covered only by a unit test.

The chain no longer stops at `ChangeWorldMap 9000`. With
`--world-exits 9000:4`, it runs the world script's own exit code
(`EVT_WORLD_WORLD00`, a `JumpSwitch` on the party's location in map variable
29) with the real VM and follows its `ChangeField`. Location 4 is the Ice
Cavern: run 86 went from the Evil Forest to field 300 (`IC_ENT`) and on to
301 (`IC_STP`) at scenario 2500. Walking the world map is not modeled.

Run 90 fixes four gaps in field battles:

- Member 66 (`MODEL_TYPE`) now reads the player's `CharacterSerialNumber`
  (`GetHackedSerialNumber`, `btl_scrp.cs:861`), computed with the
  `CharacterParameters.csv` formulas from the default weapon's `GraphicsId`
  and the scenario counter. It used to read 0 for everyone, so the scripts
  put the whole party in every "who is Zidane / Garnet / Steiner" variable.
- Enemy commands read the scene's attack list (`enemy_attack[sub_no]`,
  `btl_util.cs:422`): script, power, rate, elements, status set
  (`Battle/StatusSets.csv`), category and target. Before this every enemy
  command was a basic script-1 hit.
- Script 96 (Trance Full) is ported, and the trance gauge follows
  `btl_init.cs:371`: on only when `gEventGlobal[16]` is set, not for
  category-16 guests, not for Garnet while `gEventGlobal[17]` is set.
- Every command takes a stand-in 75 ticks (its effect lands at tick 40),
  since SFX lengths are not ported. The caster's ATB is still while it
  runs (`sel_mode`), as in `HonoluluBattleMain.cs:445`. With 0 ticks the
  Prison Cage acted three times per party turn.

| Battle | Scene | Run 89 | Run 91 |
| --- | --- | --- | --- |
| 335 | TH_E004 | defeat | victory |
| 302 | EF_E006 | no result after 20000 ticks | victory: the Trance tutorial plays out and Zidane's Free Energy breaks the cage |
| 303 | EF_E009 | victory | defeat: the party wipes with the Plant Brain at 404 HP in its last phase |

The stand-in party policy now picks two abilities from `Actions.csv`. Vivi
casts Fire (Black Magic, command 22) while he has 6 MP. Zidane uses Free
Energy (Dyne, command 26) in Trance, which is open while its Skill twin
Flee is equipped (`BattleHUD.cs:1368`, `AbilityTranceList`). Everyone else
attacks. The machine builds both from `aa_data[sub_no]`.

303 is still a defeat. The chain fights almost no random battles in the
Evil Forest, so the party reaches the Plant Brain at level 1. Even at full
HP that party takes off only 610 HP before it wipes. A player would arrive
a few levels higher and would heal with Potions; the policy does neither.

Run 93 adds items:

- A new game starts with the `InitialItems.csv` inventory (7 Potion,
  2 Hi-Potion, 2 Ether, 2 Phoenix Down, 2 Remedy, 1 Phoenix Pinion, 1 Tent).
  It used to start empty.
- The Item command reads `ItemEffects.csv` through `Items.csv`'s `EffectId`,
  draws from the field inventory, and the items used leave it after the
  fight, won or lost.
- The policy revives a fallen member with a Phoenix Down or Pinion (Vivi
  first), gives a Potion or Hi-Potion to the member under 60% HP with the
  least left, and a Remedy to a blinded fighter. Vivi never stops casting to
  heal. An ally already named by a queued Item command is skipped; without
  that, three members spent their turns on one Potion.

The Plant Brain script sets its HP to 458 (`(max - 10000) / 2`) after its
sixth turn or once Zidane is low or down, so damage before that is lost.
Its Thunder on the whole party deals about 70 to each level-1 member, which
matches Memoria's magic formula. `tantalus-trace fight --hp` reproduces the
chain's start (Zidane at 48 HP). It wins now,
as does a full-HP start. In run 93 the chain still loses it with the boss at 8 HP
(run 91: 404): the Prison Cage fights spent two Potions first. The party is
still level 1; random encounters in the Evil Forest would give a player the
levels this fight expects.

Run 95 adds field random battles, ported from `EventEngine`:

- `SetRandomBattles` (`ENCSCENE`) stores the pattern and four scenes, and
  `SetRandomBattleFrequency` (`ENCRATE`) the rate.
- The controlled actor's walked distance feeds the timer. Every 960 units
  (`EncounterInterval`) the rate is added to the danger value, and a
  `random8` under danger / 8 starts a battle. `SelectScene` picks one of
  the four scenes with the reference's chance table.
- Each field start resets the timer to -1440, the danger to 0 and the rate
  to 0 (`InitEncount`). `ENCOUNT` also clears the danger.
- The chain jumps the player to each target, so a jump counts as walking
  its length: one check per 960 units.

| Battle | Scene | Run 93 | Run 95 |
| --- | --- | --- | --- |
| 67 | EF_R007 | not fought | victory, 7 EXP each |
| 310 | EF_R002 | not fought | victory, 13 EXP each |
| 309 | EF_R003 | not fought | victory, 13 EXP each |
| 303 | EF_E009 | defeat, boss at 8 HP | victory, Steiner standing |

The three Evil Forest fights take the party to level 2, and the Plant Brain
falls. Every battle in the Alexandria to Evil Forest slice is now a victory
(337 is the scripted result 5).

## Story route: field 301 to Lindblum

`audit/story-route.sh` is a second regression beside the chain99 run. It
runs chain99 with `--save-at 301:<file>` (the output stays identical), then
loads that world and walks on to the cargo ship's engine room (field 504,
about 4 minutes in all). `audit/story-route-expected.json` holds the field
sequence, ticks and scenario counters it must reproduce; the script prints
`STORY-ROUTE-SAME` or `STORY-ROUTE-DIFFERENT`.

The route goes Ice Cavern (301 to 312), the world exit to Dali Village
(`--world-exits 9011:14`), the inns, the windmill errands, the underground
ladder (404) and the cargo-ship tunnels (406 to 400), the Dali airport (455
to 452, scenario 2800 to 2840), the airship (451, 453, 450), and the cargo
ship (454, 505, 504).

The reference behavior is now the default for the trace tool, the play
window and the game. The chain baseline moved with it (chainNNN, 150
records, same map sequence and same end, field 301). Engine gaps this route
closed:

- Real clip lengths (`GetCharAnimFrame`, round(length * frameRate) + 1) come
  from `audit/animation-frames.csv`, embedded in the tools. Field 301 waits
  for frame 34 of a clip the default 32 ticks never reach.
- A script request names a uid (`FindObjByUID`), and an asynchronous request
  at a level not below the object's current one is dropped
  (`EventEngine.Request`).
- `InitObject` of one entry with two uids makes two objects that run the
  same code (the Dali inn 128 and 129, the airship 128 and 129).
- The uid table is cleared when a field starts, as `InitEvents` does; a stale
  uid kept the airship guards in one object.
- `TurnTowardObject` turns the object, and a long turn ends a leftover
  animation loop (`StartTurn` sets `animFlag = 0`), which freed the airship
  scene.
- `--hold 307:16`, `--hold 404:96` and `--hold 504:16`: the slide and the
  ladders poll `KeyState(Held)`. `--skip-talk 455:1,455:2,455:4` skips the
  Dali airport talks the route does not need.

The Dali airport errand needed no engine change: with the three talks
skipped, scenario 2810 follows.

Later engine fixes took the route through the cargo ship to Lindblum:

- `MSPEED` is units per frame (default 30); the old default of 16 left the
  engine room 504 stalling (walk waits that never finished in time).
- Instance variables (`Instance` scope) belong to the object, not to the
  function (`Obj` local buffer). Objects run in creation order, and a created
  object is appended to that order.
- After a battle every object except uid 0 is suspended until the main
  object's function 10 returns (`EnterBattleEnd`/`ExitBattleEnd`).
- `PlayJumpAnimation`/`SETVY3`/`PREJUMP`/`POSTJUMP` frame spans and
  `FinishJump` (field 502 and 506).
- `--world-exits 9001:-1` runs the cargo ship's scripted world scene as a
  field; it ends in its own `ChangeField(503)`.
- `--choices '*:3=6'` answers a text on every field on every field (moogle menu
  Cancel); an open dialog with a chosen answer is no longer confirmed blind.

The region 10 re-fire on the deck (507) is the reference rule (a region's
tag 2 is requested every frame while the player stands in it with control);
the lock came from the slow walk speed and is gone. The route reaches
Lindblum (field 611, scenario 3000) and goes on to the festival fields.

### Lindblum route (fields 550 to 573, 600 to 614, 850), scenario 3000 to 3118

How a fresh worker gets from the arrival to field 565 and what to read.

1. Arrival: 614 (3000), 600 (3050), 571 (3100), 607 (3105), 554 (3110).
   Each field sets the next scenario value near its end.
2. Active Time Events (ATE). Lindblum fields 550 to 573 and 611/850 show
   the ATE icon (`SetActiveTimeEventIcon(1)`, Map155, Global238 = 1).
   `Global236` is a bitmask of the ATEs on offer; Global241 is the choice
   mask. The menu is entry 3 of field 611 (text 125; text 116 in 555): it
   needs SysVar2 (`IsMovementEnabled`) == 1, Global238 == 1 and
   `KeyOn(1)`. `KeyOn(1)` is `EventInput.Select`, so the player opens it
   with Select, never Confirm. It then does `ChangeField` to 555, 561, 609
   or 850 by SysVar9 (the dialog choice). At 3110 Global236 is 1 (field 553
   region 7 sets it once), so only the first ATE exists: it goes to 555 with
   Global2 = 1 (Map var 57 = 1), a long cutscene (texts 619 to 640) that
   clears bit 0 of Global236 at its end and then returns to the field the
   player came from (a switch on Global239 = the previous field, base 550).
   The `ChangeField(567)`, `(566)`, `(565)` branches in 555 are only these
   return targets. Region 23 in 555 is a guard dialog (object 12, tag 39).
3. After that no ATE is on offer; the way on is a normal exit. Field 563
   (Theater District air cab station) shows text 99, a destination menu.
   Choice 3 flies to the exit that leads to 566; then 566 -> 567 (region
   11/12 of 567, Global2 = 19) -> 565. Entering 565 with scenario < 3115
   sets 3115 (a debug-skip window assigns it); 565 -> 611 (the air cab cutscene
   at 3115, `JumpSwitchEx` on the scenario in 611 entry 0) -> 565 at 3118.
   Fields 565, 567, 566, 568, 576, 609 and 850 form an island entered only
   from ATE destinations and these exits; the explorer circles them at 3118.
4. The explorer options added for this: `--ate MAP:N` presses Select (up to
   N times, 90 ticks apart) while the ATE icon is up; `--region-talks MAP`
   presses Confirm inside regions that carry tag 3 (reference
   `EventCollision.CheckQuadInput`: Confirm in a region requests tag 3);
   `--hold 557:16,565:16` holds Up for the temple climb (entry 13 polls
   `KeyState(Held)` and the climb loop waits on `WaitAnimation` between
   steps); `--choices 563:99=3` picks the air cab destination. All are
   opt-in, so the chain runs are unchanged.
5. Scenario 3118 (what moves the story). Entering 565 at 3115 plays its
   cutscene, which sets 3118 and `Global236 |= 8`: ATE bit 3 (choice 3 of
   the ATE menu, text 116 in 565, destination 850) is now on offer. Pressing
   Select and picking choice 3 (`--ate 565:3,566:3,... --choices '*:116=3'`)
   reaches 850 (the Tantalus Evil Forest ATE, about 2100 ticks); it toggles
   bit 8 off (850 pc 554) and returns to the previous field. It does not
   change the scenario. The scenario moves to 3120 only inside field 611:
   with scenario 3118 its init (`JumpSwitchEx` case 3118, Map var 50 = 2)
   creates objects 17 and 20 and regions 8 and 9, and function 37 of entry
   0 (text 337 to 345) sets 3120 and clears `Global236` bit 8. 565 has no
   exit to 611 at 3118 (the `ChangeField(611)` in 565 is only the 3115
   cutscene), so 611 must be entered from 612, the castle.
6. Route to the castle at 3118: from 563 the air cab text 99, choice 2,
   flies to 610 (`--choices 563:99=2`), then 610 -> 614 -> 613. Field 613
   at 3118 (Map31 = 3) has the guard region 16 (z -1846 to -1631): standing
   in it requests entry 0 tag 18 every frame (guard 7 walks to the player,
   text 371, walks away). The explorer's target list includes every region,
   so it kept jumping back into region 16 and lost control each time. The
   fix is explorer-side: `--wall-regions 613` counts a non-exit region that
   keeps taking control (three refusals) as done, as exits already were.
   With it, `--route "613>612@3118,612>611@3118"` gives 613 -> 612 -> 611
   (3120) -> 612 -> 613 (3130) -> 601 -> 613 -> 612 -> 611 (3140) -> 617 ->
   616 -> 615 -> 606 (3140). The flag is opt-in, because field 255 of the
   chain run also refuses three times and would change chain109.

### Disc 1 segments

Segment probes start a fresh world at a field with the scenario preset
(`--set-global 0:LOW,1:HIGH` writes the little-endian scenario counter),
no `--load-world`, so party, items and story bits are the new-game defaults
and later cutscenes may take other branches than a played game. Each run
used `--explore --confirm-after 30`, 6000 ticks per field, at most 60 to 80
fields. Scenario values come from the init `JumpSwitch` cases of the
entry fields. "Furthest" is the highest scenario counter the run reached.

| Segment | Start field @ scenario | Furthest | Where it ends, blocker and lead |
| --- | --- | --- | --- |
| Lindblum, hunt | 613 @ 3118 (saved world from the story route), then 606 @ 3140 | 3700 | fixed: 606 waited for Cancel or Special behind a camera-position test on system variables 12/13, which `VRP` fills from the field view. The view now starts at the vrp midpoint and follows SCROLL, release and the 3D player service (`FieldMap.BG_init`, `EBG_scene2DScroll`, `SceneService3DScroll`). With `--pan 606@110:225/165:105/400:104/395:190/275:205/300:105` the run goes 606 -> 615 -> ... -> the Festival of the Hunt (3175, timed, scored by the field scripts from system variables 24/27) -> 3180/3190 -> 3700, then the explorer circles 615 to 619 and 601. Also fixed (2026-11-18): the hunt's battle scripts (`EVT_BATTLE_LB_E080A` to `J`) read system variables 21 (`TimerUI.Enable`) and 17 (`TimerUI.Time`) and end the fight with battle code 33 = 3 when the clock reaches 0; the battle host answered both with 0. A battle now carries the shown field clock, which falls at the battle tick rate. A lost fight in a scene without `NoGameOver` is a game over (`btl_sys.ManageBattleEnd`: result 6, party records not saved) and ends the trace run; the hunt scenes have the flag, so the run above still goes on after 552's lost fight (battle 35) |
| Gizamaluke's Grotto | 701 @ 3740 | 3740 | 701 -> 702 -> 703 -> 704 -> 706 in 6000 ticks, 6 talks; no blocker found yet, needs a longer run |
| Burmecia | 755 @ 3820 with entrance 604 | 3900, disc 2 | fixed. No engine gap past the floor: the route is input. After the floor drops (3830) the player goes back 753 -> 755 -> 756, answers Yes to "Go out to the balcony?" (text 115), which jumps the roofs 752 -> 757 (entrance 607); in 757 he declines the same question, hears the soldier and checks under the bed (3840, 3850), then jumps back and declines at 756 to walk 755 -> 753 (the door, 3860) -> 758 (3870) -> ... -> 766. In 766 the menu text 197 ("Talk to her / Leave her alone / ...") loops on Talk; Leave her alone (choice 1) goes on to 767/768 and the disc change. Explorer change: a story step (scenario change) also lets it re-enter regions that do not leave the field, as it already re-talked to objects. Inputs: `--waypoints 755@1100:3800/600:4400/400:4450 --route "753>755@3830,755>756@3830,756>755@3850,755>753@3850" --choices "757:115=1,757:115=0@3850,756:115=1@3850,766:197=1"`; one run goes 755 @ 3820 -> disc 2 -> Treno (903, 4400) |
| South Gate | 800 @ 3740 | 3750, Gizamaluke | input, not an engine gap. In 801 the stealth scene's soldiers are regions with a confirm function (tag 3), and the seat in 813 (entry 8) fires only past x 224, z -470. `--region-talks 801 --wall-regions 801 --waypoints "813@400:-600"` goes 800 -> 801 -> 802 -> 813 -> 814 -> 707 (Gizamaluke's Grotto) -> world 9000 |
| Cleyra trunk | 1000 @ 4650 | 4660 | with `--hold MAP:16` for 1000 to 1018 (the same Up-held climb as 557/565: entries poll `KeyState(Held)` 16/192/32 and wait on `WaitAnimation`) the run goes 1000 -> 1006 and reaches the world map (9000) |
| Cleyra settlement | 1050 @ 4660 | 4720 | with `--hold MAP:16` for 1000 to 1018 and 1050. Routing, not an engine gate: at 4700 1051 plays its ATE chain (1053, 1055, 1054, 1057, 1058 with entrance 4) and the explorer then took the free exit 1052 -> 1010 into the trunk. `--route "1051>1055@4700,1055>1054@4700,1054>1058@4700,1058>1059@4700"` reaches the cathedral; talking to the priests in 1059 sets 4720. The explorer then takes 1059's door to 1060, where nothing gives control at 4720 (only entry 0's init with Map bit 158 set, or entry 3's ATE menu, which needs control). The story goes back out to the town (4725 is set in 1055); 1060 is likely the wrong way, not investigated further |
| Cleyra, the attack | 1100 @ 4700 | 4888 | with the hold: 1100 -> 1101 (4888) -> ... -> 1109 <-> 1110 loop |
| Red Rose | 1150 @ 5000 | 5020 | 1150 -> 1153 (balance game, text "Press or to shift your center of gravity!") -> 1208 at 5020 |
| Alexandria castle | 1208 @ 5020 | 5050 | 1200/1201 @ 5050 was the wrong start: 5050 is set in 1211. Fixed: 1206 (`EVT_ALEX2_AC_QUEEN`) entry 6 waits for system variable 1 (`GetFieldCamera`, `fieldmap.camIdx`, `EventEngine.GetSysvar.cs`) to leave 0; it had no case and read 0. It now reads the view's camera, which `SetFieldCamera` sets on every field (the tile animation copy is kept only on fields with tile animations). Then input: the cage in 1208 swings with Square/Circle pressed along the swing (`--press-when "1208:42:0:0:32,1208:42:255:255:128"`, map int24 40 is the swing speed) and Steiner climbs the chain in 1209 with Up held (`--hold 1209:16`). The run goes 1208 -> 1207 -> 1206 -> 1205 -> 1107 -> 1108 -> 1153 -> 1152 -> 1151 -> 1150 -> 1208 (5030) -> 1209 -> 1211 (5050) -> 1212 -> 1222 -> 1220 -> 1219 -> 1215. 5070 is set in 1206 (path 1215 -> 1214 -> 1224 -> 1206). 1215's exit region 5 leads to 1214 only above height -100 (the upper floor) and to 1219 on the ground, so the explorer, which saw it lead to 1219, never walked the stairs. Fixed in the explorer (not an engine gap): a routed exit that led away from the target remembers the floor height it was taken from, is not fired again from that floor, and the player walks the walkmesh to a floor under the region at another height (`floor_point_in_region`, `walkmesh_step`'s `wanted` filter): the gallery at height 1716 behind the ramp. `--start-party 0,1,2,3` gives a fresh-world probe a party (an empty party loses the first fight, which now ends the run). Probe from 1209 so the timer of 1211 (`SetTimerTime 1801`) is running (a world started at 1211 to 1215 with timer 0 hits the scripts' timer-expired `GameOver`): `--start 1209 --set-global 0:182,1:19 --hold 1209:16 --wall-regions 1215 --route "1215>1214@5050,1214>1224@5050,1224>1206@5050"` runs 1209 -> 1211 -> 1212 -> 1222 -> 1220 -> 1219 -> 1215 -> 1214 -> 1215 -> 1224 -> 1206 (5075) -> 1202 |
| End of disc 1 | 768 @ 3890 with entrance 626 or 627 | 4400, disc 2 | disc 1 ends in Burmecia, not in 1800: `EVT_BURMECIA_PALACE_2` (768) sets entrance 1 and changes to field 70, whose `DISCCHANGE 17184` goes to field 800 on disc 2. Fixed: 812 waited in entry 0 function 1 for `B_OBJ 12` member 6 (`Obj.level`, `EBin.getvobj`) to rise above 2, that is for the player's requested walk (level 2) to return; the member was never kept and read 0. It now is the level of the running request, 7 in the idle loop (`EventEngine.Call`/`Return`). Also fixed: `EVT_WORLD_WORLD02` (world map 9002) reads the place from map byte 30, not 29, so `--world-exits 9002:6` (Treno, field 908) works. The run goes 768 -> 767 -> 768 (3900) -> 70 -> 800 -> 814 -> 813 -> 812 (4150) -> 810 (4210) -> 809 -> 815 -> 811 -> 816 -> 803 -> 804 -> 806 (4230) -> world 9002 -> Treno 908 (4400) -> 905 -> 915 -> 907 -> 911 -> 903, which has no control for 6000 ticks (text 111). 1800 @ 10250 is disc 3 (Garnet's haircut); its `DISCCHANGE` (-30956) goes to 1812 on disc 3, and its party menu wait in 2850 needs a roster (a fresh world has an empty party) |
| Treno, disc 2 | 903 @ 4400 | 4500 | not an engine gap. 903's card usher (entry 4, region 12) shows its rules menu (text 113) and the stadium door (region 9, a `StartCardGame` door) needs 5 cards (`GetCardAmount`, system variable 19, `FieldWorld::cards`); a fresh-world probe has none, so the explorer circled the usher for 6000 ticks. `--cards 8` gives the cards. The rules menu is `EnableDialogChoices` on global byte 370 (65 at first: bits 0 and 6 enabled, each answer enables the next question), so the explorer, which picks choice 0, kept asking; `--choices 903:113=6` leaves (the last question, case 6). Region 12 is not an exit and keeps taking control: `--wall-regions 903`. The real story step is 904 (scenario 4420, set when 904's init runs below 4420). The Treno loop then goes 904 -> 912 (the ladder needs `--hold 912:16`: the player's tag 14 plays animation 10741 while its height is between 4654 and 5799) -> 900 (4442) -> 916 -> 909 -> 931 -> 902 -> 900 (4450) -> 912 -> 908 -> 905 -> 913 (4455) -> 914 (4460) -> 932 -> 930 -> 914 (4470) -> 950 (4500) with `--start-party 0,1,2,3 --cards 8 --wall-regions 903 --choices "903:113=6,*:3=6,*:4=1" --hold 912:16` |
| Gargan Roo to Cleyra | 950 @ 4500 | 4600, world 9000 | 950 -> 952 -> 953 (4520) -> 952 (4530) -> 953 -> 954 -> 956 -> 1201 (4600) -> 769 -> world map 9000. A level 1 party loses 954's fight (`--battles`, which now ends the run in a game over); the probe leaves `--battles` off there (the fight is skipped). `--world-exits 9000:19` is Cleyra's trunk (1000): the `EVT_WORLD_WORLD00` `JumpSwitch` has base 2, so the case is the place number minus 2 |
| Cleyra cathedral | 1051 @ 4700 | 4800 | not an engine gap: 1060 at 4720 is idle by design (its init holds the scenes below 4725 as idle objects); the story goes back to the town. A route entry that waits for a scenario now also takes effect inside a field (`--route` is re-read every tick), so after the priests set 4720 in 1059 the route goes 1059 -> 1058 -> 1054 -> 1055 (4725, set in 1055) -> 1051 (4727) -> 1052 (4730, antrion) -> 1060, where the cathedral scene runs 4618 ticks and sets 4800 (the ticks are more than the default 3000 of a probe). Route: `1051>1055@4700,1055>1054@4700,1054>1058@4700,1058>1059@4700,1059>1058@4720,1058>1054@4720,1054>1055@4720,1055>1051@4725,1051>1052@4727` with `--hold 1000:16..1018:16,1050:16,1100:16` |
| Cleyra, the attack (second pass) | 1060 @ 4800 | 4888 | 1060 -> 1110 -> 1208 -> 1207 -> 1206 -> 1205 (the queen) -> 1107 (4860) -> 1108 -> 1109 -> 1110 -> ... -> 1101 (4888) -> 1100 (the ladder needs `--hold 1100:16`). Then the explorer circles 1100 to 1110 for 8000 ticks at 4888: the next step is 4910 in 1101 (entry 25's stage 9 plays battle 298 after stage chain 1 to 9 of entry 3, which advance on map bit 231 and depend on the entrance variable the previous field set); not found yet which entrance starts it |
| Cleyra attack to Alexandria (probe, fights skipped) | 1016 @ 4888 | 5020 | not an engine gap, the route is input. 4888 -> 4910: `--route "1016>1014@4888,1014>1012@4888,1012>1011@4888,1011>1010@4888"` with holds `1000:16..1018:16,1100:16`; 1010 sets 4900 and goes to 1105 then 1101, where battle 298 sets 4910. 4910 -> 4980: the stage drivers in 1104/1105 wait on the player's position, so `--start 1104 --set-global 0:46,1:19,2:41,3:3 --waypoints "1104@129:3930" --wall-regions 1104`; the Beatrix scene in 1110 sets 4980, then 1109, 1108, 1153 (4990), 1150 (5000), 1153 (5020) |
| Alexandria, Pinnacle Rocks (probe) | 1208 @ 5020 | 5510 | `--hold 1209:16`, `--press-when "1208:42:0:0:32,1208:42:255:255:128"`, `--wall-regions 1201..1230,1101`, `--route "1215>1214@5050,1214>1224@5050,1224>1206@5050"`; 5050, 5075, 5090, 5100, 5120, 5140, 5160, 5200 (Gargan 956/955/957), 5300, 5400 (1250/1251). Fixed: the shared-script entity key `SHARED_SCRIPT_KEY` was `0x1_0000`, which is the key of the second object `(1<<16)|2` of an entry, so a clone's function was mistaken for the shared script and 1251 stalled at 5400; it is now `1 << 48` (test `shared_script_keys_do_not_name_second_objects`). Only the `not_completed` diagnostic keys in the chain change (chain114 = chain113 apart from those labels, 7 lines). 1251 -> 1252 (5510; `--choices "1252:4=6,*:3=6,*:4=1"` for the moogle menu) -> 1253 -> 1254 -> 1255 (5598) -> 1300 (5600) -> 1302 -> 1305 -> 1307 (5660, the Artania scene in Lindblum castle) |

What is left: from 1307 at 5660 (Lindblum castle) on: Qu's Marsh, Fossil Roo, Conde Petie, Madain Sari, the Iifa Tree and field 1800 are not probed. Probes use `--start-exp 4000` (new trace option: experience for each party member) and omit `--battles`. `audit/story-route.sh` reported STORY-ROUTE-DIFFERENT on `origin/main` (6055031): not an engine bug, the stand-in party was too weak. (1) Field 308's fight (EVT_BATTLE_IC_E011, Zidane alone against two enemies) never ended: the second enemy heals the first (attacks 7 and 9 heal 88 to 126) faster than Zidane's 44 a hit, and the policy always hit the first enemy. After 1500 ticks the stand-in now attacks the weakest enemy first (the healer): won in 6392 ticks. (2) The Dali airport fight (BSC_AP_E012, Black Waltz No. 2, 11030 HP, the script ends it when the HP is 10000 or lower) answers every Fire with a counter (cmd 53) of 90 damage to all, so Vivi lost himself and the party; once a counter aimed at the party has run, the stand-in Vivi attacks instead of casting. The cargo ship fight (CA_E013) counters physical hits with a no-op aimed at itself, which does not count, so Vivi keeps casting there. Result: STORY-ROUTE-SAME, 97 fields to 565 at 3118, expected file unchanged; chain113 identical. The party menu (`PARTYMENU`) is
answered in the trace by keeping the party, filled from the roster when it
is too small; `play` drives it with the pad.

The chain moved with these fixes (the scrolled field view, `MOVH`, pointer
250, the per-actor floor): chain110 has the same 158 fields, next maps and
scenario counters as chain109, with different ticks and operation counts
in fields that use `MOVH` or read the view (103, 115, 307 and others).
`audit/story-route-expected.json` was regenerated for the same reason
(same fields and scenario counters, new ticks).

chain111 (2026-11-18): a choice list with a `[PCHM]` tag takes the
script's `EnableDialogChoices` mask (`ETb.SetChooseParam`,
`DialogBoxSymbols.SetupChoose`), so the scripted player can no longer pick
a removed option; the cursor starts on the lowest one left. The ticket booth
(104, text 251) hides "Show ticket" on the second visit, so the run now
asks the other questions there. Same 159 records, fields, next maps and
scenario counters as chain110; ticks, texts and choices differ from field
104 on. The story route changed only in the ticks of field 455 (the region
retry) and was regenerated. chain110 vs chain110.worker: not
nondeterminism. Two runs of main are byte-identical, and the worker file
is reproduced exactly by adding `*:3=6` to `CHOICES` (it answers the
moogle menu at field 253, the only moogle field the default list leaves
out).

chain112 (2026-11-18): system variable 1 reads the field camera. Same 159
records, fields, next maps, scenario counters and ticks as chain111; only
operation counts differ, in the fields that poll the camera (51, 52, 63, 67 and 153).

`audit/story-route.sh` now ends at field 565, scenario 3118 (97 fields,
`MAX_FIELDS`); probe with
`tantalus-trace chain --start 301 --load-world <w301.json>` plus the
options above.

Lindblum to Qu's Marsh (probe, no engine change): from 1352 at 5690 (`--start 1352 --set-global 0:82,1:22 --start-exp 4000`) the world map exit is `--world-exits 9000:22`. The world script's own place prompt (`WorldScript::enter_place`, which `world_exit` already runs first) turns place 22 into field 650 (Qu's Marsh; place 30 gives 650 too). Places at 5690 found by running every place 0..130: 2 -> 1856, 5 -> 1950 (Quan's Dwelling cave, not the marsh), 22/30 -> 650, 23 -> 1256, 24 -> 1352, 25 -> 1300, 31 -> 1463, 32 -> 1425 (Fossil Roo's exit field: it sets 5990 at once, so it is not the way in), 33 -> 1500, 34 -> 1600, 35 -> 1557, 36 -> 1555, 37 -> 1650, 42 -> 2250; places 0, 1, 26, 27, 39, 43 and 44 have no field. Probe inputs: `--choices "652:90=11,*:3=6,*:4=1"` (the moogle help menu in 652), `--start-party 0,1,2,5` (Quina; field 655's marsh scene needs `PartyContains(5)`). Quan's Dwelling (1950, 1952, 1953) needs `--hold 1950:16,1952:64` for its ropes (the entry's function 17 climbs on held direction keys), and 1950 sets nothing in the scenario counter. The way to Fossil Roo is not the world map: field 1422 (Fossil Roo's entrance, which tests scenario 5690) is reached only from 663 (EVT_KUINA_KM_FET_1), and 663 from 662 (FET_0), which field 655 (KM_RED_0, entry 0, function 1) changes to when the marsh state `Global Int16 2 & 0xF000 == 0x2000`, Quina is in the party, the scenario is at least 5690 and global bit 3245 is clear. Entry 3 of 655 is the marsh maze: its exit region goes to 656, 657, 658 or 659 by the state nibble (0x1000, 0x2000, 0x4000, else) and writes 4196, 8292, 16484 or 32868. The explorer wanders 650 -> 652 -> 651 -> 655 -> 657 -> 660 -> 661 -> 655 -> 651 -> 652 -> 654 -> 650 and never reaches 662, so the run stays at 5690. Not solved: why 655 does not take the 662 exit (its script's `ChangeField(662)` was reached in a probe that started in 655 with the party, but the explorer's region exit to 659 won the same tick), and whether bit 3245 or the state nibble is the blocker. Not probed: Fossil Roo (1400 and up), Conde Petie (1500), Madain Sari (1600), the Iifa Tree (1650, 1750) and field 1800. `audit/story-route.sh` and chain115 are unaffected (no code change).

Qu's Marsh to Fossil Roo (probe, no engine change): field 655 takes its 662 exit with the engine as it is. The blocker is the maze, not an op. 655's init (entry 0, function 0) switches on `Global UInt16 396 / 100 * 100` (`JumpSwitchEx`): 100 and 200 disable the Quina triangles, only 300 (stage 3) reaches the `Int16 2 & 0xF000 == 0x2000`, `PartyContains(5)`, scenario >= 5690, bit 3245 clear test that sets map bit 487 and starts the Quina and Quale scene (`Int16 2 = 12287`, `ChangeField(662)`). Variable 396 is the maze stage (hundreds) and the last exit taken, written by the exit regions of 651 to 661. Starting in 655 with `--set-global 2:100,3:32,396:49,397:1` (state 0x2064, 396 = 305) goes 655 -> 662 -> 663 -> 1422 (scenario 5900). From the real route the explorer reaches 660 with state 8372 but takes its entry-8 exit to 661; entry 7 of 660 (THE_0, region at x 2881..3541) writes `396 = 307` and `Int16 2 = 8376` and changes to 655, so `--route 660>655` is the whole fix: 1352 -> 650 -> 652 -> 651 -> 655 -> 657 -> 660 -> 655 -> 662 -> 663 -> 1422 -> 1423 -> 1424 (`--world-exits 9000:22 --route 660>655 --choices "652:90=11,*:3=6,*:4=1" --start-party 0,1,2,5`). Bit 3245 is only set by Fossil Roo's entrance field (1422, `EVT_FOSSIL_FR_ENT_0`), so it is clear on the way in. The earlier note ("the explorer's region exit to 659 won the same tick") was the explorer in the wrong maze stage, not a tick race.

Fossil Roo (probe, one explorer option, no engine change): `tantalus-trace chain` has a new opt-in `--talk-grace TICKS` (default 0, so chain115 and `story-route.sh` are unchanged): after a talk press the explorer waits that many ticks before it acts again. Without it the explorer talks to the gargant region in 1405 (EVT_FOSSIL_FR_SL1_0, entry 8, sets `Global Byte 408 = 1`) and fires the exit region 12 (to 1424) one tick later, before the ride script takes control, so the ride never starts. With `--talk-grace 60` the ride script runs (entry 0 function 1 switches on byte 408: cases 8, 1, 5, 2) and 1405 stays up for the full 6000 ticks, but nothing leads on to field 1400 (its `ChangeField(1400)` with `Int16 2 = 7` is behind the ride's last case), so the explorer still returns to 1424. Fossil Roo facts for the next worker: 1422 (entrance, sets bit 3245 and scenario 5900) -> 1423 (trap room; fights R013, R001, E081 with `--battles`) -> 1424 (SBW; the Lani fight EVT_BATTLE_FR_E033 ends by script at 15145 of 15708 HP; the stand-in party loses it below about 15000 experience each, use `--start-exp 15000`) -> 1405 (SL1, gargant). 1424 also holds `Int16 2 = 5`; 1405's init tests bit 3242 and `Int16 2 == 5` to set byte 408 = 8. Save the world at 1424 with `--save-at 1424:<file>`, restart with `--start 1424 --load-world <file> --route 1424>1405 --talk-grace 60`. Not reached: fields 1400 and up past the first ride, Conde Petie (1500), Madain Sari (1600), the Iifa Tree (1650, 1750) and field 1800. `audit/story-route.sh` still prints STORY-ROUTE-SAME (last map 565, scenario 3118); chain99 is byte-identical to chain115.

Fossil Roo gargant ride (engine fix, probe `--start 1424 --load-world <w1424> --start-exp 15000 --start-party 0,1,2,5 --route "1424>1405" --region-talks 1405 --talk-grace 100 --retalk 1405:7`): the first fix was route input, the second an engine bug. 1405 has two gargant talk regions: entry 8 (west, starts the first ride, `Global Byte 408 = 1`, runs while map bit 311 is clear) and entry 7 (east, `408 = 2`, runs once the first ride set bit 311). The explorer needs `--region-talks 1405` to press confirm inside them, and `--retalk 1405:7` (now also applied inside a field: a new talk makes the field's `--retalk` entries talkable again) to go back to entry 7 after the first ride; `--talk-grace` now counts from the tick a talk function starts, not from every confirm press. The second ride's script (entry 0 function 1, case 2) waits on `408` values written by entry 3's functions 18 (level 6) and 19 (level 4) and entry 13's functions 17 and 20. `EventEngine.Request` (EventEngine.cs:337, `Call`) pushes the running function when a request has a lower level, so function 18 waits while 19 runs; `EntryScheduler` ran both at once, so function 18 wrote `408 = 4` while 19 was still waiting and the later writes overwrote it, and the main script waited for 4 forever. `EntryScheduler::nest_requests` now holds an object's requested functions at a higher level while a lower-level one runs. Test: `field_session::tests::a_lower_level_request_holds_the_object_s_older_function`. With it 1405 -> 1400 -> 1411 at scenario 5900. This moves two baselines by operation or tick counts only: chain99 differs from chain115 only in field 64's operation count (83508 -> 83504; saved as chain116), and `audit/story-route-expected.json` field 450 takes 692 ticks instead of 686 (nothing else moved).

Fossil Roo to Madain Sari (probe, no further engine change; new opt-in `tantalus-trace chain` options `--heal-after-battle` (HP and MP restored after each fight, a stand-in for potions and tents), `--win-battles N,...` (those battle numbers count as won without fighting) and `--start-exp` now also applies to the characters who join later). Save the world at 1424 as above (with `--start-exp 3000000`) and restart with `--start 1424 --load-world <w1424> --battles audit/battle-scene-ids.tsv --heal-after-battle --region-talks 1405,1406,1407,1408,1409 --talk-grace 100 --retalk 1405:7,1406:7,1407:7,1408:7,1409:7 --world-exits 9000:4,9003:33,9003:34@6300 --skip-talk 1420:14,1420:15,1420:16 --hold 1550:64,1551:64,1552:64,1553:64,1554:64 --win-battles 107 --route "1424>1405,1411>1418,1418>1407,1420>1409,1404>1425,1550>1551,1555>1556"`.
Fossil Roo graph from the `ChangeField` constants: SL1..SL5 (1405..1409) are the gargant rides (talk regions 8 then 7 in each, as in 1405), MV1..MV5 (1400..1404) the rides' stops, SR (1411..1417) the rooms, DN1..DN4 (1418..1421) the hubs; the way out is 1418 -> 1407 (SL3) -> 1402 -> ... -> 1420 (DN3) -> 1409 (SL5) -> 1404 (MV5) -> 1425 (exit, sets scenario 5990, world map 9003; `9003:33` goes to Conde Petie 1500). The switch tables in 1420 (entries 14, 15, 16: "Switch No. 2 flip the switch?") start entry 22's cursor game (functions 25 to 27 poll `KeyState` held and wait for a position), which the auto-player cannot play, so those talks are skipped and the exit region 20 leads on. Conde Petie runs by itself (1500 -> 1501 -> 1507 -> 1506 -> 1505 -> 1509 -> 1503 -> 1504 at 6260/6270) to the Mountain Path 1550 (scenario 6300). The Mountain Path fields 1550 to 1554 slide Eiko down a branch with the held Down key (`--hold 1550:64,...`: the slide function loops on object 255's height and `PlayAnimation 10539`); 1550>1551 and 1555>1556 are route input (the explorer would take the lowest exit). 1555 holds the Ralvuimago fight (battle 107, EVT_BATTLE_CM_E051, 8106 HP): the stand-in battle engine lost it even at level 99 because field fights built every party member at level 1 (`party_unit` set `level = 1` and the level 1 strength, magic, speed and spirit with only the HP and MP of the real level; `PartyFighter.level` now carries the level from the experience and `level_basis` applies `FF9Level_GetDex/Str/Mgc/Wpr`: at level 81 Zidane's strength is 52, not 21). The fight now ends in a victory (`--win-battles 107` is no longer needed). `TANTALUS_BATTLE_LOG=1` prints every machine event and script command of a field fight; `tantalus-trace fight --level N` runs one battle at a level. 1555 -> 1556 -> 1557 (scenario 6310) -> world map 9003 -> 1500 -> 1600 (Madain Sari entrance, 6600) -> 1601 (fountain, 6610). Not solved: in 1601 the explorer's two talks (entries 9 and 12 at ticks 3091 and 3106) leave user control off with no waiting script besides idle loops, so the run idles there; 1650/1750, the Eidolon Wall and field 1800 are not reached.

Madain Sari 1601 control (engine fix, two parts; chain99 stays byte-identical to chain117, no regeneration). (1) The control stayed off after the two explorer talks at ticks 3091 and 3106 because both talked to objects that did not exist yet: the host makes an actor stub for every entry, and entry 9 (Mocha, initialized by entry 0 only from scenario 6622) and the region entry 12 sat at the origin next to the player. Entry 9's talk set map Int16 29 = 57 and returned without restoring control, since Mocha's own loop (the one that finishes the scene) was never running. `EventCollision.Collision` walks live objects only, so `FieldWorld::mark_uncreated` marks the stubs and `InitObject` (`init_object`) clears the mark; `talkable_from` skips marked actors. The trace explorer applies it only with the new opt-in `tantalus-trace chain --live-objects`: by default the explorer still talks to stubs, and the Alexandria and Ferry fields (102, 103, 115) need that (their stub talks stand in for regions: with the flag the route does not reach the ticket booth and the ferry unless `--region-talks 103,115` is added, and then field 308 is lost). Probe 1601 onward with `--live-objects`. (2) `read_object_member` now resolves the uid operand: kitchen 1607 entry 36 reads `uid` of object 133 (`InitObject` source 9 uid 133), which used to error ("uid without a UID mapping") and read a zero stub actor; member 5 answers the uid asked for. At 6610 the explorer then goes 1601 -> 1602 (`--route 1601>1602`) -> 1607 (scenario 6615, sets itself) -> back to 1602, 1601, 1600. `audit/story-route.sh` on origin/main (b1ffe9d) already prints STORY-ROUTE-DIFFERENT (last map 308, scenario 2510: the Ice Cavern fight is lost after 7147c1e "party members fight at their real level"); verified with my changes stashed, so it is not from this change.

Madain Sari 1601 onward (probe with `--live-objects`, from `--start 1601 --load-world <w1601>` where w1601 is `--save-at 1601:<file>` of the w1600 run; the run4 command above plus `--live-objects` and a `--route`): the world map exits, the fields and the scenario steps found so far. 1601 is the hub: exits 13 -> 1600 (entrance), 14 -> 1603 (-> 1605), 15 -> 1602 (-> 1606, 1607 by script). The fountain scene sets scenario 6610 whenever 1601 is entered at 6600, so a later start must be in another field. Global UInt16 294 counts the places visited (increments in the region functions of 1601, 1607 and 1600, 1607's kitchen objects and SSD_1); entry 5 of 1602 (talk) needs it at 5 or more to say the long lines, and sets scenario 6620 (EVT_SARI1_MS_EKH_0 entry 5 function 3). With `--route "1601>1603,1601>1602,1602>1606"` the run reaches 1602 at scenario 6620 and idles there (control on from tick 975): region 7 (to 1606) does nothing below scenario 6625 except the one scene (entry 4 function 15, Morrison's "get some sleep"), so the way on is 6620 -> 6622 (1600 entry 10 function 1, an object initialized at scenario 6620 with `G297 & 8`) -> 6625 (1605 entry 12 function 1) -> 6630 (1602 entries 4 and 10). The scenario steps 6640 (1606, Eiko's room, EVT_SARI1_MS_EIK_0) and 6650 to 6870 (1600, 1604 to 1610), the Eidolon Wall, the Iifa Tree (1650, 1750) and field 1800 are not reached. A saved world cannot be restarted at 6620: `--load-world` restores the saved scenario over `--set-global`. `audit/story-route.sh` already printed STORY-ROUTE-DIFFERENT on origin/main before this work (see above).

chain122 (live objects by default; the chain regression command changes). `tantalus-trace chain` now marks every entry's actor stub as not yet created (`--live-objects` is the default; `--stub-objects` restores the old explorer that talks to stubs, and `audit/story-route.sh` passes it so that script is unchanged). The chain regression needs the ticket-booth and ferry talks as region talks, which stood in for stub talks before: add `--region-talks 103,115` to the chain99 command. The chain moved at record 5 (field 102): the cat (entry 3) and Theodore (entry 5) are stubs there, so their talks no longer happen, and field 202 skips two musician talks (entries 7 and 8). With that alone the run reaches `EVT_ICE_IC_STA_0` (field 307; chain121 circled 306/305/652 at 12000 ticks and never left 307) but the stand-in party then loses field 308's fight (battle 21), and on the current main it already loses battle 303 in field 257 (the fewer talks and ticks shift the random sequence; a stub run on the same main wins it), so the command also takes `--heal-after-battle` (HP and MP restored after each fight): `sh local/probe/run_chain.sh chain99 --ticks 12000 --max-fields 175 --waypoints '<as before>' --battles audit/battle-scene-ids.tsv --world-exits 9000:4 --region-talks 103,115 --heal-after-battle`. Result: 164 records, no game over, field 307 left for 308 and the run ends in Dali's view field 312 (scenario 2540, 2950 ticks); two runs are byte-identical. Saved as chain122 next to chain121 in `local/probe/`. Against a stub run of the same main (chain121 rerun, `--stub-objects`, old command) chain121 itself already differs at records 101, 136, 138, 149 and 151 in `battles_fought` only: other workers' battle changes moved it, not this change.

Madain Sari to the Iifa Tree (probe, explorer changes only, no engine change; chain122 and the unit tests are unchanged by them). Start `--start 1600 --load-world <w1600> --start-party 0,1,2,5 --heal-after-battle --live-objects` plus the options of the Fossil Roo run. The scenario steps and what each needs:
- 6600 -> 6622: the 1601 hub as in the note above, with `--route 1601>1603,1601>1602,1602>1601@6620,1601>1600@6620`; 1600's entry 10 function 1 gives 6622 and 1605's entry 12 gives 6625 (`1600>1601@6622,1601>1603@6622,1603>1605@6622`), 1602 gives 6630 and 1606 6640.
- 6640 -> 6645: field 1606 entry 22 is a talk region (`InitializeRegion`, initialized only at 6630 and 6640), so the run needs `--region-talks 1606`; its talk requests entry 28 function 16, which sets global bit 2076 and lets the kitchen's entry 36 (1607) talk give 6645, then 1602 gives 6650 and 1600 6690 (Eiko joins). Probe party `[0,2,1,6]`.
- 6690 -> 6700: the way on is the Conde Petie Mountain Path again (`--route 1500>1501@6690`), then `--world-exits 9003:37@6690` is the Iifa Tree entrance (field 1650); 1650's text 96 menu ("Poke again", "Body slam", "Ask") loops on choice 0 forever: `--choices 1650:96=2` (Ask) gives 6700.
- 1651 and 1758 are mazes (the Iifa Tree roots). Their exit regions only write `Global Int16 2` and re-enter the field; `JumpSwitch` over that state in entry 0 decides which two regions exist. 1651: exit 8, 6, 4, 2 reach 1653 (`--route 1651>1653`); 1758: from state 12 the exits 3, 5, 7, 9, 11 reach 1757 (`--route 1758>1757`). The explorer takes an exit back into the field when no exit leads to the route target (`pick_exit` fallback in `tantalus-trace.rs`), and no longer records a maze step as an exit that "led away" from another floor.
- 1652: text 136 menu ("Touch it again", "Try standing on it", "Ask"): `--choices 1652:136=1`. 6710 (1751 to 1754), 6720 (1754), 6730 (1755).
- 1755 (EVT_EVA2_IU_SDV_0) is a scene of talks: entries 18 and 22 (the dialogue states in `Map Int16 28`), the region talk 13 (state 34, `--region-talks 1755`, `--skip-talk 1755:12` because region 12 is "go back"), and region 11 (a plain region that fires once `Map SByte 37` is 1, set at the end of state 41). `--retalk` now also makes a region entry of that field retriggerable after each new talk: `--retalk 1755:11,1755:13,1755:18,1755:22 --skip-talk 1755:12,1755:17`. The scene ends at 6740 (1756) with 6790 in 1757; the moogle room 1759 is a dead end at 6795 and the story goes back out (1757 exit 6 to the world map, `--world-exits 9003:34@6795` is Madain Sari).
- Madain Sari at 6795 -> 6800 -> 6810 (kitchen scene, 1608) -> 6840: 1605's fight (battle 132, `EVT_BATTLE_MS_E075`, Zidane alone against the red-headed man, 18985 HP, no `NoGameOver`) is lost by the stand-in policy (Zidane has 4583 HP, did 2800 damage in 4600 ticks): `--win-battles 132` for now; route `1600>1601@6810,1601>1603@6810,1603>1605@6810`. After it the scenario is 6840 and the village loop goes 1603 -> 1601 -> 1602 -> 1606 -> 1607 -> 1608.
- 6840 in 1608 (`EVT_SARI1_MS_SCR_0`, the 6850 scene) was an engine bug, now fixed: entry 15's function 1 waited forever at `WaitAnimation` after `PlayAnimation 3764` because an earlier branch had set `SetAnimationFlags` 2 (loop) and the actor then faced another actor with `TurnTowardObject`. In the reference `StartTurn` (DoEventCode.cs:3340) sets `animFlag = 0` and plays the turn clip whenever the turn needs more than 5 frames, which ends the leftover loop; only `StartActorTurn` did that here, `TURNA`, `TURNTO` and `TURNDS` did not (they share `FieldHost::start_turn` now, with a unit test). Chain122 is unchanged.
- Battle 132 (Amarant) stays on `--win-battles 132`: Zidane alone with the stand-in policy (attack only, no items, no Trance) loses it, and nothing in the engine's damage or AI looked wrong (a level 30 Zidane takes about 1100 ticks to fall while Amarant loses 300 HP); not investigated further.
- 6850 -> 6860: the scene plays in 1602 (`EVT_SARI1_MS_EKH_0`): region 8 is initialized at 6850 and starts entry 9 function 14 (Eiko hears the song), so `--region-talks 1602`; its menu (text 33, entry 1) goes to 1610 with `--choices 1602:33=0`. 1609 and 1610 (`EVT_SARI1_MS_SSD_0/1`, the Eidolon Wall at sunset) play by themselves, 1610 sets 6860, then 6870 (1606, 1610, 1605, 1604) and 6890 in 1600.
- 6890 -> 6900: the world map exit to the Iifa Tree is again `--world-exits 9003:37@6890` (1757). 6910: 1757 text 289 menu ("Change party members", "Go to menu", "Keep"): `--choices 1757:289=2`; 1658 and 1758 (maze, exit 4 to 1657) as before. 6920 to 6930: 1654 text 196 menu ("Organize party", "Menu", "Finished"): `--choices 1654:196=2`. Battles 115 (6950, 6960) and 114 (1655) are won by the stand-in policy.
- 1655 (`EVT_EVA1_IF_PTS_0`, the climb down the trunk) ends by distance: Map Int16 32 counts up while `KeyState(Held)` 192 (down or left) is held and the exit to 1656 needs it above 134: `--hold 1655:192`. Then 1656 (6970), 1662, 1658 (6980), 1659 (6990) and `ChangeField 1800`.
- 1800 (`EVT_ALEX3_AC_OHAKA`) runs `ChangeDisc` (-30956: field 1812, disc 2, which is disc 3) and the chain goes on into 1812, 1861 and the Alexandria streets 1850 to 1865 (7010 to 7020), with no error; the run stopped at the field limit, not at a blocker. Saved worlds (local only): 1610, 1654, 1655, 1658, 1757 (6910), 1800 and 1812 under `local/probe/story17-worlds/`, with `go2.sh` (the full command from `w1608.json` at 6810 through 6990 and the disc change).

Disc 3, Alexandria (story18; explorer options only, no engine change; chain122 unchanged). Start `--start 1812 --load-world <w1812>` (the world saved at the disc change), scenario 6990. Fields are `EVT_ALEX3_*` (ids 1800 to 1866 in `FF9DBAll.Events.cs`: 1850 street A, 1851 B, 1852 C, 1853 center, 1856 gate, 1861 pub, 1866 wharf, 1803 guard, 1806 hall 2F, 1807 entrance 2F, 1811 outer E, 1815 outer C2, 1816 outer C9, 1823 hall 2F_2).
- 6990 -> 7010 -> 7020 -> 7030 -> 7040 -> 7060: the explorer walks 1812 -> pub 1861 -> streets -> 1854, 1865, 1864 (7030) -> princess room 1802/1821 (7040) -> guard 1803 (7060) -> hall 1806, with no route or choices needed.
- 1806 at 7060 (`EVT_ALEX3_AC_H2F`) is an explorer artifact, not an engine gap. The player starts at (1620, 107) by the east door, but the explorer drops the actor at each region's centroid; regions 8 and 13 share a polygon at the south end whose guard script (entry 13, "Where am I going?") walks the actor back and fires again every 120 ticks, so the run sat there for 12000 ticks. `--wall-regions 1806` lets the explorer treat the guard as a wall after a few refusals: 1806 -> 1807 (entry 31, the scene that sets 7070) -> 1815 -> 1861 (the pub scene sets 7100) -> streets.
- 7100 -> 7200: the wharf, 1866 (`--route "1850>1851@7100,1851>1852@7100,1852>1853@7100,1853>1866@7100"`, 2800 ticks) sets 7200 by itself. The streets are all visited at 7100 and 7200 with nothing else to do.
- 7200 -> 7300 (fixed by story19, engine): 7300 is set by field 1816 (`EVT_ALEX3_AC_OUT_C9`), entered from 1823 (`EVT_ALEX3_AC_H2F_2`, Garnet's dressing scene), which the explorer reaches by `--start 1811` (outer E). In 1823 entries 3, 13 and 18 wait for `ReadObjectMember(16, 1) <= -1400` (Garnet, entry 16, on the gallery at height 1499) and Garnet waits at a barrier they release. Garnet is placed by `POS3` (height 1) and `SetPathing(1)`, then walks (-799,748), (-1280,677), (-1682,338), (-1525,-216) up the spiral ramp (floor 2, heights 61 to 863) and on to (-299,-540), where the gallery (floor 0, 1499) lies over the hall (floor 3, 1). Two engine bugs kept her in the hall: the player's scripted walks never took the walkmesh height, and a step of any pathing actor took whichever triangle held the new x/z (the hall, once the straight line leaves the ramp) instead of crossing linked edges (`ServiceChar`, `WalkMesh.BGI_traverseTriangles`). Now `FieldNavigation::step_along` keeps the any-floor clamp when its triangle is within three linked edges of the one under the actor (`FindAdjacentTrianglesRecursively`) and otherwise walks the edges along the step, stopping at an unlinked edge (the walk slides along the ramp wall); edge links come from shared world end points. The player follows the walkmesh in a scripted walk when a script set its height (`POS3`/`DPOS3`/jump, `placed_unchecked`). The scene ends, 1816 sets 7300, and the run goes on: 1816 -> 1861 (pub) -> 956 (Gargan Roo) -> 2054 -> Treno (`EVT_TRENO2_*`, 1900 to 1915), where 7550 is set.
- Chain: this changes chain122; the new baseline is chain123. It has the same 164 records, maps, exits, scenarios, texts, battles and party. Only the player's height in `control_state` differs in 13 records (63, 153 three times, 154, 204 four times, 206, 312), now on the walkmesh where chain122 had heights on no triangle (153 at (30, 777): 408, where the triangles are at 1 and 1499; 312 at (57, 1945): -23, where the one triangle is at -862), plus operation counts in 153 and 259 and texts 5 ticks later in 307. A player placed by `POS` can still hold the 32768 default height (153's Zidane does through its whole first scene; the cause is not traced), which the explorer reads as any floor. Following the walkmesh for those players too changes the flow of 153 (it loops on texts 120 to 123), so that is left open.
- Patching `global_memory.bytes[0..2]` of a saved world (the scenario counter, little endian) is the quick way to start a later step; `local/probe/story18-worlds/patch.py` does it (`--load-world` restores the saved scenario over `--set-global`). With 7300 forced, 1816's play scene (the Tantalus troupe, texts 386 to 390) runs but did not end inside 12000 ticks from that unnatural start.
- Saved worlds (local only): `local/probe/story18-worlds/` w1850 (7010 entry), w1806 (first entry, 7030; `w1806_7060` has the counter set to 7060), w1866 (`w1866_7200`, `w1850_7100`, `w7300` are counter-patched copies). `go3.sh START WORLD MAXFIELDS [args]` is the full command (the options of `go2.sh` plus `--wall-regions 1806`); the Alexandria run is `go3.sh 1812 w1812.json 40`, then `go3.sh 1850 w1850_7100.json 8 --world-exits 9000:4 --route "..."` for the wharf. Not reached: Lindblum, Esto Gaza, Ipsen's Castle, the shrines, Terra, Bran Bal, Pandemonium and the disc 3 to 4 change.
- Battle 132 (Zidane alone against Amarant, `EVT_BATTLE_MS_E075`): the enemy AI is the original script, run as is (`ai_script true`). Its entry 1 function 1 ends the fight with battle code 33 = 1 (victory, after a special attack) once Amarant's HP is below 10000 (`Member(36) < 10000` at offset 125), so the fight is a damage race: 18985 HP at start, 8985 damage to deal. The attack roll (`strength + random16 % (1 + ((level + strength) >> 2))`) is the same as Memoria `BattleCaster.SetPhysicalAttack`. `tantalus-trace fight --battle 132 --party 0 --level 30` ends in defeat after 1123 ticks with Zidane dealing 294 and dying at 1711 HP, which fits a level 30 Zidane using only plain attacks. I did not rerun a single hit against the C# end to end (no damage log in the `fight` output), so the claim is: formula entry points match, the script is original, and the loss is the stand-in policy (attack only, no items, no Trance, no skills); `--win-battles 132` stays.

Disc 3, Treno to Oeilvert (story20). Start from the story19 worlds (`local/probe/story20-worlds/`, local only); `go3.sh START WORLD MAXFIELDS [args]` as before, with `CH` for the choices, `WALLS` for `--wall-regions` and `WX` for `--world-exits`. The base command is `W=$(seq -s, 2000 2010); CH="652:90=11,*:3=6,*:4=1,1602:33=0,1757:289=2,1654:196=2,2151:103=1,2151:102=1,2151:102=2@9370,2160:124=1,2110:109=0,2111:359=0,2101:110=1,2173:639=0" WALLS=1806,1807,1821,$W bash local/probe/go3.sh ... --play-card-games --region-talks 1602,2151@192`.

- Treno card tournament (7600 to 7750): `--play-card-games` plays every `MINIGAME` through `CardGame` with the look-ahead player in `tantalus_tools::card_auto` (the five strongest cards; each turn it tries every hand card on every free cell in a copy of the game). It wins opponents 124, 125 and 127. `--win-card-games` sets only the result to a win. Both are off by default and change nothing in the simulation.
- 8000 in 2050 (`EVT_ALEX4_AT_STREET_A`) is not an engine bug. Entry 4 waits for the player's z to go above 6500 or below 3000, a position trigger with no region, so the explorer never walks there. `--waypoints 2050@100:2900` moves the run on to 8340.
- 1807 at 8340 and 8600: exit regions 20 and 21 lead to 1806 only from the gallery (member 1 below -100, height above 100). The explorer now tries another floor of an exit region when the region refuses the player on one floor, or when its dwell runs out and another floor exists elsewhere in the region. 8600 needs `--route "1807>1806@8600,1806>1821@8600,1807>1806@8600,1821>2008@8600,2008>2007@8600,1821>2008@8600"` (Zidane follows Garnet to the altar) and `--waypoints 2006@-2633:-2307` (the SAI_E stairs, height above 900, set 8710). The run then reaches 8800, 9000 (Lindblum 2105) and 9050 (2161).
- Lindblum castle at 9050: the lift switch (2151 entry 3) tests the facing (154 to 248), so `--region-talks 2151@192` turns the player first, and `2151:103=1` picks Upper Level. Route `2161>2162@9050,2162>2163@9050,2163>2151@9050,2151>2168@9050,2168>2170@9050,2170>2150@9050` reaches 9100 (2150), 9150 (2172), 9200 (2169) and the Desert Palace scene in 2211.
- 2211 at 9200 (`EVT_SUNA_SP_KDC`, engine bug, fixed): entry 4 asks entry 5 for function 20 async and then sync in the same step. The VM applied the async request only at the end of the step, so the sync one was accepted at once and left a pending level that never cleared. An accepted request now applies at once (`EventEngine.Call` sets the level immediately), and the run reaches 9250 and 9300.
- 9300 to 9330 (no engine change): GRM 2161 sets 9310 on entry and CFR 2150 sets 9320. 9330 needs items 274, 275 and 276. Get 274 from Cinna in HIG 2114 by walking there (the ATE list in 2170, key 1, only shows the scene). Get 275 from Alice in PLZ 2107 (choice 0 on text 359). For 276, first talk to Michael in ATL 2111 (entry 4, choice 0, sets bit 3546), then press region 7 (`--region-talks 2111 --retalk 2111:4,2111:7`). The air cabs are 2160 `2160:124=1` (Theater District), 2110 `2110:109=0` (Business District) and 2101 `2101:110=1` (castle); `--skip-talk 2102:3` avoids an MST talk that never returns control. Whichever item comes last sets 9330.
- 9330 to 9450: the HFC guards (2170 entries 4 and 5) open the door to CFR once bit 3612 is set, and CFR sets 9350, then MET 9370. The ship leaves from POT_M 2173 (route `2170>2168@9370,2168>2151@9370,2151>2153@9370,2153>2155@9370,2155>2173@9370`, lift `2151:102=2@9370`, `--region-talks 2173`, `2173:639=0`). The Hilda Garde 3 (2855) sets 9400. World map 9003 place 31 (`WX=9003:31`) is the Black Mage Village (1450), which sets 9410 to 9450.
- 9450 to 9600: place 51 on world 9003 is the Desert Palace (2200). `--route "2201>2209@9515,2201>2209@9530"` takes RPW's region 8 to KKR, and `--skip-talk 2203:8` avoids a moogle menu loop. The run reaches 9510 to 9550 and 9600 (KDC 2211). Without the route, the explorer reaches REX's region 3 at 9515 and enters MP1 (2213), whose entry sets 9825 (Cid's later part). That is an explorer artifact, not a story step.
- World map 9004 is the scripted flight to Oeilvert: `EVT_WORLD_WORLD04` has no place switch and changes to 2261 after its own timer. `world_exit` now runs such a script on by itself, for up to 3000 ticks. 2261 sets 9605, and world 9005 place 42 (`WX=9005:42`) leads to 2250, the Oeilvert exterior.
- Furthest: 2250 (`EVT_OEIL_UV_EXT_0`) at 9605. Control is never enabled there. The player's function 1 enables it only from 9700, and entry 0 tag 1 (which sets 9700) shows no text. This is not traced yet. Not reached: Oeilvert, Esto Gaza, Ipsen's Castle, the shrines, Terra, Bran Bal, Pandemonium and the disc 4 change. The 32768 placeholder height did not come up on this route.
- Chain: re-baselined as chain126 (from chain125). The VM change moves five records (fields 257, 261, 262, 312): the same texts in the same order, 16 to 58 ticks later. Field 257 (`EVT_EFOREST1_EF_DEP_0`) at offset 486 asks entry 15 for function 13 (`RequestScriptWhenReady`), then at 496 for function 14 (`RequestScriptSync`), and the sync request now waits for the first to return. The explorer's floor change moves field 164 (it leaves for 165 at tick 505 instead of 624). A bisect build without the VM change gives chain125 except record 34.

Disc 3, Oeilvert to the Desert Palace (story21). Start from the story20 worlds (`local/probe/story21-worlds/`, local only). `go5.sh START WORLD MAXFIELDS [args]` runs the story20 base command (`go4.sh`) with the Oeilvert aids below. `WINB` sets `--win-battles` (default 132,465), `RT` adds region talks, `RTE` adds routes and `SKIP` adds skipped talks.
- 9605 to 9700 (engine bug, fixed): the world map shows a continent title for scenarios 2400, 5990, 9605 and 9890 (`ff9.w_worldSystemConstructor`, `w_naviTitle`). When the title fades in (`w_frameCounterReady > w_naviFadeInTime`: 20, or the `RunWorldCode 11` value plus 5), it writes `w_frameScenePtr + 10` to the scenario counter. `w_frameScenePtr` is the counter as it was when the world loaded. 9605 becomes 9615, and 2250 (`EVT_OEIL_UV_EXT_0`) needs 9615 to 9699 to run its pan and set 9700. No field script assigns 9606 to 9699. `WorldScript` now keeps the title, and a world exit taken before the fade-in still shows it. On the chain this turns 2400 into 2410 when world 9000 first loads, as in the game.
- Oeilvert aids (explorer options only): `--skip-talk 2250:4,2259:8` (moogle menu loops), `--region-talks 2253,2254,2255,2256,2257,2258,2260`, routes `2254>2256@9735,2256>2257@9735,2252>2259@9745,2259>2255@9750`, and `--win-battles 465,0`. Battle 465 is BSC_UV_R008: Epitaph petrifies the stand-in party. Battle 0 is the Ark (UV_E052). The run sets 9705, 9710, 9715, 9720, 9725, 9735, 9740, 9745, 9750 and 9790 (FDP, after the Ark).
- Desert Palace, second visit: 9800 (2200), 9805 (2202) and 9810 (2201). Add 2201 to `WALLS`: its region 9 is Cid's "no time" refusal, and the explorer kept firing it. `RTE="2201>2204@9810"` leads to 9812 in 2204 (`EVT_SUNA_SP_RFG_0`, the frog Cid key chase).
- Walk loop count (engine bug, fixed): `MoveToward_mixed_ex` ends a walk on a frame that leaves `Actor.loopCount` at 0. `CLRDIST` sets the count to 255, and `MOVT` and `AMODE` set it to their own value. In 2204 each Circle press (`ResetWalkState`, `SetAnimationLoopCount 1`, `WalkActor 484 210`) is one 4-unit hop. Before the fix, one press walked Cid the whole way. In 2204, and in 2209 at 9850, an arriving walk does not snap onto its target. Steps there follow the reference: an `angleAsm` heading, a turn limit of `omega` 11.25 degrees, and the whole-unit `GetMoveVector` step.
- `--press-when MAP:INDEX:LO:HI:MASK:ENTRY:ANIM/ANIM` holds a key only while the actor of ENTRY plays none of the listed animations. `--press-when 2204:27:0:0:8192:3:7470/7474` hops only while the creature looks away. Cid then reaches the key without being caught.
- Open blocker, 2204 at 9812: the chase ends when Cid's x reads 485 or his height reads 210 (entry 2 function 12). Height 210 never comes up on this flat floor. Cid now hops to the key and circles it in a fixed orbit that never lands on x 485, and the timer reloads the field. A Python model of the reference step, using `ratan_tbl`, shows the result turns on a single unit of the start position and on the start facing. From (193, -171) at facing 151, the field's own start, the model never lands on 485 either. Some reference detail on Cid's way, not modeled here, must differ: the `FieldMapActorController` walkmesh service or collision. Not reached: the rest of the Desert Palace (9815 to 9890), Esto Gaza, Gulug, Ipsen's Castle, the shrines, Terra, Bran Bal, Pandemonium and the disc 4 change.
- Chain: re-baselined as chain127 (from chain126). Thirty-one records move, all in fields whose scripts walk one frame at a time with `ResetWalkState`, `SetAnimationLoopCount 1`, `WalkActor` and then test the distance again: 102, 150, 55, 200, 202, 203, 204, 205, 207, 307 and 312 (Alexandria followers, the cargo ship crew, the Ice Cavern). These walks used to run to their target in one command. Most records change only in operation counts, plus the final positions and pending walks in 200 to 205. In 307 and 312 the texts come 123 and 10 ticks sooner. The field sequence and every scenario counter are the same. The continent title moves nothing on this route.

Disc 3, the Desert Palace rescue (story22). Start from the story21 worlds; `local/probe/story22-worlds/` (local only) adds `w2204.json` (2204 at 9812), `w2201_9815.json` (2201 at 9815) and `w2217.json` (2217 at 9825). Choices for the 2204 scales: `2204:251=2,2204:262=3,2204:263=3,2204:252=1` (put weights on the dish, leave weight 6 out, then climb). The route after the rescue is `2201>2209@9820`.
- 2204 at 9812 (engine bug, fixed): the chase ends when Cid's x reads 485 while every hop aims at (484, 210). `FieldMapActorController.UpdateMovement` ends every frame of a pathing actor with `UpdateActiveTri`, which rebuilds `curPos` from the triangle's barycentric ratios (`Math3D.CalculateBarycentricRatioXZ`) in single precision. The rebuilt x and z come back a few ulps off whole units ((197, -165) becomes (197, -164.99998)), and `angleAsm` truncates the remaining distance to whole units, so some hop headings differ from the exact model. On exact whole units Cid circles the key forever, passing through (484, 210) itself. With the rebuild he lands on x 485 at hop 243 from his scripted start (193, -171, facing 151). `FieldNavigation::rebuild_on_triangle` follows the reference float order, and `FieldWorld::advance` applies it to pathing actors in the reference-step fields only (2204, and 2209 at 9850). Model: `rfg_float_sim.py` (float32 and float64 variants, one to three rebuilds per hop; every variant lands on 485). After the key, the scales puzzle takes the choices above and the story reaches 9815 (2205) and 9820 (2201).
- 2201 at 9815 and 9820 (engine bug, fixed): each party entry gives itself the player when its uid equals slot 251's (`ReadObjectMember 251 member 5`). `GetObjByUID` turns 251-254 into `partyUID[slot]`, so a member read of a party slot reads that member's own object. The host kept 251-254 as plain uids, no entry matched, and the party had user control with no player.
- Moogle menus past field 1999 (tool bug, fixed): `--choices *:TEXT=N` covered fields 0 to 1999 only, so the Desert Palace moogles (2203, 2220) left their menus open. It now covers every field id below 4000.
- Open blocker, 9825 (Desert Palace dungeon, 2213 to 2222): region 5 of 2222 (`EVT_SUNA_SP_MBS_0`) sets 9830 only with global bits 3567, 3565 and 3560 set. 3567 and 3565 are candles in the hall (2217, `EVT_SUNA_SP_MHL_0`). Each candle is live only when the hall is entered through one door (`Global.Int16[2]` 76 or 87 for entry 11 at (248, 2311), 89 or 94 for entry 12 at (-2793, -1078)), and it lights on a confirm press within reach at the candle's height (-2921 to -2321, and -4607 to -4007). 3560 is the candle of 2222 at (-2764, 2546). The explorer reaches the hall from 2216 (door 76), but its walk to a `--waypoints` point drops the player to the height-0 floor, so the height test fails, and it does not route through the other doors. Presetting the bits in the world (byte 445 |= 225) leaves the explorer stuck in 2217 for 12000 ticks. Not reached: 9830 onward (the palace boss), Esto Gaza, Gulug, Ipsen's Castle, the shrines, Terra, Bran Bal, Pandemonium and the disc 4 change. Not modeled yet and due at 9860 to 9989: `IsEikoAbducted` (`EventState.cs`), which makes the party-slot lookup skip Eiko's entries.
- Chain: chain127 holds byte for byte.

Disc 3, the Desert Palace dungeon to Gulug (story23). Start from the story22 worlds; `local/probe/story23-worlds/` (local only) adds worlds for 2211, 2212, 2207 (9840 and 9860), 2212 (9860), 2856, 2300, 2350, 2362 and 2363. Trace runs pass `--suspend-loops`.
- Candles, 9825 (not an engine bug, route aid): the hall waypoint (248, 2311) lies only over the height-0 floor, so the explorer's jump to it lands there, as the tool picks the floor nearest the current height. The reference does not move the player this way. The route takes the hall's upper floors instead: `--route 2217>2216,2216>2217,2217>2222,2222>2217,2217>2222`, `--waypoints 2217@202:2560/-2983:-1164/-1862:3034,2222@-2764:2546` and `--region-talks 2222` (region 5 of 2222 is a talk). Exit 10 of the hall needs a height above 4800; the stair top is (-1862, 3034). The story sets 9830 and 9835.
- 2207 at 9840 (engine bug, fixed): the pad glow (entry 6, tag 29) and the entry's idle loop share `Instance.Byte[0]`, and the loop kept resetting it, so the glow never finished. An object runs one function at a time (`EventEngine.Request` and `Call`, EventEngine.cs:337-372): a requested function holds the object's idle loop until it returns. `EntryScheduler` now suspends the idle loop of an object with a running request, when loops do not run during talks (`--suspend-loops` in the trace). The story then sets 9845 (2209), 9850 (2207) and 9860 (2209).
- 2212 at 9860 (engine bug, fixed): `REPLY`, `REPLYSW` and `REPLYEW` (0x16, 0x18, 0x1A, DoEventCode.cs:235-270) request a function of the object that sent the running request (`getSender`). The VM sent them to its own object, so `RequestCallerSync` waited on itself. Requested functions now carry their sender (`RequestedScript::sender`, `EventVm::set_sender`). The story sets 9890.
- `IsEikoAbducted` (engine gap, fixed): from 9860 to 9989 (`EventState.IsEikoAbducted`) the party-slot lookup skips Eiko's entries when it fills `partyUID`. `party_uid` takes the flag, and `FieldWorld` reads it from the scenario counter.
- Tool aids: `--skip-talk MAP:ENTRY@SCENARIO` and `--save-at MAP@SCENARIO:PATH` apply from a scenario on. Options for this stretch: `--skip-talk 2211:11@9860`, routes `2208>2207@9860,2211>2212@9860,2300>2301@9910,2350>2352,2352>2351`, world exits `9006:0` (the flight to 2856, 9910) and `9003:48` (Esto Gaza, 2300), `--choices 2351:239=1` (pull the Gulug lever down three times, global bit 3263), `--hold 2351:64,2361:64` (the rope) and `--win-battles 267,195` (a random battle and the 2361 event battle, two enemies at 8000 HP). Esto Gaza sets 9950 (2301); the story then goes 2302 to 2305, then Gulug 2350, 2352, 2351, 2361, 2362, 2365, 2362 and 2363.
- Open blocker, 2363 at 9950 (`EVT_GULUGU_GV_PW4_0`, after Kuja's lines 174 to 177): entry 0 waits for object 41's request (tag 40) to end. Tag 40 sets `Map.Byte[34]` to 24 and counts it down by one a frame (a one-frame walk toward object 13 each time) until it reads 0. Two shared scripts write the same byte at the same time: entry 8, run by object 46, loops forever, raising it by one a frame and resetting it after 9; entry 35, run by object 34, raises it by one a frame until 38. Net, the byte rises one a frame and resets, and tag 40 never reads 0. The frame order here (objects in creation order, then the shared scripts) matches `ObjList` order in the reference, so some reference timing that ends entry 8 or entry 35 sooner is not yet found. Also found: the host ignores `EndSharedScript` (0x45, `ENDSEQ`); entry 0 and entry 13 of 2363 use it, but on their own shared scripts, not on these two. Not reached: 2364, 2357 (9990), Lindblum (10000), Ipsen's Castle, the shrines, Terra, Bran Bal, Pandemonium and the disc 4 change.
- Chain: chain127 holds byte for byte on the final bins build (the idle-loop hold is off on the chain, which runs without `--suspend-loops`).

Disc 3, Gulug to the Hilda Garde 3 (story24). Start from the story23 worlds; `local/probe/story24-worlds/` (local only) adds `w2850.json` (the Hilda Garde 3 bridge at 10400), `w2169.json` and `w2169_10100.json` (Lindblum MET). The run is `RT23=2363 WINB=132,465,267,195,200,198 RTE23="<route below>" source env23.sh; go5.sh 2363 w2363.json 45 --suspend-loops`.
- 2363 at 9950 (engine bug, fixed): the host ignored `EndSharedScript` (0x45, `ENDSEQ`). `EventEngine.DoEventCode.cs:1405` disposes the calling object's shared script (`FindObjByUID(gExec.uid + UID_OFFSET_SEQ)`), and `WAITSEQ` then finds none. After battle 835 the main script requests function 15 of objects 46, 15 and 16, and entry 46's function 15 runs `ENDSEQ` on its glow loop (entry 8, started by function 27 before the battle). In the reference the loop stops there. Here it kept running, raising `Map.Byte[34]` each frame and resetting it above 8. Entry 35 (object 34's shared script, which only ends once that byte passes 37) then never ended either, and object 41's countdown (tag 40, `--Map.Byte[34] != 0`) never reached 0. This holds in either frame order: with both scripts running, the countdown always reads 2 or more. The host now ends the shared script (`FieldWorld::finish_shared_script`, so `WAITSEQ` goes on), `DirectorStep::ended_shared_scripts` lets the scheduler drop its VM, and `FieldScene::step` skips a shared script that a `STARTSEQ` or `ENDSEQ` disposed earlier in the same pass. An `ENDSEQ` inside a shared script ends nothing (there `gExec` is the `Seq`). Test: `field_session::tests::end_shared_script_stops_the_object_s_shared_script`. No route aid: the countdown does not depend on player input.
- The story then goes 2363 -> 2364 (MG1) -> 2357 (HLD, 9990) -> Lindblum 2150 (CFR, 10000) -> 2161 (GRM, 10010). It needs `--win-battles 200,198` (the event battles of 2363).
- Lindblum 10010 to 10100 (route aid): the next scene is in 2169 (`EVT_LIND3_CS_LB_MET_0`, its init at 10010). Only region 6 of 2170 (HFC) leads there by walking; the other `ChangeField(2169)` calls are Active Time Event returns. Route: `2161>2162@10010,2162>2163@10010,2163>2151@10010,2151>2168@10010,2168>2170@10010,2170>2169@10010`. MET's scene goes to 2114 (HIG, 10030), back to MET, then 2157 (HNG, 10050) and MET (10100). The story then moves on by script to Alexandria (`EVT_ALEX5_*`, 2400 to 2457), 1800 (`EVT_ALEX3_AC_OHAKA`, 10400) and the Hilda Garde 3 bridge 2850 (`EVT_HILDA3_G3_BRG_0`, 10400).
- Stops at 2850: its next map is world map 9007 (the Hilda Garde 3 flight), and no `--world-exits 9007:<place>` is known yet for Ipsen's Castle (2500, `EVT_IPSEN_IP_EXT_0`, 10500). Next step: try places 0 to 130 on 9007 from `w2850.json`, as story20 did for 9003. Not reached: Ipsen's Castle (10500 to 10595), the shrines (10620 to 10700), Terra (10830 to 10890), Bran Bal (10900 to 10925), Pandemonium (10930 to 10995) and the disc 4 change.
- Chain: chain128 replaces chain127 (see `local/probe/chain128.reason.txt`). The map sequence, scenario counters and 164 records are unchanged. Five records (fields 116, 62, 166, 51, 65) differ in VM and operation counts because shared scripts that the reference ends with `ENDSEQ` now end. Field 62 also ends 5 ticks sooner.

Story-route stall at field 451 and the cargo ship fight (battle worker 7). `audit/story-route.sh` printed STORY-ROUTE-DIFFERENT at 451 (Dali barrel scene, 12000 ticks, only Steiner, "Waiting ActorAnimation"). Root cause, found by tracing the host's animation events in a good run (01ffe2b) and a bad run: entry 10 of `EVT_DALI_A_AP_AIR_0` plays animation 582 with the loop flag set (`SetAnimationFlags 2`), and the state machine later reaches `TurnTowardObject` plus `PlayAnimation 8198` plus `WaitAnimation` without clearing it. `StartTurn` (DoEventCode.cs:3339) clears `animFlag` only for a turn long enough to need its turn animation (more than 5 frames); when the actor already faces the target it returns early and the loop flag stays, so `WaitAnimation` never ends. Whether the turn is long depends on the actor's facing when the command runs, which follows the pace of the scene (text windows closing under the explorer's confirm presses), so any change in fight length moved it. Not a battle bug; main's "Field turns: TURNA, TURNTO and TURNDS restart the animation flags on a long turn" (45f428c) changed the turn rule and the route passes 451 again (verified on 7606a55: 451 takes 2612 ticks and the route goes on). The route then ended at field 501: battle 296 (CA_E013, Black Waltz No. 3, 11128 HP) is lost by a party that fights at real levels (max HP 233, 158, 305) because the route skips the random battles; the scene ends by an event once his HP is low enough (`tantalus-trace fight --battle 296 --party 0,1,3 --level 22 --gear`: result 5 after 1428 ticks, party alive). `audit/story-route.sh` now passes `--win-battles 296` in part 2, as the Fossil Roo and Madain Sari probes do for their boss fights; the map sequence and scenario counters are unchanged and only six tick counts moved (fight lengths in 307, 310, 312, 352 and 567, and 451), so `audit/story-route-expected.json` was regenerated.

Battle support abilities (same worker): `Battle::new` now runs the permanent-status half of `btl_init.InitBattleData` (`btl_init.cs:448-462`): the permanent set is put aside and `MakeStatusesPermanent` alters each status in (`btl_stat.cs:176-185`), so Auto-Haste starts the hasted gauge coefficient, Auto-Regen ticks on `OnOpr`, and enemy `AutoStatus` (Haste, Regen, Reflect, Float) now acts as a status instead of a bit `alter_status` refused. HP+10/20% and MP+10/20% (`CharacterAbilityGems.TriggerOnEnable`: `MaxHP + MaxHP / 10`, `/ 5`) raise the battle unit's maximums (caps 9999 and 999; the field menu's own maximum is not changed). Half MP halves `mp_cost_factor`. Machine-level tests: Swallow, Melt, Auto-Haste, Auto-Regen, Half MP in `tests/battle_more.rs`, HP+/MP+ in `field_battle.rs`.

`tantalus-trace fight --corpus --party 0,1,2,3 --level 30 --gear` (856 battle ids, 39 s; the same counts and outcomes before and after the permanent-status change): 400 victories, 132 defeats, 20 enemy flees, 2 scripted `result 5`, 302 `none`. Of the `none` rows 294 are scenes missing from p0data2.bin (test and unused scenes, ticks 0) and 8 time out at 20000 ticks: 75 (BSC_GR_E032) is a party too weak (victory in 206 ticks at level 99); 45 and 51 (BU_R004, BU_R002), 302 (EF_E006) and 941 to 944 (WM_9900 to WM_9903) stay unfinished at level 99 with the enemies unhurt (45 and 51 never leave the opening Event phase, 941 to 944 stay in Event/Event after the first 323 ticks): the battle script waits on something the host does not provide, an open engine gap not looked into yet.

Battle worker 8: script sequences (`BTLSEQ`), HP+/MP+ on the field, the corpus timeouts and the missing scenes.

- Script sequences. `AttackSpecial` (event op 0xE5, `btlseq.StartBtlSeq(GetSysList(1), GetSysList(0), seq)`) now starts the attack's sequence outside the command queue: the caster's cmd[0] takes the attack's data with no command number, nothing is queued and `ReqFinishCommand` never runs. The sequence plays in every phase (Event included), keeps `BtlSeqBusy` (sysvar 25) true until its end, applies the effect at its effect point (`SeqExecCalc` or an effect's `EffectPoint`, skipping untargetable units, `UnifiedBattleSequencer.cs:971-992`), and does nothing when the scene has no sequence for the attack (`SeqData[n] == 0`, `btlseq.cs:89`). A sequence without `Calc`, `Vfx` or `SVfx` applies no effect (the Masked Man closing sequences, battle 336, used to cut a mask's HP from 37 to 29). The sequence's last "Change Stand Animation" op (code 17, `btl_mot.ToggleIdleAnimation`) sets the enemy caster's `bi.def_idle` (member 69) when it ends; the machine now keeps member 69 for the scripts.
- Battles 45 and 51 (BU_R004, BU_R002) open with `QueueSpecialAttack` and wait on sysvar 25 before `SetBattleData` 35: they now leave the Event phase and are victories in 718 ticks. 941 to 944 (Ragtime Mouse, WM_9900 to WM_9903) wait for the mouse's stand animation to change after the question sequence: now they end by script (battle code 33 = 7, reported as `enemy_flee`) in 590 to 652 ticks.
- Battle 302 (EF_E006, Prison Cage and Garnet) is a corpus setup gap, not an engine one: the script needs Zidane to Trance, which needs global 16 (`TRANCE_GAUGE_FLAG`), 0 in the corpus default. With the flag on it is a victory in 779 ticks, and the new-game chain (which has the flag) wins it.
- HP+/MP+ (SA 5 to 8) now raise the field maximums as `FF9Play_Update` does: max = the level's basis, then each enabled ability's `Permanent` formula in `saExtended` order (HP+ `MaxHP + MaxHP/10`, HP+20% `MaxHP/5`, the same on MP; they stack), capped at 9999 and 999, current HP and MP cut to the new maximums. Toggling an ability in the ability menu and a level-up both rerun it. The battle no longer adds them a second time. `support_enabled` holds `SupportAbility` indexes (0 to 63, as `player.saExtended`) everywhere: the ability menu, the Steam save bits and the battle loadout (which used to subtract 192).
- Missing scenes: 294 corpus ids name scenes that are in no StreamingAssets bundle (UnityRaw, uncompressed, so a raw byte search is valid) and no FF9_Data file; p0data2.bin holds exactly 562 `evt_battle` scenes. They are 176 `B3_*` and 100 `CAM_*` test scenes, the dev scenes (YASU00 to 05, TEST03/04, TESTK, TESTSCN, SIOTES00/01, QTARO1, NOFUJII) and four cut random battles (UV_R002 16, FR_R010 90, IF_R009 125, EF_R006 171); only a text list in resources.assets still names some of them ("16,uv_r002"). They stay `none` with 0 ticks.
- Corpus (`tantalus-trace fight --corpus --party 0,1,2,3 --level 30 --gear`, 856 ids): before 400 victories, 132 defeats, 20 enemy flees, 2 `result 5`, 302 `none`; after 402 victories, 132 defeats, 24 enemy flees, 2 `result 5`, 296 `none` (294 missing scenes, 75 too strong for a level-30 party, 302 without the Trance flag). No other battle changed outcome.
- New-game chain: re-baselined as chain124 (from chain123). Six records differ, all from script sequences now holding sysvar 25: records 2, 43, 49, 50, 59 (battles 336, 337, 335, 334, 302) reach Close 46 to 151 ticks later with the same outcome and HP; record 158 (battle 21, IC_E011, Zidane alone) runs its opening sequence 10 with its effect, which shifts the fight's random draws: still a victory, 8290 ticks instead of 6991. The HP+/MP+ change does not touch the chain (it never enables a support ability).
