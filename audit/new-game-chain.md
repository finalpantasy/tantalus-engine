# New-game chain (milestone G3)

Goal: from a new game (`fldMapNo` 70, zeroed state), follow each field change until the first interactive field. Source: `Global/Event/Engine/EventEngine.Initialize.cs:51-64` (`NewGame` sets `fldMapNo = 70`, zeroes `gEventGlobal`), `Global/ff9/FF9DBAll.Events.cs` (map number to event archive name).

## Tool

`tantalus-trace chain --bundle <install>/StreamingAssets/p0data7.bin --event-db .reference/Memoria/Assembly-CSharp/Global/ff9/FF9DBAll.Events.cs --atan-table <ratan_tbl.bin> --max-fields 15 --ticks 4000`

It runs each field's archive under the orchestrated `FieldDirector` (each entry's tag-0 function, requested tags spawned on demand, as in `tantalusctl --orchestrate`), stops a field at its first `MAPJUMP` (`world.field_transitions`), and carries `FieldWorld` into the next field. The EventDB table is read from the pinned Memoria source at run time and is not copied into the repository. `tantalus-trace disasm --bundle <p0data7.bin> --event <NAME>.eb [--entry N]` prints decoded instructions.

## Result on 2026-11-16 (head after this commit)

| Map | Archive | Ticks | Ops | Outcome |
|---|---|---|---|---|
| 70 | `EVT_ALEX1_TS_OPENING` | 35 | 122 | Plays movie `0x103` (disc 1, movie 3), then `ChangeField(50)` |
| 50 | `EVT_ALEX1_TS_CARGO_0` | 4000 | 12,246 | Stalls: entry 16 polls Map byte 44, which another entry writes |

Before the movie service, field 70 polled `SyncCinematic` (system variable 15) forever: 6,089 ops over 2,000 ticks and no field change. The new `FieldFmv` (`crates/tantalus-game/src/field/fmv.rs`, ported from `Global/fldfmv.cs`) answers system variables 14 and 15 and handles `FMV` dispatch operations 0 to 5. Movie lengths are not imported yet, so a played movie finishes on its first frame, as if skipped.

## Blocker for field 50: shared variables (milestone G1)

Field 50's entries talk through Map-scope variables, and later fields branch on Global variables (Global Int16 2 is assigned in field 70). Memoria stores Global variables in one 2048-byte `gEventGlobal` and Map variables in one 80-byte `mapvar` shared by every entry and zeroed per field (`EventContext.cs:9`, `EventEngine.cs:556`), with typed, overlapping reads and writes (`EBin.GetVariableValueInternal` / `SetVariableValueInternal`). The Rust VM keeps a private `(scope, index) -> i32` map per VM, so a write in one entry is invisible to the others and to the next field. Field 50 also contains the first battle (`StartBattle` 336, entry 13) and the exit `ChangeField(52)` (entry 15).
