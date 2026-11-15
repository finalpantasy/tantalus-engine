# Legacy FF9 save tree schema (milestone G7)

Reference: pinned Memoria snapshot `c716e1faaeee8c856cc10921234d017f1187b45b`, files under `Assembly-CSharp/`. All findings are source observations from the C#. Nothing here was checked against a real save (none exists on the development machine); real-save verification is `NOT_RUN`. Rust: `crates/tantalus-legacy/src/saves/ff9_tree.rs`, tests `crates/tantalus-legacy/tests/ff9_tree.rs` (synthetic data only).

## How the tree becomes bytes

- `Global/JsonParser.cs:33-239` (`ParseFromFF9StateSystem`) builds two parallel trees: a data tree and a schema tree whose leaves are .NET type names. All data leaves are decimal strings (`ToString()`); the schema decides the binary type.
- `Global/Shared/Data/SharedDataBytesStorage.cs:176-232` (`ParseJsonTreeToDataList`) flattens the schema: a class visits its keys after `List<string>.Sort()` (line 188), an array visits indices in order, leaves in that order are the byte layout. `ReadDataFromStream` (297-341) and `WriteDataToStream` (343-396) define the primitives: little-endian Byte, SByte, Int16, UInt16, Int32, UInt32, Int64, UInt64, Single, Double, Boolean (1 byte), `System.String` fixed 128 bytes and `String4K` fixed 4096 bytes (UTF-8, NUL padded; reads trim NUL both ends; writes truncate at the field size). The written stream is `SAVE` + leaves (`WriteDataToStream` 345-348); `DataSize` is the leaf byte sum without the magic (`EstimateDataSize` 254-295), limit `DataReservedSize = 18432` (line 1435).
- Write casts leaf text through `AsInt`/`AsUInt`/`AsFloat`... then to the schema type (`WriteDataToStream` 353-374), so an oversized value is truncated, not rejected (for example `status` is stringified from an Int32 but stored as Byte).
- The root is `{"Data": {...}}` plus a separate `MemoriaExtraData` class that `Save` removes before flattening (`SharedDataBytesStorage.cs:676`) and writes it as a sidecar file after the base block (`SaveToFile`, line 725). Only `Data` is in the base payload.
- Rust: `Ff9SaveTree::encode_payload/decode_payload` build the same pair of trees and reuse `BaseSavePayload` for the primitives. Computed payload size: **17,966 bytes** (fits 18,432 - 4). Key order is selectable (`KeyOrder`), see "Not source-backed".

## Sections of `Data` (all in the base payload)

Ids sorted ordinally: 10000, 20000, 30000, 40000, 50000, 60000, 70000, 80000, 91000, 92000, 93000, 94000, 95000, 96000, 97000, 98000, 99000.

### 10000_State - `JsonParser.cs:415-513` (bytes: 19)
Keys (ordinal order) with type: `btlMapNo` Int16, `btlSubMapNo` SByte, `fldLocNo` Int16, `fldMapNo` Int16, `mode` Byte, `prevMode` Byte, `timeCounter` Single, `timerControl` Boolean, `timerDisplay` Boolean, `wldLocNo` Int16, `wldMapNo` Int16. Data source lines 420-466, schema 467-513. Rust `StateSection`.

### 20000_Event - `JsonParser.cs:575-633` (bytes: 4,100)
`gEventGlobal` String4K (base64 of `EventState.gEventGlobal`, 2048 bytes, `Event/EventState.cs:10`; 2,732 base64 chars), `gStepCount` Int32 (`EventState.cs:8`). Schema lines 628-632. The base format omits `gAbilityUsage`, `gScriptVector`, `gScriptDictionary` (only in the sidecar, lines 580-624). Rust `EventSection`, `EventGlobal`.

`gEventGlobal` byte ranges named in the C# (Rust `EventGlobal` accessors):
- bytes 0-1 `ScenarioCounter`, little endian (`EventState.cs:16-24`); bytes 2-3 `FieldEntrance` (`:26-34`).
- bytes 184-186 chocograph "opened" (gem) bit field, bytes 187-189 "found" (map) bit field, 24 maps (`ChocographUI.cs:250-251`, `HintMapMax = 24` at `:428`); byte 191 chocograph ability (`ChocographUI.cs:245`).
- Treasure Hunter points (`EventState.cs:53-72`): popcount of bytes 896-960 and 966-975 (1 point per bit) plus 2 per bit in bytes 182-186.
- Legacy ability usage counters at `1100 + id`, ids 1-191 (`JsonParser.cs:538-539`).
- Other named bytes (not typed in Rust): 100 world navigation mode (`ff9.cs:2263,2268`), 101 (`ff9.cs:3085`), 181 (`WMBeeMenu.cs:117`), 190 (`ff9.cs:5665`), 192 Tonberi count, 193 summon ray flag, 207 summon-all-long flag (`battle.cs:41-43`), 199 bit 16 (`SFX.cs:880`).

### 30000_MiniGame - `JsonParser.cs:690-763` (bytes: 1,106)
`MiniGameCard` array of 100 classes, `sDraw` Int16, `sLose` Int16, `sWin` Int16. Card keys: `arrow` Byte, `atk` Byte, `cpoint` Byte, `id` Byte, `mdef` Byte, `pdef` Byte, `side` Byte, `type` Int32 (schema 741-761). Empty slot: every value 255 (lines 723-733); the writer always writes `side` as 0 for real cards (line 712). Rust `MiniGameSection`, `CardSlot`.

### 40000_Common - `JsonParser.cs:765-1126` (bytes: 2,790)
Top keys: `dragon_no` Int16, `escape_no` UInt16, `frog_no` Int16, `gil` UInt32, `items` (256 x {`count` Byte, `id` Byte}; empty slot id 255 count 0, lines 976-983), `players` (9), `rareItems` (64 Byte, line 990 + `FF9StateGlobal.cs:870-885`: bit 2k obtained, 2k+1 used for ids 4i..4i+3), `slot` (4 Byte, 255 = empty member), `steal_no` Int16, `summon_flag` UInt16. Schema lines 1018-1125.
Player keys (9 entries, legacy slot order via `SelectOldSaveSlot` 1831-1851), schema lines 1022-1098: `basis` {`dex`,`max_hp` Int16,`max_mp` Int16,`mgc`,`str`,`wpr`; stats Byte}, `bonus` {`dex`,`mgc`,`str`,`wpr` UInt16}, `category` Byte, `cur` {`at` Int16, `at_coef` SByte, `capa` Byte, `hp` UInt16, `mp` Int16}, `defence` {`m_def`,`m_ev`,`p_def`,`p_ev` Byte}, `elem` {`dex`,`mgc`,`str`,`wpr` Byte}, `equip` 5 x Byte, `exp` UInt32, `info` {`menu_type`,`party`,`row`,`serial_no`,`slot_no`,`win_pose` Byte}, `level` Byte, `max` (as `cur`), `name` String128, `pa` 48 x Byte, `sa` 2 x UInt32, `status` Byte, `trance` Byte, `web_bone` Byte. Per player: 244 bytes. Rust `CommonSection`, `PlayerRecord`, `Resources`.

### 50000_Setting - `JsonParser.cs:1410-1470` (bytes: 1,444)
`cfg` {15 UInt64: `atb`, `camera`, `control`, `control_data`, `cursor`, `fld_msg`, `here_icon`, `move`, `skip_btl_camera`, `sound`, `sound_effect`, `target_win`, `vibe`, `btl_speed`, `win_type` (types from `FF9CFG.cs:34-49`), `control_data_keyboard` 10 x Int32 (schema line 1459 uses Int32, length `HonoInputManager.DefaultInputKeysCount = 10`, `HonoInputManager.cs:1179`), `control_data_joystick` 10 x String128}, `time` Single (always written -1, line 1439). `camera` is added twice (line 1436) and is one key. `btl_msg` is not saved. Rust `SettingSection`.

### 60000_Sound - `JsonParser.cs:1530-1546` (bytes: 4)
`auto_save_bgm_id` Int32 (`Sound/SoundState.cs`). Rust `Ff9SaveTree::auto_save_bgm_id`.

### 70000_World - `JsonParser.cs:1554-1620` (bytes: 23)
Literal dotted keys: `data.cameraState.rotation` Single, `data.cameraState.rotationMax` Single, `data.cameraState.rotationRev` Single, `data.cameraState.upperCounter` Int16, `data.cameraState.upperCounterForce` Boolean, `data.cameraState.upperCounterSpeed` Int32, `data.hintmap` UInt32 (`ff9.cs:11109-11138`). Rust `WorldSection`.

### 80000_Achievement - `JsonParser.cs:1642-1728` (bytes: 2,848)
`AteCheckArray` 100 x Int32, `EvtReservedArray` 17 x Int32, `blkMag_no`, `whtMag_no`, `bluMag_no`, `summon_no`, `enemy_no`, `backAtk_no`, `defence_no`, `trance_no` Int32, `abilities` 221 x Int32 (unused -1), `passiveAbilities` 63 x Int32 (-1), `synthesisCount`, `AuctionTime`, `StiltzkinBuy` Int32, `QuadmistWinList` 300 x Int32 (-1). Field types `Achievement/AchievementState.cs:116-178`. Rust `AchievementSection`.

### 91000_State, 92000_Event, 93000_MiniGame - `JsonParser.cs:20-31,47-49` (512 bytes each)
Reserved: 128 x Int32 each, all written as "0". No named fields. Rust `reserved_91000/92000/93000`.

### 94000_Common - `JsonParser.cs:50-70`
`00001_player_bonus` 9 x UInt32 (`bonus.cap` per legacy slot; line 61), `99999_ReservedBuffer` 119 x Int32. The first `AddReservedBuffer` array (line 50) is replaced by the class (line 63; `JSONClass.Add` replaces, `SimpleJSON/JSONClass.cs:75-92`). 512 bytes. Rust `Common94000`.

### 95000_Setting - `JsonParser.cs:71-80`
`00001_time` Double (`SettingsState.time`), `99999_ReservedBuffer` 126 x Int32. 512 bytes. Rust `Setting95000`.

### 96000_Sound, 97000_World - `JsonParser.cs:81-82`
Reserved: 128 x Int32 each. 512 bytes each. Rust `sound_96000`, `world_97000`.

### 98000_Achievement - `JsonParser.cs:83-226`
`00001_abnormal_status` UInt32, then Boolean flags `00002_summon_shiva`, `00003_summon_ifrit`, `00004_summon_ramuh`, `00005_summon_carbuncle_reflector`, `00006_summon_carbuncle_haste`, `00007_summon_carbuncle_protect`, `00008_summon_carbuncle_shell`, `00009_summon_fenrir_earth`, `000010_summon_fenrir_wind`, `000011_summon_atomos`, `000012_summon_phoenix`, `000013_summon_leviathan`, `000014_summon_odin`, `000015_summon_madeen`, `000016_summon_bahamut`, `000017_summon_arc` (the digit run grows from 5 to 6 at the tenth key, lines 122-153), plus `99999_ReservedBuffer` 123 x Int32. 512 bytes. Rust `Achievement98000`.

### 99000_Other - `JsonParser.cs:227`
Reserved: 384 x Int32 (1,536 bytes). Rust `other_99000`.

## MemoriaExtraData sidecar - `JsonParser.cs:229-238`

Not part of the base payload; a SimpleJSON binary tree (`SimpleJsonDocument`). Written with `oldSaveFormat = false`; keys in writer order: `95000_Setting` {`00001_time`}, `20000_Event`, `40000_Common`, `30000_MiniGame`. `MetaDataSlotID`/`MetaDataSaveID`/`MetaDataFileName` are added in memory on load only (`SharedDataBytesStorage.cs:649-657`) and are never stored.

- `20000_Event` (`:575-624`): `gStepCount`, `gEventGlobal` (base64 string), `gAbilityUsage` [{`id`,`count`}], `gScriptVector` [{`id`,`entries`[int]}], `gScriptDictionary` [{`id`,`entries`[{`id`,`value`}]}].
- `30000_MiniGame` (`:690-737`): `sWin`, `sLose`, `sDraw`, `MiniGameCard` [{`id`,`side`,`atk`,`type`,`pdef`,`mdef`,`cpoint`,`arrow`}] with only the cards that exist (no 100-slot padding).
- `40000_Common` (`:765-1017`): `players` (per player: `name`, `category`, `level`, `exp`, `cur`/`max` {`hp`,`mp`,`at`,`at_coef`,`capa`}, `trance`, `web_bone`, `elem`, `defence`, `basis`, `info` {... + `sub_replaced`}, `status` (UInt64 text), `equip`, `bonus` {... + `cap`}, then `pa`+`sa` or `pa_extended` [{`id`,`cur`}]+`sa_extended`, `maxHpLimit`, `maxMpLimit`, `maxDamageLimit`, `maxMpDamageLimit`, `basis_capa`, `basis_max_capa`), `slot`, `escape_no`, `summon_flag`, `gil`, `frog_no`, `steal_no`, `kills_per_category` [{`id`,`count`}], `kills_per_model` [{`id`,`count`}], `items` [{`id`,`count`}], `rareItemsEx` [{`id`,`obtained`,`used`}], `battle_no`.
- Leaves: `JSONData.Serialize` (`SimpleJSON/JSONData.cs:55-88`) picks the tag from the text: Int32 if it round-trips, else Float, Double, Bool, else string. Booleans are `Boolean.ToString()` ("True"), which never round-trips as `AsBool`, so they are string leaves. Rust `MemoriaExtra` (strict: all keys required, extra keys rejected).

## Reader quirks worth knowing (`JsonParser.cs:246-358, 1183, 1637`)

- The loader reads player key `wep_bone` (line 1183) but the writer stores `web_bone` (line 812), so `wep_bone` never loads from the base tree.
- `ParseWorldJsonToData` loads `hintmap` from `data.cameraState.rotationRev` (line 1637-1638), not from `data.hintmap`.
- `timeCounter` is read with `AsFloat` and truncated to Int32 seconds (line 408).
- Missing `players`/section keys are tolerated by the reader (every field is guarded by `!= null`); the stream itself always carries every key.
- `94000_Common.00001_player_bonus` seeds `bonus.cap`; zero caps are recomputed as `(level - 1) * 5` plus an Eiko bonus (lines 344-351).

## Not source-backed

Fields or behaviors whose layout or meaning I could not read from the C#. Raw values are kept as named blobs; nothing is guessed.

1. **Key sort comparer.** `nodeFieldList.Sort()` (`SharedDataBytesStorage.cs:139,188`) uses the default string comparer (culture-sensitive). The Unity/Mono collation is not in the source. `KeyOrder::Ordinal` (default, same as `BaseSaveSchemaNode`) and `KeyOrder::CultureApprox` (my approximation) produce different layouts only in `80000_Achievement` (keys mixing case: `AteCheckArray`, `QuadmistWinList`, ...) and `98000_Achievement` (`000010_...` vs `00001_...`). Which one a real save uses is unverified; do not trust either for those two sections until a real save is checked.
2. **Vanilla vs Memoria schema.** The layout comes from Memoria's writer, which keeps the vanilla format. A byte-level match with an unmodified original-game save is not verified.
3. **Reserved sections.** `91000_State`, `92000_Event`, `93000_MiniGame`, `94000_Common.99999_ReservedBuffer`, `95000_Setting.99999_ReservedBuffer`, `96000_Sound`, `97000_World`, `98000_Achievement.99999_ReservedBuffer`, `99000_Other` are written as zeros by this writer; what the original game stored there is unknown. Kept as raw Int32 arrays.
4. **Enum and bitfield meaning.** `mode`, `prevMode`, `category`, `info.*` (slot/serial/menu type/row/pose), `status` bits, `cfg` values, `equip` item ids are raw integers; the enums (`CharacterId`, `CharacterOldIndex`, `BattleStatus`, `CharacterPresetId`, `RegularItem`) were not mapped. The `SelectOldSaveSlot` slot remapping (1831-1851) is not implemented.
5. **gEventGlobal contents.** Only the byte ranges named above are typed. The remaining bytes are event-script globals whose meaning lives in the field scripts, not in this C#.
6. **Lossy casts.** Because the writer casts to the schema type, values written from wider fields (`status` Int32 to Byte, `cur.hp` UInt32 to UInt16, `capa`, `bonus.cap`) cannot be recovered from the base tree; only the sidecar holds them.
7. **Sidecar numeric tags beyond Int32.** For integers above Int32 the Rust encoder emits a Double leaf below 1e15 and a string leaf above; this follows from `JSONData.Serialize` plus an assumption about .NET `Double.ToString()` and is unverified. The sidecar `time` leaf keeps its original tag when decoded; a freshly built one uses Double, but the C# would pick Float when the text round-trips as one.
8. **Whitespace in base64.** .NET `Convert.FromBase64String` ignores whitespace; the Rust decoder is strict. Strings longer than the field are truncated by the C# (possibly mid-codepoint) but rejected by Rust.
9. **Preview and file container** are covered by `audit/legacy-saves.md`, not repeated here.
