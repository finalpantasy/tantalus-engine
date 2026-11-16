# Whole-game coverage ledger

Scope: the owned PC install at `E:/SteamLibrary/steamapps/common/FINAL FANTASY IX`, read-only. This lists what the install contains, where it lives, and how much of it Rust parses or runs today. It is an inventory, not a gameplay claim. Real saves and real-game execution stay NOT_RUN. Machine-readable copy: `audit/game-coverage.json`.

| Domain | Where | Count in install | Rust status today |
|---|---|---|---|
| Fields (background, walkmesh, atlas) | `p0data11`-`p0data19` (mirrors `p0data101`-`p0data181`) | 674 field directories | all 674 load: scene, navigation and atlas parse and the navigation mesh builds. Rendering and play proven on one field only |
| Field, battle and world scripts | `p0data7` `eventengine/eventbinary`, 7 languages | 1395 scripts (818 field, 562 battle, 15 world) | 42,581 functions decode; 1 fails (expression operation 12). Dry-run: 35,300 complete, 6,956 wait on host state, 36 stop at the instruction budget, 2 stop on the unsupported WATCH opcode |
| Field and movie config | `p0data7` `mapconfigdata`, `mbgdata` | 818 map configs, 56 movie backgrounds | all parse (`--map-config-all`, `--mbg-all`); unread bytes kept raw |
| Battle scenes and models | `p0data2` | 562 scenes, 178 info files, 6,243 model files | scenes, info files and sequence headers parse (`--battle-scenes`); each scene has an unread tail; `.raw17` enemy action sequences decode (3,814 slots, 0 unknown opcodes, `--battle-sequences`); 560 of 562 scenes resolve every model, clip and background (`--battle-model-coverage`); 372 `ef<id>` spell-effect blobs parse as containers, script bodies are native MIPS code (`--special-effects`) |
| World map | `p0data3` | 3,439 assets | all classified; 2,178 block meshes decode; raw `wmap` packs account for every byte (`--world-map`); `.hed`/`.nwp` tables kept raw |
| Character and object models | `p0data4` | 5,252 assets, 525 model directories | 522 assemble to render-ready skinned models (`--assemble-all`); 3 hold only textures; all 345 field model ids and all 38 player battle model references resolve once `ModelFactory` name rules are applied (`audit/model-gap-analysis.md`) |
| Animations | `p0data5` | 9,278 clips for 702 models | all clips parse; 7,930 of 7,939 paired clips match their model skeleton |
| Audio | `p0data61`-`p0data63` and mirrors | 4,029 sound assets | all classify as AKB2 with Vorbis; headers parse (`--audio-all`); all 4,029 decode to PCM with `lewton` and report loop points (`--audio-decode`, `audit/audio-decode.json`); no mixer or device output |
| Movies | `StreamingAssets/ma` | 60 files | all parse as single-stream Theora 1280x960 at 15 fps with no audio stream; frames decode through the pure-Rust `oxideav-theora` crate and match ffmpeg bit for bit on FMV000 and mbg119 (`--decode-frames`); FMV000 declares 739 frames in its granules but holds 700. Movie audio is separate sound assets: the `GetMovieSoundIndex` table is ported and 61 of 66 FMV/MBG rows resolve to assets (`audit/audio-decode.json`) |
| Data tables, UI, fonts, text | `resources.assets`, `sharedassets*` | 59 files, 5,291 TextAssets | inventoried (`inspect-serialized`); 4,537 `.mes` files parse; vanilla battle, item, ability, shop and level tables read from `Assembly-CSharp.dll` static initializers (17 tables, `--battle-tables`); dialog font decodes (TrueType in `p_fa.mpc`) and one dialog page renders on the CPU |
| Saves | user save folder | not inventoried | legacy import is T10, NOT_RUN |

## Event decode gaps

1 function in 1 event fails to decode:

- expression operation 12 (`B_SINGLE_PLUS`): `EVT_GARGAN_GR_GGT_0`; Memoria pushes a stale value for it, so it stays unknown.

Two opcodes were closed in this pass. `BSACTIVE` (`0x102`) now decodes and emits a host event. `WATCH` (`0x4E`) decodes as a label plus one variable expression, but it refuses to run: the pinned source does not define its behavior. The dry run reaches that refusal in 2 world events (`EVT_WORLD_WORLDSV`, `EVT_WORLD_WORLDTS`).

Two extended opcodes stay recorded as unknown: entry 13 function 1 `0x180` and entry 16 function 1 `0x1C8`.

447 names in Memoria's event table are not present in this bundle (294 battle, 57 startup, 20 SPS and others); they are listed in the JSON. The dry run has no other runtime errors.

## Reproduce

- `tantalusctl assets inspect-bundle --path <p0dataNN.bin> --inventory`
- `tantalusctl assets inspect-bundle --path <p0data7.bin> --event <name>.eb --probe --json`
- `tantalusctl assets inspect-bundle --path <bundle> --field <UPPERCASE_NAME> --json`
