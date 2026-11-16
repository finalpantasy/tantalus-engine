# Model resolution gap analysis

Scope: why the field and battle coverage audits reported missing models, textures and clips, what was fixed, and what stays open. Data only; nothing here says a model renders or plays.

Sources: pinned Memoria `c716e1fa` (`Global/Model/ModelFactory.cs`, `Global/Animation/AnimationFactory.cs`, `Global/ff9/Battle/FF9BattleDB.*.cs`, `Global/ff9/FF9DBAll*.cs`) and the Steam install's `p0data*.bin`, `mainData`, `resources.assets`, `level*` and `Assembly-CSharp.dll` (the `Modding/` folder was ignored). The installed assembly's `GEO` (710 rows), `upscaleTable` (174), `revertUpscaleTable` (107) and `AnimationDB` (14,111 rows) were evaluated with the repo's `managed_il` evaluator and equal the pinned source, in the same order.

## Counts

| Item | Before | After |
|---|---|---|
| Field model ids that assemble | 320 / 345 | 345 / 345 |
| Field animation ids with a clip | 3,144 / 4,197 | 4,197 / 4,197 |
| Fields with an unresolved item | 817 / 818 | 0 / 818 |
| Battle enemy models (distinct geo ids) | 168 / 182 | 182 / 182 |
| Battle player model references | 14 / 38 | 38 / 38 |
| Battle enemy animations (distinct) | 728 / 729 | 728 / 729 |
| Battle sequence animations (distinct) | 2,022 / 2,023 | 2,022 / 2,023 |
| Battle background textures that decode | 19 backgrounds held undecodable textures | all 38 decode |
| Scenes fully resolved | 547 / 562 | 560 / 562 |
| Undecodable model textures in `p0data4` | 2 | 0 |

## (a) Wrong name-to-path mapping: confirmed, main cause

The audits looked for `Models/<type>/<geo id>` using the id from the script or record. `ModelFactory.CreateModel` does not do that. It resolves the geo name in three steps:

1. `upscaleTable[name]` replaces the name when a row exists (174 rows). The directory type is the second token of the replaced name.
2. `revertUpscaleTable[name]` (107 rows) maps the replaced name back to an `FF9BattleDB.GEO` name; if there is no row, `GetNameFromFF9DBALL` applies special cases for `GEO_MAIN_UP3_ZDN` to `UP5_ZDN` and `UP4_GRN`.
3. `GetGEOID` returns the first key of the resulting name (`TwoWayDictionary` keeps the first key for a duplicate value), with fixed ids 347 for `GEO_MON_B3_110` and 5461 for `GEO_MON_B3_109`. The path is `Models/<type>/<id>/<id>`. `GEO_WEP*` uses type 6; ids 429 and 430 force type 5.

Examples from the install:

- Battle Zidane `GEO_MAIN_B0_000` becomes `GEO_MAIN_UP0_ZDN`, then `GEO_MAIN_F0_ZDN`, then `2/98`. The party battle models are the field models; `2/98` carries a `battle_model` child that `CreateModel` shows in battle and hides in the field. This is why 24 player references looked absent in every container: the audit searched for ids that are never loaded.
- `GEO_MAIN_F3_ZDN` to `F5_ZDN` also load `2/98`; the game then swaps textures from `Models/2/668`, `669`, `670`. Those three directories hold textures only, so they have no root object (the "3 lack a root" in the field audit).
- Duplicate names: 5461 is listed before 33 and 347 before 201 for the same names, so the first key picks the directory.

Evidence of the fix: after applying the tables (`crates/tantalus-assets/src/pc/model_factory.rs`), field models resolve 345/345, battle enemies 182/182, players 38/38. The 25 field ids and 14 enemy ids that looked absent all load a directory that exists in `p0data4`.

Animations had the same kind of gap (e). Folder choice follows the `GEO` first key of the `GEO_<a>_<b>_<c>` part of the clip name, with `FOLDER_OVERRIDES`; unchanged.

## (b) Objects in another container: ruled out

All 31 containers were scanned for model GameObjects. None outside `p0data4` hold any. The `ResourceManager` (class 147) container in `mainData` has no `models/` entries, and the standalone `resources.assets`, `mainData` and `level*` files hold no AnimationClips. No missing item was found elsewhere.

## (c) Genuinely absent: two items only

Two animation clips remain missing, and both exist in no `p0data5` folder or anywhere else scanned:

| Item | Id | Folder | Scene |
|---|---|---|---|
| `ANH_MON_B3_169_002` | 9957 | 671 | UV_R010 |
| `ANH_MON_B3_140_012` | 10159 | 351 | CW_E061 |

Ruled out: wrong folder (searched by clip key in every folder), a different first key (both ids are their own first key), another container, and a fallback in the pinned source (none exists). The audit marks both `in_pattern: true`, so a scene pattern can select them. Why the install lacks them is not known. This is the only reason two scenes stay unresolved (560 of 562).

## (d) Undecodable background textures: reader limit, fixed

The 19 backgrounds were not missing. `Texture2D` handled only a few formats and treated any mip chain as one level. Now supported: Alpha8, RGB24, RGBA32, ARGB32, RGB565, BGRA32, DXT1 and DXT5, with the full mip chain length validated and level 0 decoded. All 38 background textures decode; all 1,891 `p0data2` textures decode; the 2 undecodable model textures in `p0data4` decode.

Caveats: byte orders for the raw formats follow Unity's documented layouts and were checked only by eye on a contact sheet, not against a reference decoder. The 4-bit formats 2 and 13 are not implemented (not checked whether any install texture uses them; all 1,891 `p0data2` textures decode without them).

## (e) 1,053 missing clip ids: first-key rule, fixed

`AnimationFactory.GetRenameAnimationPath` uses the first `AnimationDB` key of the animation name as the clip file name. 4,738 ids share a name with an earlier id; scripts use the later id, but only the first has a clip. Resolving by first key gives 4,197/4,197 field animation ids (3,144 ids are their own first key; the other 1,053 map to an earlier id). Each of the 1,053 previously missing ids maps to a key that has a clip.

Caveat: the first-key behavior comes from the pinned `TwoWayDictionary` source and is confirmed by clip existence for every id. The installed assembly's duplicate handling was not executed.

## Not done

- The `ZDN` texture swap (`Models/2/668` to `670`) is not applied; only the directory is resolved.
- The installed `animationPathTable` (102 rows, populated) is commented out in the pinned source and not used. No id needed it.
- Rendering, skinning of the shared battle child, and placement are untouched.

## Reproduce

```
tantalusctl assets inspect-bundle --path p0data2.bin --battle-model-coverage audit/battle-model-coverage.json
tantalusctl assets inspect-bundle --path p0data7.bin --field-models
```

Both read the install; `audit/field-model-coverage.json` and `audit/battle-model-coverage.json` hold the results.
