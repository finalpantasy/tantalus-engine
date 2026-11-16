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
| Model actors checked | 2,723 |
| On the walkmesh | 2,441 |
| Off the walkmesh | 282 in 182 fields |
| Script errors | 0 |

Before the snap was ported: 343 of 2,725 off the mesh in 213 fields (266 in fields with `SetPathing`, 77 in 56 fields without it, called `unexplained`). After: 282 off the mesh, all with pathing off; 0 `unexplained`.

Off-mesh actors by class:

- `pathing_off`, 282: the actor's `charFlags & 1` bit was clear when it was placed (or it was placed while clear and re-enabled later), so the reference leaves it where the script put it. 215 are in fields whose scripts contain `SetPathing` (down from 266: 51 of those were snapped on `POS`/`DPOS`), 67 in fields without it (placed by `POS3`/`DPOS3`, which clear the bit, or by a jump).
- `unexplained`, 0: pathing on and off the mesh, which would be an engine gap.
- `unplaced` (still at 0, 0, 0): 0.

No off-mesh actor has a height of 32768 or more in magnitude; those are all taken from the walkmesh now.

## What the field world does (ported from Memoria)

- `POS` (0x1D) and `DPOS` (0xBF) set pathing on (`WalkMesh.BGI_charSetActive(1)`), then `SetActorPosition(x, 32768, z)` calls `FieldMapActorController.SetPosition(pos, true, true)`. With the bit set, `GetTriIdxAtPos` takes, among all triangles of all floors whose XZ footprint holds the point, the one whose center height is nearest the requested height (32768 takes the highest); the actor stands on it by barycentric height. No triangle holds the point: `SetDefaultCharPos`, the center of triangle 0. `FieldNavigation::snap_to_walkmesh` and `FieldWorld::snap_actor_to_walkmesh`.
- `POS3` (0xA1) and `DPOS3` (0xAD) clear the bit and keep the script's position. `SetPathing` (0xA8) sets or clears it. `JUMP3` setup clears it. `ActorState::path_active` (runtime only).
- A position set while the bit was clear is never checked, and a later `SetPathing(1)` does not move it (`ResetPos` returns to the same `lastPos`): `ActorState::placed_unchecked`.
- `SetActorModel` snaps an actor that already has a position (`AddFieldChar` calls `SetPosition`).
- Scripted walks of pathing actors take the triangle height each step (`UpdateActiveTri`) and a step off the mesh is clamped to the nearest mesh point (`ServiceChar`; `FieldNavigation::nearest_on_walkmesh`); a clamped walk that stops closing on its target ends.

Deviation: the controlled actor is not snapped on placement or clamped on scripted walks. The chain tool picks its start floor from a height of 32768 ("unset"), and a snapped player height changes the route in map 153. Input walking is still clamped by the host.

## Not covered

- Final state only; an actor that is off the mesh mid-script and back on by tick 120 is not seen, and one that would leave later is not seen.
- No input, so fields that wait on a button or region run only their idle path.
- Memoria's per-frame forces (`ServiceForces`, radius pushes, `GetActiveTriIdxAtPos` adjacency preference) are not ported; only the placement snap and the walk clamp are.

The full list (field, actor uid, position, model, class) is in `audit/field-actor-placement.json`.
