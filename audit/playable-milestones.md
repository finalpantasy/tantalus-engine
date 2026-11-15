# Playable-game milestones

Goal: a playable game in Rust from new game to ending, using only data from the owned install and source-backed rules from the pinned Memoria checkout. This file turns the coverage ledger (`audit/game-coverage.md`) into ordered milestones per domain. It is a plan, not a compatibility claim. Real saves and real-game execution stay NOT_RUN.

Each milestone lists what it delivers, the source it follows, the data it reads, one check that proves it, and what exists today. Paths under Memoria are relative to `.reference/Memoria/Assembly-CSharp/`. IDs: B = battle, W = world, models and fields, A = audio, text, UI and tables, G = game flow and saves.

## Status on 2026-11-16 (update: merged)

| Slice | State |
|---|---|
| Event VM | 42,581 of 42,582 real functions decode; 42,307 dry-run; 1 decode gap (expression op 12), 2 WATCH refusals |
| B1 battle scene, info, sequence-header parsers | merged; all 562 scenes parse; each has an unread tail |
| A1 inventory and A2 `.mes` text | merged; 59 files inventoried, 4,537 `.mes` files parse |
| A4 `mbgdata` and `mapconfigdata` | merged |
| W3 Unity objects, W4 model assembly, W5 clip binding (data side) | merged; 522 of 525 model directories assemble; CPU sampling and skinning exist; no GPU rendering yet |
| W7 world-map data | merged; all entries classified; raw packs fully accounted; block meshes decode |
| A8 UI atlases (data), W6 field model coverage, G7 save tree schema | merged; 22 NGUI atlases (1,626 sprites) decode; 320 of 345 field model ids assemble; save schema typed with synthetic fixtures only |
| A5 movie measurement, A6 audio container | merged; 60 Theora files with no audio stream; 4,029 AKB2 Vorbis assets; no decoders |
| Everything else below | not started |

## Rules for every milestone

- Source first. A field Memoria does not define stays a named raw field and is listed as unknown.
- Memoria CSV and script files are cross-checks, never game data. Tables come from the install.
- Golden values come from running the reference code or reading the install, never from hand arithmetic.
- No milestone claims gameplay until its check runs against real data.

## B. Battle

| ID | Delivers | Source | Data | Check | Today |
|---|---|---|---|---|---|
| B1 | Scene, patterns, monster types and attack records; `.inb` info; `.raw17` header | `Global/BTL_SCENE.cs`, `Global/SB2/*`, `Global/AA_DATA.cs`, `Global/BBGINFO.cs`, `Global/btlseq.cs` | `p0data2` (562 scenes, 178 info files) | All 562 scenes parse; totals match an independent script | Parsers done on a branch. All 562 parse, but each has an unread tail of 4 to 2048 bytes, so the check is open |
| B2 | Battle models, animations, backgrounds | `Global/btl_init.cs`, `FF9/btl_mot.cs`, `Global/ff9/Battle/FF9BattleDB.*.cs`, `Global/battlebg.cs` | `p0data2` models (6,243 files), `p0data5` clips | Every model and clip a scene references resolves | Not started. Depends on W3 |
| B3 | Action, command, status, item, stat and level tables | `Global/ff9/Battle/FF9BattleDB.cs`, `Memoria/Data/*`, `Global/ff9/FF9ITEM_DATA.cs`, `Global/ff9level.cs` | Probably the installed managed assembly or `resources.assets`; unconfirmed | 192 actions decode with contiguous IDs; spot rows match the install | Not started. Depends on Kimi's `managed.rs` output (A3) |
| B4 | Damage, hit and status formulas | `Memoria/Battle/Calculator/*`, `Global/btl_stat.cs`, `Global/btl_cmd.cs`, `Global/ff9/ff9btl.cs` | B3 tables | 20 seeded cases give exact HP and MP, with expected values produced by the reference code | Synthetic placeholder rules only |
| B5 | ATB timing, command queue, targeting, defend, flee, trance | `Global/battle/*`, `Global/btl_cmd.cs`, `Global/BTL_DATA.cs`, `Global/CMD_DATA.cs` | B3, B4, party state (G4) | A fixed-seed battle reaches Victory on a tick that matches a reference trace | Synthetic turn flow. Reference trace not captured |
| B6 | Enemy AI and battle events through the event VM | `Global/btl_scrp.cs`, `Global/ENEMY/*`, `Global/EBin.cs` | 562 battle `.eb` in `p0data7` | All battle scripts dry-run with zero runtime errors; one boss picks the reference action | VM decodes them. No battle host. 294 names Memoria lists are absent from the bundle; cause unconfirmed |
| B7 | Action sequences, camera, spell effects | `Global/btlseq.cs`, `Memoria/Battle/SFX/*`, `Global/btl_vfx.cs` | `.raw17` blocks; effect data location unknown | One ability emits the same ordered markers and frame counts as the reference | Markers on synthetic data only. The original effect plugin is native code, so rendering may not be recoverable from C# |
| B8 | EXP, AP, gil, drops, level-up, learned abilities, return to field | `Global/battle/Result/*`, `FF9/ff9level.cs` | B1 monster data, B3 tables | One scripted win credits exact rewards once into save state | Gil applies once; experience recorded but not applied; items lack a mapping |

Order: B1, B2 and B3 run in parallel. B4 needs B3, B5 needs B3 and B4, B6 needs B5, B7 and B8 follow B5.

## W. World, models and fields

The Unity bundles have no parser beyond the object table, so W3 gates everything that draws a model. Measure first whether these files carry a type tree.

| ID | Delivers | Source | Data | Check | Today |
|---|---|---|---|---|---|
| W1 | All 674 fields composed in correct layer and camera order, offscreen | `BGSCENE_DEF.cs`, `BGOVERLAY_DEF.cs`, `BGCAM_DEF.cs`, `Field/Map/FieldMap.cs` | `p0data11`-`p0data19` | Every field renders non-empty with no out-of-range sprite; 10 sampled against reference math | `bgs.rs`, `bgi.rs`, `field.rs`, `texture.rs` and one proven field. No batch driver |
| W2 | Animated tiles, scrolling layers, walkmesh toggles | `BGANIM_DEF.cs`, `BGI_ANM_DEF.cs`, `BGI.cs`, `BG_*` opcodes | Field bundles | Frame-hash sequence on 3 animated fields; every animation in all 674 fields consumed with no unknown op | Structs and opcode decode only. No runtime |
| W3 | Unity GameObject, Transform, Mesh, SkinnedMeshRenderer, Material and AnimationClip parsing | Unity format; names from `FF9Char.cs`, `GEOTEX*.cs` | `p0data4` (525 model directories), `p0data5` | All meshes parse with in-range indices and exact byte consumption | Done: all objects in both bundles parse and consume exact bytes |
| W4 | Static models with textures and bind-pose skeleton | `GEOTEX.cs`, `ff9/FF9DBModelParameters.cs` | `p0data4` | Gallery of at least 20 models; bounds match the bone hierarchy | Data side done: 522 models assemble with zero bone or reference failures. No renderer or gallery capture |
| W5 | Clip sampling on the skeleton; walk, run, idle and event animation mapping | `AnimationFactory.cs`, `ff9/FF9DBAll.Animation.cs`, `Anim.cs` | `p0data5` plus EventAnimation text in `p0data7` | Every clip samples at start and end inside a bounds threshold | Sampling and CPU skinning exist; 4,016 of 7,930 clips pass the 0.25 t=0 bounds check, which is a weak test. Animation ID to name table not embedded; 1,339 clips sit in folders with no model in `p0data4` |
| W6 | Player and NPCs in fields, on the walkmesh, behind foreground layers | `Field/Map/Actor/*`, `Event/Engine/EventEngine.cs` | W1 to W5 plus `eventbinary` | Zero actors off the walkmesh across 818 field scripts; capture of one populated field | Binding data done (`--field-models`): 345 distinct model ids, 320 assemble, 22 absent from `p0data4`, 3 lack a root; 3,144 of 4,197 animation ids have clips. No depth sorting or placement check |
| W7 | World map blocks, terrain, textures, sky, water | `WM/WMWorld/*`, `WM/WMBlock/*`, `WM/WMCell.cs` | `p0data3` (3,439 assets) | Full block grid renders with no seams; triangle counts match the parse | Data side done: 2,178 block meshes and 481 prefabs of 24 by 20 cells decode. Not rendered; `.hed`/`.nwp` tables and `map.bin` values unknown |
| W8 | World movement, camera, collision, vehicles, field transitions | `WM/WMActor/*`, `WM/WMScriptDirector.cs`, `ff9/FF9StateWorldSystem.cs` | 15 world scripts, `p0data3` | Scripted multi-block walk matches the Memoria logic; Lindblum round trip works | Scripts decode; 2 stop at WATCH. No runtime |
| W9 | One command that prints pass and fail counts per asset class | none | all of the above | CI runs it | Not started |

## A. Audio, text, UI and tables

| ID | Delivers | Source | Data | Check | Today |
|---|---|---|---|---|---|
| A1 | Inventory of `resources.assets`, `sharedassets*` and `mainData` | `Global/Asset/AssetManager.cs` | `x64/FF9_Data` | Listing in `audit/` whose count matches the object table | Done on `agent/text`: 59 files parse, 5,291 TextAssets in `resources.assets` |
| A2 | `.mes` entry splitter and text-tag decoder | `Memoria/Assets/Text/*`, `FF9TextTool.cs` | 4,537 `.mes` TextAssets | Every file parses; unknown tags listed | Done on `agent/text`: 244,758 entries; 37 unknown tag spans in two files, kept raw |
| A3 | Typed data tables: items, abilities, shops, characters, levels | `Memoria/Data/*`, `Global/ff9/FF9ITEM_DATA.cs` | Installed managed assembly resources | Counts and a known row match the source | Waits on Kimi's `managed.rs` embedded-resource extraction |
| A4 | `mapconfigdata` and `mbgdata` | `Global/MBGDataReader.cs`, `MBG*.cs` | `p0data7` | Every byte consumed | Done on `agent/mbg`: 818 map configs, 56 movie backgrounds. Unread bytes kept raw: 817 map configs have trailing bytes; three movie backgrounds hold fewer frames than their header declares |
| A5 | Movie demux and decode (Ogg Theora) | `Graphics/Movie/MovieMaterial.cs`, `Field/FF9FieldFMVServiceStatus.cs` | 60 files in `StreamingAssets/ma`, 2.7 GB | `FMV000` frame count equals the Ogg granule position; frame 0 matches a reference decode | Container measured: single Theora stream per file, 1280x960 at 15 fps, 2,880.7 s total, CRCs pass. The files carry no audio; Memoria plays movie audio through `SoundLib.PlayMovieMusic`, source not located. No video decoder |
| A6 | Audio container extraction, decode to PCM with loop points | `Global/Sound/Loader/SoundLoaderResources.cs`, `Memoria.Prime/OggFile/AKB2/AKB2Header.cs` | `p0data61`-`p0data63` (4,029 assets) | All assets classify; sample count and rate match the header; loops match metadata | Containers and headers done: all AKB2 with Vorbis at 44.1 kHz; loop points parse. Header sample count is a placeholder on looped assets. No decoder |
| A7 | Mixer, channels, state graph, script sound opcodes | `Global/Sound/*`, `Global/ff9/Snd/FF9Snd.cs` | A6 output, sound metadata from the managed assembly | Sound-play call sequence for one field matches the dry-run expectation | Opcodes decode. No sink or device backend. Library choice still open |
| A8 | Atlases, fonts, dialog box, menus, battle HUD | `Memoria/Assets/Text/GraphicResources.cs`, `Global/Dialog/*`, `Global/battle/BattleHUD/*` | UIAtlas and font objects in the A1 inventory | One atlas sprite and one dialog box match a reference capture | Atlas data done: 22 atlases, 1,626 sprites, none outside their texture. Fonts live in encrypted `.mpc` bundles (UnityWeb, LZMA) that the reader cannot open yet. No layout code |

Order: A1, A2 and A4 first (done on branches). A3 waits on Kimi. A5 and A6 are independent research. A7 follows A6. A8 follows A1 to A3.

## G. Game flow and saves

| ID | Delivers | Source | Check | Today |
|---|---|---|---|---|
| G1 | 2048-byte `gEventGlobal` plus step count, ability usage and script dictionary, shared across fields; zero on new game | `Global/Event/EventState.cs`, `EventEngine.Initialize.cs:51` | Replay the dry run; the 6,956 functions that wait on host state fall | `FieldWorld` has system variables and lists. No persistent global array |
| G2 | Answers for the host queries `FieldHost` still refuses | `EventEngine.OperatorExtract.cs`, `EventEngine.GetSysvar.cs`, `Field/Map/FieldMap.cs` | Completion count rises with zero runtime errors | Gaps at `host.rs` lines 260, 296, 305, 311, 326, 342-362, 455, 480 |
| G3 | New game from `fldMapNo` 70, field-to-field flow, `SetNextMap`, `DISCCHANGE` | `Global/TitleUI.cs:989`, `Global/Honolulu/HonoluluFieldMain.cs`, `FF9StateSystem.cs` | From field 70 with zeroed state, the chain of map IDs reaches the first interactive field | Transitions queue; no loop loads the next field. Field 70's script and its follow-on are not traced |
| G4 | Initial party, stats, inventory, equipment | `Global/ff9play.cs`, `Global/ff9/FF9PLAY_*.cs`, `FF9ITEM.cs` | Party and inventory after new game equal values read from the source | Inventory, gil and party stats exist. Base-stat tables unparsed (A3) |
| G5 | Field to battle to field, with rewards written once | `Global/Honolulu/HonoluluBattleMain.cs`, `btl_result` | One scripted battle returns Victory and updates EXP, gil and items once | Synthetic only (needs B1 to B5) |
| G6 | Field and world travel, world encounters | `WM/WMScriptDirector.cs`, `World/WorldState.cs` | Field 70 to world and back keeps the scenario counter and state | Needs W7, W8 |
| G7 | Legacy save codec for the real tree | `Global/JsonParser.cs`, `SharedDataBytesStorage.cs:297-343`, `SharedDataAesEncryption.cs` | Decode a real `SavedData_ww.dat` read-only and re-encode it byte-identical | Typed tree for all 17 sections with synthetic round trips (`audit/save-tree-schema.md`). No real save exists on this machine; key-sort order and reserved sections unverified |
| G8 | Native save and load with global variables, party, field and position | `Global/SaveLoadUI.cs`, `EventEngine.Initialize.cs` | Save, reload, and the next 100 ticks match an uninterrupted run | Snapshot stores field and world state, not the global array or party |
| G9 | Shops, Tetra Master, Chocobo and Chocograph, Moogle, ending field | `Global/ShopUI.cs`, `Global/Quad/*`, `Global/ChocographUI.cs` | Buy an item, win a card game, reach `FLDSCRPT_EVTNO_ENDING` | Not started |

## Critical path to "new game to first battle"

1. G1 and G2 (global state, host queries): unblocks most waiting scripts. Pure Rust, no new asset formats.
2. W1 and W3 to W6: a field with real models and animation. W3 is the biggest unknown.
3. A2 and A8 text and UI: dialog on screen.
4. A3 then G4 then B1 to B5: a real fight.
5. G3 and G5: new game to field to battle to field.

Saves (G7, G8) and the sound and movie chains (A5 to A7) run in parallel with that path.

## Open unknowns

- Where the movie audio lives, and where the 85 weapon and other geo IDs with clips but no model sit.
- Where the vanilla battle and item tables live in the install (A3, B3).
- The 2048-byte tails after battle scene records and the trailing bytes in map configs and movie backgrounds.
- Where the first real field sits after field 70.
- Opcodes recorded as unknown: entry 13 function 1 `0x180`, entry 16 function 1 `0x1C8`, `STOP` `0x4F`, and expression op 12.
