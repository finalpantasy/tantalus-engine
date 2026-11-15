# New-game chain (milestone G3)

Goal: from a new game (`fldMapNo` 70, zeroed state), follow each field change until the first interactive field. Source: `Global/Event/Engine/EventEngine.Initialize.cs:51-64` (`NewGame` sets `fldMapNo = 70`, zeroes `gEventGlobal`), `Global/ff9/FF9DBAll.Events.cs` (map number to event archive name).

## Tool

`tantalus-trace chain --bundle <install>/StreamingAssets/p0data7.bin --event-db .reference/Memoria/Assembly-CSharp/Global/ff9/FF9DBAll.Events.cs --atan-table <ratan_tbl.bin> --max-fields 15 --ticks 4000`

It runs each field's archive under the orchestrated `FieldDirector` (each entry's tag-0 function, requested tags spawned on demand, as in `tantalusctl --orchestrate`), stops a field at its first `MAPJUMP` (`world.field_transitions`), and carries `FieldWorld` into the next field. The EventDB table is read from the pinned Memoria source at run time and is not copied into the repository. `tantalus-trace disasm --bundle <p0data7.bin> --event <NAME>.eb [--entry N]` prints decoded instructions.

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
