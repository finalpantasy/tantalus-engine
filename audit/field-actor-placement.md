# Field actor placement sweep (W6)

Check: after each field's scripts run, every actor with a model and the show-model flag (`flags & 1`) should have its (x, z) on a triangle of that field's walkmesh.

Run: `tantalusctl field actor-placement --assets <StreamingAssets> --game-data <FF9_Data> --memoria <root> --atan <ratan_tbl.bin> [--ticks 120] [--field <map>] --out audit/field-actor-placement.json`

Method: each field starts on the new-game world, runs 120 ticks with no input (open dialog windows closed every tick; stops early on a field or world-map change), then the final state is checked. A point counts as on the mesh when some floor has a triangle containing it (edges and corners included).

## Result (120 ticks)

| Count | Value |
|---|---|
| Event DB entries | 1,842 |
| Fields swept (event binary and walkmesh found) | 816 |
| Entries skipped (no walkmesh or no binary: world, battle, FMV, empty) | 1,026 |
| Model actors checked | 2,725 |
| On the walkmesh | 2,382 |
| Off the walkmesh | 343 in 213 fields |
| Script errors | 0 |

Zero off the walkmesh is not met: 343 of 2,725 (12.6 percent).

Off-mesh actors by class:

- `scripted_prop`, 266: the field's scripts contain `SetPathing`. In Memoria `FieldMapActorController.SetPosition` an actor with `charFlags & 1` set is snapped onto the walkmesh (off it, to `SetDefaultCharPos`); only an actor with that bit cleared may stand anywhere. `SetPathing(0)` clears it (`WalkMesh.BGI_charSetActive`, `EventEngine.DoEventCode.cs` case `BGI`). The engine does not track the bit, so these are candidates, not proven props.
- `unexplained`, 77 in 56 fields (largest: map 752 `EVT_BURMECIA_TOWN_1` 6, maps 56, 2454, 3005 3 each): no `SetPathing` in the field, so the reference would have snapped them. These are engine gaps (missing snap, or a script path that is not run right).
- `unplaced` (still at 0, 0, 0): 0.

Actors with a height of 32768 or more in magnitude ("take the walkmesh height") appear in 17 of the 343.

## Not covered

- Final state only; an actor that is off the mesh mid-script and back on by tick 120 is not seen, and one that would leave later is not seen.
- No input, so fields that wait on a button or region run only their idle path.
- The snap itself (`SetPosition` on `DPOS`/`MoveInstant`) is not implemented in the field world; adding it and re-running this sweep is the next step toward zero.

The full list (field, actor uid, position, model, class) is in `audit/field-actor-placement.json`.
