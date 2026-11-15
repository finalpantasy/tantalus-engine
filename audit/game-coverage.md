# Whole-game coverage ledger

Scope: the owned PC install at `E:/SteamLibrary/steamapps/common/FINAL FANTASY IX`, read-only. This lists what the install contains, where it lives, and how much of it Rust parses or runs today. It is an inventory, not a gameplay claim. Real saves and real-game execution stay NOT_RUN. Machine-readable copy: `audit/game-coverage.json`.

| Domain | Where | Count in install | Rust status today |
|---|---|---|---|
| Fields (background, walkmesh, atlas) | `p0data11`-`p0data19` (mirrors `p0data101`-`p0data181`) | 674 field directories | all 674 load: scene, navigation and atlas parse and the navigation mesh builds. Rendering and play proven on one field only |
| Field, battle and world scripts | `p0data7` `eventengine/eventbinary`, 7 languages | 1395 scripts (818 field, 562 battle, 15 world) | 42,551 functions decode; 31 fail on 3 expression operations not yet decoded. Dry-run: 35,284 complete, 6,951 wait on host state, 36 stop at the instruction budget, 1 runtime error |
| Field and movie config | `p0data7` `mapconfigdata`, `mbgdata` | 818 map configs, 57 movie backgrounds | not parsed |
| Battle scenes and models | `p0data2` | 562 scenes, 178 info files, 6,243 model files | not parsed (T08) |
| World map | `p0data3` | 3,439 assets | not parsed |
| Character and object models | `p0data4` | 5,252 assets, 805 model directories | not parsed |
| Animations | `p0data5` | 9,278 clips for 702 models | not parsed |
| Audio | `p0data61`-`p0data63` and mirrors | 4,029 sound assets | not parsed |
| Movies | `StreamingAssets/ma` | 60 files | not decoded |
| Data tables, UI, fonts, text | `resources.assets`, `sharedassets*` | not inventoried | not parsed |
| Saves | user save folder | not inventoried | legacy import is T10, NOT_RUN |

## Event decode gaps

31 functions in 9 events fail to decode. These are expression operations, not opcodes:

- operation 258: 6 events, first `EVT_CHOCO_CH_FGD_0`
- operation 12 (`B_SINGLE_PLUS`): 1 event, `EVT_GARGAN_GR_GGT_0`; Memoria pushes a stale value for it, so it stays unknown
- operation 78 (`B_OBJSPEC`): 2 events, first `EVT_WORLD_WORLDSV`

Two extended opcodes stay recorded as unknown: entry 13 function 1 `0x180` and entry 16 function 1 `0x1C8`.

447 names in Memoria's event table are not present in this bundle (294 battle, 57 startup, 20 SPS and others); they are listed in the JSON. One dry-run runtime error: `EVT_LIND1_TN_LB_LDH_0` (InvalidExpression opcode Some(5): assignment target is not writable).

## Reproduce

- `tantalusctl assets inspect-bundle --path <p0dataNN.bin> --inventory`
- `tantalusctl assets inspect-bundle --path <p0data7.bin> --event <name>.eb --probe --json`
- `tantalusctl assets inspect-bundle --path <bundle> --field <UPPERCASE_NAME> --json`
