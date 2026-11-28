# Memoria bug fixes for the vanilla game

Owner decision: Tantalus includes every Memoria bug fix for the vanilla game. Optional features and config toggles are out of scope. Widescreen, high FPS and PS1 audio are tracked separately and ship as defaults.

## Sources

- Pinned reference checkout `E:/FFIX Recomp/.reference/Memoria` at `c716e1f` (a git checkout; 1074 commits, 343 with fix/bug/softlock/crash in the subject). Commit messages are mostly vague ("Bug fixes", "Hotfix"), so they only date a fix.
- Wiki changelogs, one page per release: [v2024.04.28](https://github.com/Albeoris/Memoria/wiki/Changelog-v2024.04.28), [v2024.05.19](https://github.com/Albeoris/Memoria/wiki/Changelog-v2024.05.19), [v2024.06.09](https://github.com/Albeoris/Memoria/wiki/Changelog-v2024.06.09), [v2024.08.05](https://github.com/Albeoris/Memoria/wiki/Changelog-v2024.08.05) (with its patches 08-10, 08-12, 08-18, 08-24), [v2024.11.17](https://github.com/Albeoris/Memoria/wiki/Changelog-v2024.11.17), [v2024.12.04](https://github.com/Albeoris/Memoria/wiki/Changelog-v2024.12.04), [v2025.05.12](https://github.com/Albeoris/Memoria/wiki/Changelog-v2025.05.12), [v2025.07.04](https://github.com/Albeoris/Memoria/wiki/Changelog-v2025.07.04), [v2026.11.09](https://github.com/Albeoris/Memoria/wiki/Changelog-v2026.11.09). The changelog is the only place that lists fixes by name. Cited below as `CL <version>`.
- Code markers in the decompiled C# (`// Fix`, `vanilla`, `bug`), cited as file:line under `Assembly-CSharp/`.
- Not read: the Qhimm thread and the FF9 modding wiki. No search was done there, so fixes known only to those pages may be missing. Closed GitHub issues were not walked either; the changelog cites issue numbers (#nnn) where they exist.

## How to read the table

- Gating: `always` means no config switch. `cfg` names the switch and its default.
- Status: `done` has a cited code path or ledger row. `todo` means not found in Tantalus. `todo (verify)` means Tantalus ports the Memoria battle or event code that carries the fix, so it is probably in, but no test or code line proves it. `n/a` means Unity, launcher, mod-manager or modding-only.
- The "wide/fps" rows are fixes whose symptom exists only with widescreen or interpolated frames. They belong to the separate widescreen and high-FPS work; they are listed so nothing drops.
- Rows with source "CL only" had no code location found. The implementer should locate the code from the changelog line.

## Gameplay fixes

| ID | Fix | Source | Area | Gating | Tantalus |
|---|---|---|---|---|---|
| G01 | Trance decay overflow: the original `tranceDelta` was a Byte, so a low-level Quina could stay in trance up to 64 turns. Now `(300 - Level) / Will * 10` without wrap | `FF9/btl_cmd.cs:1301-1312` | battle | always (`TranceDecreaseFormula` empty) | done: `tantalus-legacy/src/battle/machine.rs:2592` |
| G02 | Quina walks speed 25 (not 30) in the marsh scene, field 658 | `EventEngine.DoEventCode.cs` MSPEED | field script | always | done: ledger `memoria-quina-658-walk-speed`; `tantalus-game/src/field/host.rs:1040` |
| G03 | Thunder Slash uses Magic Weapon hit rate instead of Gravity | commit `3e99138` (2017); `Data/Battle/Actions.csv` row 149 | ability | always (data) | done: ledger `memoria-actions-thunder-slash-script` |
| G04 | Life / Full-Life camera data | `Actions.csv` rows 5, 6 | ability | always (data) | done: ledger `memoria-actions-life-default-camera` |
| G05 | `GetCardAmount` fix, generic card ids 512-611 unclamped | commit `5d73810` | item/cards | always | done: ledger `memoria-card-amount-unclamped` |
| G06 | Field heals (tent, inn, springs, beach) reach Cinna, Marcus, Blank and Beatrix; non-main characters are healed and cured in every situation, including World Map tents | `EventEngine.DoEventCode.cs:2748-2840`; CL 2024.04.28 "scripted heal", CL 2025.05.12 #1028 | field script | always | done: `tantalus-game/src/field/host.rs:2000` (Beatrix and old-index spread, test at `:2231`); verify the World Map tent branch (gMode 3) |
| G07 | "Clear all statuses" (mask 0x7F) also clears extra `OutOfBattle` statuses; items and revive abilities clear them too | `DoEventCode.cs:2830`; CL 2026.11.09 | status | always | done: field host maps 0x7F to every status (`field/host.rs` ClearPartyStatus, `FF9BattleDB.AllStatuses`); test `tantalus-game/tests/field_host.rs` `party_and_flow_ops_are_recorded` |
| G08 | Esuna cures Blind again (regression in v2024.08.05, restored) | CL 2024.11.17 | ability | always (data) | done: Esuna (action 10) adds set 5 which holds Blind in the loaded PC tables, identical to Memoria; `known_rows.rs` checks (ignored test `real_install_matches_known_rows`) |
| G09 | Back-row resistance and elemental equipment resistance were ignored | CL 2024.08.05 patch 08-18 | battle formula | always | done (verify): Back-row halving and element guard/half/weak/absorb are ported in `calc.rs`; tests `s1_defend_and_back_row_halve_twice`, `s9_element_modifiers`, `s1_guarded_weapon_element`. Equipment-to-element wiring belongs to the battle6 work |
| G10 | Trouble now spreads damage on lethal blows | CL 2024.08.05 patch 08-18; `BattleCalculator.cs:811`, `SBattleCalculator.cs:194` | status | always | done: `Battle::spread_trouble` in `machine.rs` (`TroubleStatusScript.OnFigurePoint`); test `battle_more.rs` `trouble_spreads_half_the_damage_also_on_a_lethal_blow` |
| G11 | Weapon statuses are applied before damage, like spell statuses (the vanilla order differed) | `SBattleCalculator.cs:188` | battle formula | always | done (verify): `calc_result` applies the weapon status before damage (`SBattleCalculator.cs:188`). Open: SA 25 Add Status (the feature that sets `AddStat`) is not modeled, so weapon statuses never proc yet |
| G12 | Accuracy+ support ability worked wrongly with Distract | CL 2024.04.28; `BattleTarget.cs:87` `PenaltyDistractHitRate` | battle formula | always | done: `Calculator::hit_rate_setup_features` (SA 9 Accuracy+ sets rate 100, evade 0, `DisableSA 10`; SA 10 Distract halves); test `battle_formulas.rs` `g12_accuracy_plus_overrides_distract` |
| G13 | Auto-Potion fired when the character absorbed an element or was healed by an enemy attack | CL 2025.07.04 (v2025 list); `FF9/btl_abil.cs:74-93` | AI/ability | always | done: Auto-Potion condition matches `AbilityFeatures.txt` SA 56 (no absorb or heal); test `battle_more.rs` `auto_potion_ignores_absorbed_and_healed_hits` |
| G14 | Double-cast effect 80 targets the same single target when both abilities are single-target | CL 2025.07.04 | ability | always | done (verify): tests `double_cast_queues_two_spells_and_charges_once`, `double_cast_all_enemy_first_spell_takes_the_whole_side` |
| G15 | `AutoAttack` ignored a replaced Attack command | CL 2025.07.04 | ability | always | todo (verify) |
| G16 | Default targeting of spells usable on single and multiple targets | CL 2025.07.04 | ability | always | todo (verify) |
| G17 | Death-dealing attack commands (Avenger with Add Status) showed "Miss" instead of "Death" | CL 2025.07.04 | battle text | always | todo (verify) |
| G18 | SA features changing caster statuses did not apply when the caster was also the target | CL 2025.07.04 | status | always | todo (verify) |
| G19 | Missing captions on the battle result screen | CL 2025.07.04 | text | always | done (English only): `tantalus-client/src/battle_view.rs` `results_lines`; ledger `memoria-battle-result-captions` |
| G20 | Steiner Magic Sword attacks were far too long; wait removed when only one attack is queued | CL 2024.08.05 | battle | always | todo (verify): `machine.rs:204-215` `CommandTimings` exists; the Memoria timing change is not confirmed |
| G21 | Occasional double input in battle | CL 2024.08.05 | battle UI | always | n/a (Unity input); verify no repeat in Tantalus input |
| G22 | Gradual Petrify and Doom countdown disappeared (dynamic ATB, model change after Trance) | CL 2024.06.09, CL 2026.11.09 | status | always | todo (verify): `battle/status.rs` has the statuses; the display countdown is UI |
| G23 | Fixed `AbilityElementForBonus` bug | CL 2024.06.09 | ability | always | n/a: modding feature only (`CharacterAbilityGems.cs:676`) |
| G24 | Lich's Death Cutter and Nova Dragon soft-lock with `SFXRework = 0` | CL 2025.07.04 #1029; `SFX.cs:1125` | battle | cfg (`SFXRework`, default 1) | n/a: Tantalus does not use the `SFXRework = 0` path |
| G25 | Atomos left some enemies hidden or tiny (`SFXRework = 0`) | `SFX.cs:1052-1058` | battle | cfg | n/a (same reason as G24) |
| G26 | Soft-lock when no player remains in combat | CL 2025.05.12; `Battle/Calculator/BattleUnit.cs:601` | battle | always | done by identity: `BattleUnit.Remove` has no caller in the vanilla game (only mods), and Tantalus never removes a player from combat; a dying player calls `check_battle_phase` at `die_seq` 6 (`machine.rs` `die_sequence`); test `battle_atb.rs` `defeat_when_all_players_die`; ledger `memoria-g26-player-remove-unreachable` |
| G27 | Battle ending: Maliris's final attack killing the party still ended in victory; Ozma cast spells on a dead party | CL 2024.12.04 | battle | always | done (verify): Tantalus ports `CheckBattlePhase` and keeps the `btl_seq != Defeat` guard; the dying move killing the party ends in Defeat and a dying move that spares it ends in Victory; tests `battle_atb.rs` `g27_dying_move_that_kills_the_party_is_a_defeat`, `g27_dying_move_that_spares_the_party_is_a_victory`. Auto-Life (`HasDeathChangerEffect` in `playerStillAlive`) is not modeled in the machine; ledger `memoria-g27-battle-end-after-dying-move` |
| G28 | Scan could cancel Zombie's suicide; Scan immunity display fixed | CL 2024.12.04, CL 2024.08.24 | status | always | todo (verify) |
| G29 | Freya's Jump could be used to cheat self-targeting onto another unit | CL 2024.12.04 | ability | always | todo (verify) |
| G30 | Fleeing command with no caster raised an unhandled exception | CL 2024.12.04 | battle | always | todo (verify) |
| G31 | Garnet's Eidolon replica mixed two Eidolons when a second was queued | CL 2024.12.04 | ability | always | todo (verify) |
| G32 | Hidden in-battle characters showed in the battle menu | CL 2024.12.04 | battle UI | always | todo (verify) |
| G33 | Heat effect now works on enemies with Easy Kill Proof (#835) | CL 2024.12.04 | status | always | done: `machine.rs` Heat branch (`btl_cmd.cs:530`); test `battle_more.rs` `heat_kills_an_easy_kill_unit_when_its_command_runs` |
| G34 | Frozen boss with Easy Kill Proof cannot be killed by Freeze (no vanilla change) | `BattleTarget.cs:367` | status | always | n/a: no vanilla effect |
| G35 | Cover: monster position corrected | CL 2025.07.04; commit `be8d87a` | battle visual | always | todo (verify): `machine.rs:2025` emits `Covered` |
| G36 | Trance duplicated or glitched the command title (`SFXRework = 0`); summon-title mixups | CL 2024.12.04 #838, CL 2024.11.17 | text | cfg | n/a (SFXRework 0 only) |
| G37 | Fix post-menu stats in battles (stats wrong after opening the menu mid-battle) | commit `45f4a5a`; CL 2025.07.04 | battle | always | todo (verify) |
| G38 | Soft-lock after a soft reset in battle | CL 2024.12.04 #830 | battle | always | n/a: Tantalus has no soft reset |
| G39 | Soft-lock when saving on the world map (introduced by the PC port) | CL 2025.07.04; commit `a741ff5` | world | always | done (wsave1, 2026-11-27): `WorldWalk::verify_valid_cast_position` (`tantalus-legacy/src/world/walk_mesh.rs`) is `w_movementChrVerifyValidCastPosition` with Memoria's square spiral (step 20 fixed units, radius under 200), run when the world script places a chocobo (`WorldPlay::set_script`; `EventEngine.SetObjectPosition` world mode, object index 3 to 7). Test `world_play.rs` `a_chocobo_without_ground_moves_to_the_ground_nearby_and_the_search_ends` (a block seam at x 512, z -1494 moves one step east; open land stays; an empty world ends with no move). The save itself: the moogle's save loads back at the same spot and ride (`world_moogle.rs`) |
| G40 | Friendly Mu could still be met after giving it an ore (PC-port bug) | CL 2025.07.04; list at `btl_para.cs:402` | world | always | open, blocked: no world-map Mu logic in Tantalus and no code location in Memoria (CL only) |
| G41 | Soft-lock in the French ending cutscene | CL 2025.07.04 | field script | always | n/a (Unity text timing); verify the French ending runs |
| G42 | Battle or naming screen soft-locks as the interface appears, on some PCs | CL 2025.07.04; commit `8c99665` | battle/menu | always | n/a (Unity UI) |
| G43 | Cure/Heal of "permanent" statuses in menus: wrong statuses could stone characters | CL 2025.05.12 (mod case) | status | always | n/a: mod-only trigger |
| G44 | Esuna, Dispel and other data-table fixes inherited from `Data/Battle/*.csv` | `Memoria.Patcher/StreamingAssets/Data/Battle` | ability | always (data) | done: all 192 actions and 39 status sets of the PC tables diffed against Memoria `Actions.csv`/`StatusSets.csv`; only rows 5, 6, 149 differ and all three are in the ledger |
| G45 | `GetUnitProperty` returns 0 for characters not in battle | CL 2025.05.12 | AI | always | n/a: modding formula API |
| G46 | AI variables `GetAttackCommandId` / `GetAttackId` worked only in counters | CL 2024.11.17 | AI | always | n/a: Memoria AI extension |
| G47 | `WhenGameOver` / `WhenDefeated` ignored death status, so no line played | CL 2025.05.12 | AI | always | n/a: Memoria Battle Voice Acting (`BattleVoice.cs:350`); Tantalus has no battle voice lines; ledger `memoria-g47-battle-voice-n-a` |
| G48 | Jump Rope: points reach the next vanilla reward (no skipped reward) | `EBin.cs:1942`; CL 2025.05.12 #1006 | field script | cfg (`RopeJumpingIncrement`, default 1) | done by identity: Memoria's rope hack at increment 1 never changes a value (owner chose PS1 values); ledger `memoria-rope-jump-no-skip` |
| G49 | Blank swordfight controls fixed; Steam adds +30% score | `EMinigame.cs:19`; CL 2025.05.12 | field script | cfg (`SwordplayAssistance`, default 1) | done: Tantalus has no +30% bonus (owner chose PS1 values); ledger `memoria-swordplay-no-steam-bonus` |
| G50 | Cinna/Marcus/Blank heal; "The Ultimate Racket" achievement tied to the wrong item | CL 2024.04.28 | item/achievement | always | n/a (Steam achievement); heal part is G06 |
| G51 | Key-item removal did not work | commit `c03cda6` (2023) | item | always | done: `Inventory::remove_generic` uses the important id; test `shop.rs` `key_items_use_the_generic_ids` |
| G52 | Revive worked on undead characters | commit `2012946` (#50) | battle formula | always | done (verify): test `battle_formulas.rs` `s13_revive_refusals_and_zombie` |
| G53 | Enemy attacking dead characters | commit `bac6b4a` | AI | always | done by identity: Memoria's own parser bug (a skipped 12-bit field in `BTL_SCENE.cs`); the Tantalus scene parser reads `for_dead` and `default_on_dead` from the real flag bits (`tantalus-assets` `battle_scene.rs`); ledger `memoria-g53-scene-command-flags` |
| G54 | Fast-battle Poison and Regen tick fix; ATB speed-up fix; Return of Cannon attack, ATB gauge color (gray/yellow) | commits `404d1f3`, `4196475`, `eeae897` | battle | always | done: Cannon uses `SetCommandAttack` as in `0091_CannonScript.cs` (`scripts.rs` `cannon`); the fast-battle Poison/Regen tick, ATB fps scaling and gauge sprite are Memoria-only (Tantalus has its own fixed-rate ATB and HUD); ledger `memoria-g54-cannon-and-memoria-only-atb` |
| G55 | Fixed Atomos, Demi, Aqua Breath, Demi Shock, Worm Hole data | commit `48bbec6` | ability | always (data) | done: same table diff as G44, no other row differs from Memoria (Atomos, Demi, Aqua Breath, Demi Shock, Worm Hole identical) |
| G56 | Eiko summons Madeen in the scripted fight (#890) | CL 2024.12.04 | field/battle script | always | open: Memoria code location not found (no `Madeen` special case in the C#; likely a battle-data change) |
| G57 | Dialog: fast inputs applied the wrong cancel choice; French dialog lacked `[IMME]` (#1237) | `Dialog/Dialog.cs:777,799,824` | text | always | n/a (Unity dialog widget); verify choice cancel in the Tantalus dialog |
| G58 | Mist Continent title stayed on screen if the player left the world map mid-fade (vanilla bug) | CL 2024.11.17; `FF9UIDataTool.cs:816` | world | always | open, blocked: no world-map title UI in Tantalus |
| G59 | Navi mode >= 2 reset on world map load (#697) | `ff9/ff9.cs:6785` | world | always | open, blocked: no world-map navi state in Tantalus |
| G60 | World map: minimap state left the map black; bugged minimap on the route to Oeilvert; 1-2 frame teleport before a battle | CL 2024.08.05; `WorldHUD.cs:1014` | world | always | open, blocked: no world-map minimap or battle-entry frame in Tantalus |
| G61 | World map auto-flight too fast and hit a wall when the path crosses a border; ship flapping animation; ship tilt direction | CL 2024.08.24, CL 2024.06.09, CL 2024.11.17 | world | always | open, blocked: world-map flight and ship are not ported (`FootWalker` rejects control types 1-3) |
| G62 | Fossil Roo exit path unaligned (#486); field 1207 Garnet teleport; 768 teleport during fade; 1554 flash after FMV | CL 2024.06.09, CL 2024.08.05 | field script | always | partial: Fossil Roo worker position (1421) and Dagger height (1207) in `field/script_fixes.rs`, test `field_host.rs` `memoria_field_script_patches`; triangle 109/110 toggles, 768 post-battle placement and 1554 flash still open |
| G63 | Madain Sari soft-lock in a specific scenario | CL 2024.08.05 patch 08-10 | field script | always | done: the soft-lock came from a Memoria hack (69ee7082, removed in patch 08-10); Tantalus has no such hack; ledger `memoria-madain-sari-softlock-hack-absent` |
| G64 | Kuja appears early, spoiling Zidane's surprise (#860); Zidane breaks through Garnet (#892); Steiner transparent (#896) | CL 2024.11.17 | field script | always | partial: Kuja #860 (2209) and Steiner #892 (257) in `field/script_fixes.rs`, test `field_host.rs` `memoria_field_script_patches`; Steiner transparent #896 not located |
| G65 | Hunt festival: unable to talk with turbo dialog | CL 2024.06.09 | field script | cfg (TurboDialog) | n/a (turbo dialog is an option) |
| G66 | Tetra Master: analog stick disrupted card placement (#963); first-turn card placement minor issue | CL 2024.11.17; `QuadMistGame.cs:458` | cards | always / cfg | todo (verify) |
| G67 | Items `OutOfBattle` on Death handled by revive items and abilities | CL 2026.11.09 | item | always | done: vanilla `OutOfBattle` excludes Death so the Memoria branch is inert; `field_item.rs` revive script 72 tested by `ether_elixir_tent_and_phoenix` |
| G68 | Dialog text appearance speed set to PSX timing (about 14 s vs 23 s Steam) | CL 2025.05.12 | text | always | done (owner call): keep vanilla dialog speed (`TurboDialog` off) |
| G69 | Moogle animation restored when reading mognet letters | CL 2025.05.12 | field | always | todo |
| G70 | Text opcode fixes: `[u][/u]` alignment, `[XTAB=0]`, negative `[FEED]` offsets blocked | CL 2024.11.17; `FFIXTextTag.cs:203`, `TextOpCodeModifier.cs:101` | text | always | todo (verify) |
| G71 | Gizamaluke Bell Room: the female moogle no longer resets the arrival counter after the Kupo Nut, so the scene ends (softlock) | `EBin.cs:307` (`expr`, map 705, sid 3, ip 541) | field script | always | done: `EXPRESSION_SKIPS` in `field/script_fixes.rs`, test `bell_room_moogle_keeps_the_arrival_count`; ledger `memoria-field-script-705-bell-room-moogle` |

## Visual and field fixes (original-game art or script glitches)

These are PSX-to-PC port glitches fixed in code or data. Several are only visible with widescreen or interpolation (marked wide/fps); the implementer should check each against the Tantalus renderer.

| ID | Fix | Source | Area | Gating | Tantalus |
|---|---|---|---|---|---|
| V01 | Blank jumps into hole: position adjusted (field 53) | `DoEventCode.cs:641` | field script | always | done: `field/script_fixes.rs` (field 53 walk X 330; moves chain field 53 by 1 tick), test `field_host.rs` `memoria_field_script_patches` |
| V02 | Chocobo dream title scrolling on screen (field 2953) | `DoEventCode.cs:1670` | field script | always | todo |
| V03 | Anim and chest visible (field 352, counter 2540) | `DoEventCode.cs:1690` | field script | always | todo |
| V04 | Ending: camera moves instantly (fields 3000-3008, #677) | `DoEventCode.cs:1837` | field | always | todo |
| V05 | Flapping camera (fields 153, 1214, 1806, #493) | `DoEventCode.cs:1930` | field | always (fps) | todo (wide/fps) |
| V06 | Quina runs to eat Mog (field 1550) widescreen fix | `DoEventCode.cs:833` | field | always | todo (wide/fps) |
| V07 | Offscreen NPCs hidden (field 103) | `DoEventCode.cs:2029` | field | always | todo (wide/fps) |
| V08 | Treno gates open fully (widescreen) | `EBin.cs:711` | field | always | todo (wide/fps) |
| V09 | Kuja descending camera too high (field 2716) | `FieldMap.cs:712` | field | always | todo |
| V10 | Camera moves instantly after an MBG (#667) | `FieldMap.cs:724` | field | always | todo |
| V11 | Hunt timer position (#366) | `TimerUI.cs:134` | field | always | todo (wide/fps) |
| V12 | ~45 background layer order problems, 12 layers smooth, 8 cropped layers or depths, 2 camera problems, 2 border lines, 1px lines | CL 2024.04.28, 2024.05.19, 2024.08.05, 2024.08.18 | field art | always | todo: needs the field list from Memoria's per-field overrides |
| V13 | Missing/invisible objects: Clayra Trunk sand, waterfall, Desert Palace dock light, Branbal web, Alexandria stage torch, Oeilvert flame, Leviathan statue light, Crystal World animation, Alexandria pub floating glass, Daguerreo NPC | CL 2024.04.28, 2024.08.05 | field art | always | todo |
| V14 | Candle, campfire and windmill overlaps (#883, #874, #862, #891, #893, #875) | CL 2024.11.17 | field art | always | todo |
| V15 | Field SPS depth; transparency between 3D/SPS and layers (#352) | commits `ec44c78`, `b6816d3` | field | always | todo |
| V16 | Battle background fixes: Clayra waterfall, Nova Dragon sky tremble (#672), seams between PNG textures, texture animation smoothness (#779) | CL 2024.08.05; `SmoothFrameUpdater_Battle.cs:130,219`; `battlebg.cs:87` | battle visual | always | todo |
| V17 | Antlion death effect sinks in the sand, as on PSX | `SFX.cs:616` | battle visual | always | todo |
| V18 | Detect eye texture animation called 4 times | `SFX.cs:1301` | battle visual | always | todo |
| V19 | Slow clock sprite, Slow/Silent Voice distortion, Stop black sprite hidden in the Steam port | `SpecialEffect.cs:22,29,338`; `SFXScreenShot.cs:47` | battle visual | always | n/a: ledger rows `sfx-battle-slow-clock-stop-sprite`, `sfx-battle-screenshot-twist` record the plugin behavior |
| V20 | Camera bugs in `FF9SpecialEffectPlugin.dll` (3 listed) | `Memoria/Battle/SFX/SFXDataCamera.cs:11-17` | battle visual | always | n/a: the DLL's behavior is ported 1:1 |
| V21 | Screen flash on Ifrit summon | `SFXRender.cs:400` | battle visual | always | todo (verify) |
| V22 | Moving battle objects position, animation, texture (#698); weapon rotation fix for weapon model 515 (#739) | CL 2024.08.05; `CharacterBattleParameter.cs:53` | battle visual | always | todo |
| V23 | Mognet machine animation; alternate animation glitches; jump-rope anim; mounting-chocobo anim | CL 2024.08.05 | field art | always | todo |
| V24 | Unlisted but marked: Wind Shrine effect has no visual on Steam or PSX | `ff9/ff9.cs:3377` | world | always | n/a: no fix, same as PSX |

## Not applicable (Unity, launcher, mod manager, audio backend, widescreen options)

Crash and leak fixes in the launcher, Mod Manager, Model Viewer, SaXAudio and SoLoud backends, AKB file handling, Wine and Linux rendering, the x86 black screen, the patcher, achievement plumbing, and mod-only API fixes (`[code=Patch]`, `InitialATB`, `RunAnimation` over 32267, custom `InitialItems.csv`). All are CL entries in 2024.04.28 to 2026.11.09 and commits such as `f88681d`, `b39389d`, `3f8f929`, `dcd6e31`, `00c2ec7`. Tantalus has no Unity code, so none carry over. Audio items that do carry over (looping pop, music starting late, ambient sounds not stopping on load, leveling sound stacking, reflect volume, long fade at high FPS) belong to the audio work, not this list.

## Differences between vanilla Steam and Memoria

- Memoria's defaults that change behavior without being fixes: `KeepRestTimeInBattle = 1` (Tantalus sets `keep_rest_time: false`, `machine.rs:238`), `SelectBestTarget = 1`, `SwordplayAssistance = 1`, `TurboDialog = 1`, `PSXMovementMethod = 1`. None are bug fixes. Owner call: `SelectBestTarget`, `SwordplayAssistance`, `TurboDialog` and `PSXMovementMethod` stay vanilla; `KeepRestTimeInBattle` stays off by default and is an opt-in (`--keep-rest-time` flag or `TANTALUS_KEEP_REST_TIME=1`, `battle::machine::set_keep_rest_time_default`; test `keep_rest_option.rs`).
- Fixes are always-on in Memoria unless a row says `cfg`. Rows marked `cfg` with default 1 (G48, G49) are opt-in hacks, not the fix itself.
- The owner gets, by default, every `always` row plus the `cfg` rows whose default fixes the behavior. Everything marked `todo (verify)` rides on the claim that the battle port copies Memoria's logic; confirm each with a test before closing it.

## Counts

Rows: 71 gameplay, 24 visual. Gameplay: done 6 (G01-G06), n/a 15, todo 15, todo (verify) 34. Visual: n/a 3, todo (verify) 1, todo 20. See the report for the priority list.
