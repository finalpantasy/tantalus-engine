# FF9SpecialEffectPlugin.dll: how the Steam game runs PS1 effect scripts

`x64/FF9_Data/Plugins/FF9SpecialEffectPlugin.dll` (435,200 bytes, x64) runs every battle
spell, summon and enemy attack effect. It is a PS1 emulator shell: it loads the PS1 effect
package, decodes the MIPS R3000 code in it, interprets that code with a GTE (the PS1 geometry
coprocessor), and hands Unity two things: VRAM rectangles to upload, and a PS1 GPU ordering
table of primitives to draw. Unity never sees a mesh from the plugin; `SFXRender` builds meshes
from the GPU packets.

This note describes the shipped binary as the reference for a Rust port. It is not a port.

## Sources and how they were checked

- Memoria's C# rewrite of the DLL, pinned at `.reference/Memoria/FF9SpecialEffectPlugin/`
  (commit `1ed8e64`, "FF9SpecialEffectPlugin (#1541)"). It is a function-by-function rewrite
  of the original: `CodeAddresses.g.cs` maps 268 code RVAs, all 13 exports are marked
  managed in `Exports.g.cs`, and only 19 functions are left out (4 CRT library, 1 thunk,
  14 unreachable from the exports). About 27,000 lines in `Rewritten/` plus 7,700 lines of
  runtime (`Runtime/`: CRT, file and registry ports, the module data image). Each unit names
  the original address (`sub_18000D140`, `dword_180066C70`).
- The shipped DLL itself, read with `pefile`: the export table and the MIPS opcode table
  below were dumped from the binary, not copied from Memoria. Both agree with the rewrite.
- Memoria's host side: `Assembly-CSharp/Global/SFX/SFX.cs` (DllImports and the host
  callback), `Global/SFXRender/SFXRender.cs` (primitives to meshes),
  `Global/PSXTexture/PSXTextureMgr.cs` (VRAM).

REA was not usable in this session: its MCP tools are not registered here, and its native
route needs a Ghidra or Hopper engine, which this PC does not have. Installing either would
change machine configuration, which the task rules exclude. The rewrite plus the `pefile`
dumps cover the same ground for this note; a full decompile would matter only for a
function the rewrite gets wrong, and the differential harness in the plan finds those.

## The 13 exports

Export RVAs are from the DLL's export directory. "Host" is Unity (Memoria `SFX.cs`).

| Ord | Export | RVA | Host signature | What it does |
|---|---|---|---|---|
| 4 | `SFX_InitSystem` | 0x1CF0 | `void (Callback)` | Stores the host callback (`qword_1801C1DE8`), then reinitializes the module: registers the VRAM and template buffers with the PSX address mapper, allocates the 0x2EC0-byte call context, clears both ordering tables, uploads the VRAM template (`SfxVramInitUpload`), resets the SFX globals and the camera. No one-time guard. |
| 11 | `SFX_StartPlungeCamera` | 0x1D00 | `void (IntPtr btlseq, int len, int camOffset, int projOffset)` | Copies the camera block at `btlseq + camOffset` and starts its first track (the battle-entry camera); a null `btlseq` reuses the last block. |
| 10 | `SFX_SkipCameraAnimation` | 0x1D10 | `void (int skip)` | Skips the running camera animation. |
| 3 | `SFX_InitBattle` | 0x1D50 | `void (SFX_INIT_PARAM*)` | Per-battle setup: fixed-camera bits, placement, and eight `BTL_DATA_INIT` unit records (radii, camera bones, weapon data) copied into the plugin's object slots. |
| 12 | `SFX_Update` | 0x1D60 | `bool (ref int frameIndex)` | One emulated frame when an effect is loaded: clears the ordering table, resets the 160x120 draw offset, runs the sequencer, the loader and the effect tasks, writes the new frame counter. Returns 0 when nothing is playing. |
| 5 | `SFX_LateUpdate` | 0x1D70 | `void ()` | Swaps the two frame buffers (draw and current); nothing happens without a loaded effect. |
| 13 | `SFX_UpdateCamera` | 0x1DD0 | `IntPtr (int isDebug)` | One frame of the effect camera script; returns the look-at matrix. Tantalus already ports part of this (`battle_camera.rs`). |
| 6 | `SFX_MoveFreeCamera` | 0x1DE0 | `void (int type, int x, int y)` | Debug free camera. |
| 8 | `SFX_SendFloatData` | 0x1DF0 | `int (int type, int btl_id, float, float, float)` | Type 1: camera target position. |
| 9 | `SFX_SendIntData` | 0x1E40 | `int (int type, int a0, int a1, int a2)` | Integer channel, types 1 to 12: 1 caster, 2 target, 3 `SetCamera`, 4 `SetEnemyCamera`, 5 `GetEffectOvRun`, 6/7 get/set the effect camera trigger, 8/9 the Monstera task gate, 10/11 get/set the camera phase, 12 `GetEffectJTexUsed`. |
| 7 | `SFX_Play` | 0x1E50 | `void (int effnum, IntPtr bin, int size, IntPtr req)` | Copies the effect package into module storage, seeds the random generator, reads caster and targets from the 72-byte `S_Eff_Req_Org`, resets the work area and starts the sequencer for `effnum`. |
| 1 | `SFX_BeginRender` | 0x1E60 | `bool ()` | Maps the frame's ordering table for walking. |
| 2 | `SFX_GetPrim` | 0x1E70 | `IntPtr (ref int otz)` | Returns the next GPU packet of the ordering table and its depth (16 times the OT index). Zero-length entries are OT links and are skipped; `0xFFFFFF` ends the walk. |

A frame on the host is: `SFX_Update`, then `SFX_LateUpdate` and `SFXRender.Update`, which
calls `SFX_BeginRender` and drains `SFX_GetPrim` (`SFX.cs:591-653`, `SFXRender.cs:53-87`).

## Host callback

The plugin talks back through one function pointer,
`int Callback(uint code, int sub, int a0, int a1, int a2, void* p)`. The top byte of `code` is
the command, the low bits often a battle object id (`_shared/HostCommand.cs`):

- Unit queries and control, 0x01 to 0x21: get/set position, rotation and scale; motion
  frame, motion and stop; bone matrix; target test; default idle; disappear; weapon, mesh
  and mesh count; status check (0x14); `ExecVfx` (0x17, the HP change); `Btl2dReq` (0x18,
  damage numbers); colors, eye blink, sounds (0x1D to 0x20).
- VRAM, 0x64 to 0x66: `LoadImage` (upload a RECT of 16-bit pixels), `StoreImage` (read a RECT
  back into plugin memory), `MoveImage` (copy a RECT inside VRAM). `PSXTextureMgr` implements
  them on a 1024x512 `u16` array (`SFX.cs:1407-1414`).
- Battle and UI, 0x6E to 0x7D: sequence step, camera config, last-boss flag, effect title,
  cursor, pause (0x74), background intensity, vibration, `CreateTexture` (0x78, the screen
  capture effects use), bonus, start type, alive count, camera number, FPS.

## Effect package

`SFX_Play` gets the `SpecialEffects/efNNN` TextAsset. Tantalus already parses it
(`crates/tantalus-assets/src/pc/special_effect.rs`): a chunk directory at offset 0, 2048-byte
sectors from sector 1, the 3-byte sequence stream at 0x400 and typed records. The loader
(`SfxLoaderStep`, a state machine of seven states) streams records by type:

- 0: image list. Each image is a RECT header and its pixels, uploaded with `LoadImage`; then
  the texture pages from the record's table.
- 2: ring-buffer data (the "small files": cameras, sounds).
- 3: script. The first word is a PSX address rebased on `0x801E7700` (+0x5000 for odd
  groups); it locates a table of 16 entry points. The loader calls the script's init entry,
  then runs `MipsDecodeAll` on the code (next section).
- 4: model textures. A 256-wide CLUT strip to (256, 230), then one 64x128 page per entry.
- 5: summon model header, relocated in place.
- 9: up to eight 64x128 VRAM pages chosen by a bit mask.

The sequence stream is interpreted by `SeqStateUpdate` (3-byte ops `op, a, b`): ops 0 to 20
are control (0 end, 1 wait, 2 set a flag, 4/6/7/8/10 to 13 wait on conditions by rewinding),
ops 32 and up start PSX tasks, which run script entry points.

## MIPS decode and the opcode table

The effect code is real PS1 MIPS. `MipsDecodeAll` (inside `u0067`) converts it once into
16-byte records, one per 32-bit word: `u16` opcode index, `u16` branch flag (the next record
is a delay slot), and three `u32` operands. The opcode table is at RVA 0x66C70: 99 entries of
56 bytes (`u32 mask`, `u32 value`, a name pointer that is null in the shipped build, up to four
operand decoder pointers, a branch byte at +48). The first entry with `word & mask == value`
wins, so order matters (`nop` before `sll`, `move` before `addu`, `li` before `addiu`).

Operand decoders (RVAs): `rs` 0xD0D0, `rt` 0xD0E0, `rd` 0xD0F0, `shamt` 0xD100, `code20` 0xD110,
`simm16` 0xD120, `imm16` 0xD130, jump `target` 0xD140 (made relative to the code base),
`imm25` 0xD150, branch offset 0xD160, `imm16 + rs` 0xD180 (loads and stores, returns both).

Dumped from the DLL; names from `PsxInterpreterRunStep` (`u0106e`):

| Index | Instruction | Index | Instruction |
|---|---|---|---|
| 0x00 | `nop` (exact 0) | 0x26-0x2C | `lb lh lwl lw lbu lhu lwr` (`lwl`, `lwr` act as `lw`) |
| 0x01-0x03 | `sll sra srl` | 0x2D-0x31 | `sb sh swl sw swr` |
| 0x04 | `move` (`addu rd, rs, zero`) | 0x32, 0x33 | `j jal` |
| 0x05, 0x06 | `jalr jr` | 0x34, 0x35 | `beq bne` |
| 0x07-0x0A | `mfhi mthi mflo mtlo` | 0x36-0x39 | `bgez bltz bltzal bgezal` |
| 0x0B-0x0E | `mult multu div divu` | 0x3A, 0x3B | `blez bgtz` |
| 0x0F-0x18 | `add addu and xor srlv sub subu nor or sllv` | 0x3C | `b` (`beq zero, zero`) |
| 0x19-0x1B | `slt sltu srav` | 0x3D, 0x3E | `swc2 lwc2` (GTE data) |
| 0x1C-0x1F | `addi addiu slti sltiu` | 0x3F, 0x40 | `cfc2 ctc2` |
| 0x20-0x22 | `andi ori xori` | 0x41 | `cop2` command |
| 0x23 | `li` (`addiu rt, zero`) | 0x42 | `break`: restart the thread at its code base |
| 0x24 | `li` unsigned (`ori rt, zero`) | 0x43-0x58 | cop0/1/3 loads, stores, branches, moves: no-ops |
| 0x25 | `lui` | 0x59, 0x5A | `mfc2 mtc2` |
| | | 0x5B-0x62 | cop3 moves, cop0/1/3 commands, `syscall`: no-ops |

Rules the interpreter keeps (`PsxInterpreterRun`):

- Registers: one 32-word bank per thread, with its own LO and HI. Each bank has a 4 KB
  stack in the call context; `sp` starts at stack + 4016.
- Branches are delayed: a branch records its target, the delay-slot record runs, then the
  branch is taken.
- A jump target with top byte 0xFF is a native call: `SfxScriptDispatch` with the low 22 bits
  as the call number (next section). Other targets with bit 31 set are absolute PSX addresses
  inside the bank's code block; the rest are code offsets.
- The thread ends when its pc goes negative.
- `div` by zero and `int.MinValue / -1` fault as in the original.
- GTE commands implemented: `RTPS` (0x180001), `RTPT` (0x280030), `NCLIP` (0x1400006),
  `AVSZ3` (0x158002D), `DPCS` (0x780010) and an `RTV0TR` form (0x480012). Any other command
  asserts (`PsxEmulator.cpp` line 0x4E7). Writing `LZCS` sets `LZCR`.

Memory model (`PsxAddrToPtr`, `u0001`): `0x80xxxxxx` below 2 MB is RAM; an address with bits
`0xC00000` set selects one of the 32-byte segment table entries by its top byte; `0x1F8000xx`
is the 1 KB scratchpad. Host pointers handed back to scripts are registered with the address
mapper and returned as PSX-style addresses.

## Native call table

`SfxScriptDispatch` (`u0106d`) serves calls 0 to 215. Arguments are `a0`-`a3` and then the
stack words at `sp + 16`, `sp + 20`, and so on; the result goes to `v0`. The holes 20, 27, 36,
41, 133, 194, 209 and 214 only clear `v0`; 17 and 44 share a handler, as do 192 and 195. By
group (handler names from the rewrite):

- Math and GTE helpers: 5 `RotMatrixYXZ`, 8 `ScaleMatrix`, 9 `CompMatrix`, 14-16 cos, sin
  and atan on the 4096 circle, 18, 45-47 interpolation, 48-49, 52 the LCG random
  (`1103515245 * x + 12345`), 51 distance, 53-62 look angles and transforms, 101, 121-122,
  148, 159 the matrix stack.
- Models: 6, 19, 21-22, 171, 206, 208 register effect models (solid, textured, texture
  list, summon); 24-26, 100, 145, 162-163 draw and animate them; 65, 147, 154-158, 185, 191-198
  color, blend mode, mesh visibility, vertices; 149, 164 summon bone position and matrix;
  151 free a slot.
- Primitive emitters (they append GPU packets to the ordering table): 3, 66-68 ribbons
  and trails, 64 screen tiles, 69-78 sprite lists, frames and scroll layers, 139, 141 a
  primitive at a depth, 144 split prims, 146 the warp grid, 153 wave gradients.
- Particles, pools and tasks: 2, 79-99, 111-120 (particle steps, 56- and 108-byte pools,
  keyframe tracks, PSX tasks, arenas, ribbon and trail slots, script slots), 179, 205.
- Loader and sequencer: 102-110 (table lookups, channels, `SfxLoaderStep`, `SeqStateBegin`,
  `SeqStateUpdate`).
- Host objects (each wraps a host callback): 32-33, 38-43, 123-138, 140, 152, 156, 160-161,
  168-170, 174-178, 183, 186-188, 197, 202-203, 207, 211, 213 (positions, rotations, scale,
  facing, mesh fade, `ExecVfx`, damage figures, eye texture, last-boss flag, status reset).
- VRAM: 0, 1, 166 send a RECT upload, store or move (`HostSendRect64`, `HostSendRect65`,
  `HostSendVramRect66`, callbacks 0x64 to 0x66); 7 writes the draw area; 11-12 start and stop
  summon texture animation.
- Sound: 4 sound FPS, 180-182, 210, 212 the effect voices.

A few handlers check the effect id first: Sinkhole skips calls 19, 28, 30, 199 and 200, Slow
skips 69 and Stop skips 143.

## VRAM model

- One 1024x512 array of `u16` pixels (PS1 VRAM), owned by the host (`PSXTextureMgr`). The
  plugin keeps its own copy of the regions it uploads and reads back regions with
  `StoreImage`.
- At `SFX_InitSystem`, `SfxVramInitUpload` builds a 64x192 block from three templates in the
  DLL's data (2,048 + 8,192 + 14,336 bytes) and uploads it at (768, 320); restores a 67,584-byte
  page buffer and uploads its four pages; uploads a 256x2 strip at (0, 240). The first 2,048
  bytes of that block are the per-effect flag words (1,024 `u16`, RVA 0x4F020) that Tantalus
  already reads for the forced-camera bit (`plugin_effect_flags`).
- Effect records upload images (type 0), CLUT strips and 64x128 pages (types 4 and 9).
- Primitives address textures the PS1 way: a texture page (`tpage`: x in 64-pixel steps, y in
  256-pixel steps, color depth 4, 8 or 15 bits, blend mode) and a CLUT position. The host
  turns each (page, CLUT) pair into a texture on demand (`PSXTextureMgr.GetTexture(TP, TY, TX,
  clutY, clutX)`) and caches it by key.
- `CreateTexture` (0x78) and the blur capture copy the rendered screen into VRAM for the
  effects that distort the screen.

## Porting plan

The port is a translation of Memoria's rewrite into Rust, checked frame by frame against the
shipped DLL. Each step ends with a test or a capture.

1. Oracle harness, local only. A `tantalus-trace` subcommand loads the player's
   `FF9SpecialEffectPlugin.dll` with `libloading` (Windows x64), installs a recording host
   callback, plays one effect against a fixed `SFX_INIT_PARAM` and request, and writes per
   frame: every callback (code, arguments, RECT pixels hashed), the `SFX_GetPrim` packet
   stream, and the camera matrix. Output goes under `local/` and is never committed. This is
   the reference for every later step and needs no window.
2. Runtime core in a new `tantalus-sfx` crate: the PSX address mapper and segments, the
   module data image (data sections read from the player's DLL at run time, like
   `plugin_effect_flags`, so no game bytes are shipped), the GTE subset, `MipsDecodeAll` with
   the opcode table read from the DLL, and the interpreter. Test: decode every effect
   package's script and compare record buffers with the oracle's.
3. Loader, sequencer and VRAM: `SFX_InitSystem`, `SFX_Play`, `SfxLoaderStep`,
   `SeqStateUpdate`, and a 1024x512 VRAM with `LoadImage`, `StoreImage` and `MoveImage`. Test:
   the VRAM after loading each effect matches the oracle's uploads.
4. The native call table, group by group, starting with what Fire, Cure and the Trance
   effects (257, 489) use: math, models, primitive emitters, then particles, host objects and
   sound. Test: the packet stream of each effect matches the oracle frame for frame.
5. Client renderer: decode GPU packets (flat, Gouraud and textured polygons, sprites, lines,
   draw-mode packets) into `tantalus-client` meshes, textures from VRAM by page and CLUT,
   PS1 blend modes. Replace the stand-in effects in `battle_effects.rs` effect by effect.
   Test: headless captures next to Memoria's exported meshes (`SFXRender` debug export).
6. Battle wiring: host callbacks drive the battle machine (`ExecVfx` applies the HP change,
   `Btl2dReq` shows numbers), which replaces the timing tables in `EffectTable` and the cast
   lead model; camera exports replace the partial `battle_camera.rs` port.

Risks and open points:

- Size: about 35,000 lines of C# to translate. Steps 2 and 3 are about 4,000 lines and give
  the first useful result (correct VRAM and textures); step 4 is most of the work.
- The rewrite is pointer-heavy unsafe C#. A Rust port keeps the PSX memory as byte arrays
  with typed accessors rather than raw pointers; host pointers become indices.
- The oracle runs native code from the player's install. It is a dev tool only and stays out
  of the shipped engine.
- Memoria's own SFX rework (`UnifiedBattleSequencer`, `SFXDataMesh`) is an optional feature
  and out of scope; the target is the vanilla plugin's behavior plus logged bug fixes.
- Memoria is MIT-licensed (Albeoris, 2017); a translation keeps its copyright notice next to
  the GPL code.

## Port status (crates/tantalus-sfx)

Each part is checked against the player's DLL by the `dll-oracle` tests
(`cargo test -p tantalus-sfx --features dll-oracle --test oracle -- --ignored`). They are
test-only and never a runtime fallback; CI builds neither the feature nor the tests.

| Part | Module | Check against the DLL |
| --- | --- | --- |
| Data sections by RVA | `image` | used by every check below |
| VRAM, page decode | `vram` | Fire (ef000) and Cure (ef008) pages decode the same; PNGs in `local/sfx` |
| Init uploads, loader records 0, 1, 4, 9, page restore | `loader` | init uploads match; effect uploads match for 370 of 372 effects |
| `MipsDecodeAll` with the table read from the DLL | `mips` | 381 of 381 scripts decode to the same records (a fresh plugin per effect) |
| GTE: RTPS, RTPT, RTV0TR, NCLIP, AVSZ3, DPCS, MFC2/MTC2 | `gte` | 8,000 random register files give the same results from the DLL's own functions |
| `PsxInterpreterRun` | `interp` | unit tests only; frame-level checks need the native calls |
| Flat memory (module image at 0x180000000, heap), PSX address mapper | `mem` | unit tests only |

| The whole plugin, translated from Memoria's rewrite | `plugin` | 358 of 372 effects give the same primitives, frame for frame, and the same host calls (`--test plugin sweep`) |

The `plugin` module is generated: `crates/tantalus-sfx/cs2rs` is a Roslyn translator that reads
Memoria's C# rewrite (`.reference/Memoria/FF9SpecialEffectPlugin`) and writes
`src/plugin/generated.rs` over the flat memory model (pointers are `u64` addresses, module
globals are loads and stores at their preferred-base addresses, address-taken locals live on
a native stack in `Memory`). Regenerate with
`dotnet run -c Release --project crates/tantalus-sfx/cs2rs -- <Memoria plugin dir> crates/tantalus-sfx/src/plugin/generated.rs`.
`src/plugin/rt.rs` holds what the generated code calls (loads, stores, the stack, indirect
calls by RVA, the host callback). The translator patches the C# where the rewrite and the DLL
disagree (`Patches` in `Program.cs`, each logged in `audit/approved-differences.csv`).

Checks (`cargo test -p tantalus-sfx --features dll-oracle --test plugin -- --ignored`):

- Fire (ef000): 126 frames, 28,858 primitives and 208 host calls, all identical.
- Cure (ef008, also Potion): 121 frames, 24,185 primitives and 191 host calls, all identical.
- `sweep` over effects 0-511: 480 of 480 match (the run counts each effect id the DLL has).
  The last 19 (212-224 and 341-346) were a harness fault: for GetMatrix (host command 14, mode 2)
  the oracle callback and `DebugHost` wrote only 6 bytes of the 32-byte `PSXMAT`, and the plugin
  does not clear the buffer, so the DLL read its own stack (rotation and translation garbage that
  moved with the address space; 341 got TRY = 16 and spawned its particle one pixel off). Both
  hosts now write the whole matrix (identity rotation, translation at the unit), as `SFX.cs` and
  `BattleHost` do. Photon (299) matches after pinning one stack word (see Known oracle differences).
  The sword skills Darkside, Minus Strike, Iai Strike, Thunder Slash (188-191, 409-412) and
  Lancer (401) now match: Memoria's `SfxOpCtx16MoveNearObj` init had the flag test inverted (the
  DLL keeps the task running; the rewrite ended it in its first frame), found with
  `probe_functions` (the functions the DLL and the port call in one frame, from INT3 hits).
- The short effects (Player_Attack 100-118, SFX_Attack 313-376 and 461-485,
  `SFXData.IsShortSpecialEffect`) have no `ef<id>` package: `SFX.PlaySFX` calls `SFX_Play` with a
  null buffer and size 0, and the plugin plays the effect's own script from its module data.
  The harness and the battle view do the same (an empty package), which is why the Player
  Attack effects had no timing (the lookup found no asset) and drew nothing. All 19 Player
  Attack effects, 64 of 64 in 313-376 and 25 of 25 in 461-485 match the DLL.
- Until the 32-byte `BTL_DATA_INIT` fix (the harness wrote 36-byte records), no effect found
  its target and every one played on the caster; the earlier 359 of 372 was for that case.

Memory layout matters for matching: the PSX address mapper opens 3 MB windows over host
blocks, so heap blocks start 4 MB apart and the stack grows down, as on Windows, or the port
opens different windows from the DLL.

What Fire and Cure run, from `coverage_probe` (an INT3 on each of the 268 functions Memoria
gives an address for; first calls only, so callees without an address are not listed):

- Fire (ef000, 126 frames), 59 functions: `ResetSfxWorkArea`, `SfxPoolsReset`, `SeqStateBegin`,
  `SeqStateUpdate`, `SfxLoaderStep`, `SfxTickEffectTasks`, `sub_1800354E0`,
  `SfxUpdateParticles`, `SfxUpdateKeyframeTracks`, `StartPsxTask`, `PsxTaskScriptHandler`,
  `SfxObjectStrikeSequence`, `SfxTrailSequenceHandler`, `PsxObjectAlloc`, the `Host*` object
  queries and commands, `SfxObjTurnTowardsPoint`, `SfxTurnTowards`, `CreateRibbonEffectSlot`,
  `CreateTrailEffect`, `GteIntplVector`, `RotateDepthVector`, `TransformPointByAngles`,
  `SfxRotTransPointZ`, `ComputeLocalYawDistance`, `DrawRingMeshEffect`, `SfxEmitParticles`,
  `EmitWorldSpriteList`, `EmitSpriteCellFrame`, `EmitWorldSpriteFrame`, `PsxAllocDrawMode`,
  `SfxOpVoicePlay`, `SfxVoicePlay`, `SfxOpObjMessage`, `SfxOpHostLevelFade`,
  `SfxOpMoveObjToPoint`, `FindPairSlot`, `DetachSfxObject` and the MIPS decoders.
- Cure (ef008, 121 frames) adds `DrawFacingRibbon`, `DrawRibbonTrail`,
  `EmitInterpolatedSpriteTrail`, `EmitScreenSpriteList` and `SfxIndexedRecordPtr`.

Harness notes:

- Effects played back to back in one plugin instance can hang inside the DLL (211 and 228
  after the effects before them) and leave buffers that change later decodes. The sweeps
  load a fresh plugin for each effect.
- RTPS multiplies by the address of the DQA register, not its value (Memoria keeps this). The
  DLL is relocated at load, so the original result changes from launch to launch. Tantalus
  uses the preferred-base address; see `sfx-gte-rtps-dqa-address` in
  `audit/approved-differences.csv`.

## In the battle view (tantalus-client)

`battle_effects` plays every party spell and item effect through the translated plugin
(`tantalus_sfx::SfxRuntime::play`, a fresh plugin per effect), effects 212-224 included:

- Background capture (host command 120): `BattleHost` keeps the seven capture words, the player
  sets the capture page (`Vram::background`), and the view renders the first ground mesh into
  a 256x256 texture (`battle_view::capture_ground`, `SFXRender.CaptureBG`); batches that sample
  it (`Source::Background`) draw from that texture, as `SFXMesh.GetTexture` does.
- Unit staging (commands 2-6, 17, 18, 25, 27): `UnitChange` records hide, move, turn, scale
  and fade per unit; the view applies it when it draws the units, so summons hide the party,
  Mini scales and Jump moves. Vanish (command 25, mode 1) hides the unit's `mesh_banish` meshes
  (`BattleStage::banish_meshes`: the enemy type's `Mesh[1]`, 65535 for the party and for the
  enemies that use a party model, `IsUseAsEnemyCharacter`) by the model's `mesh<N>` objects, and
  leaves the weapon (`btl_mot.HideMesh`); the exact fade curves are approximate.
  Mini's glow on the caster is its charge phase; the ring on the target follows about 2.5 s later.
- Jump and Spear (393, 388): the plugin drives Freya's animation (command 12, `Set Current
  Animation`; command 13 stops it), and `SFX.cs` swaps MP 26-28 for her jump clips
  (`ANH_MAIN_B0_011_110` to `112`). `EffectMotion` keeps the clip and its frames; the view draws
  the jumper with it while Jump plays (she leaves when the command ends) and once Spear shows her
  again (command 18 off, frame 58) to fall. Jump has no `EffectApplied` of its own, so its start
  takes the default lead rather than Spear's.
- Slow's clock and Stop's black sprite: `SFXMesh.End` swaps Slow's subtractive mesh for the
  clock texture (`slowClockUV`) and draws Stop's sprite subtractively, twice; both are drawn.
- Sounds (command 32): the plugin's sound calls keep their frame and fall due that many ticks
  after the effect starts (`tantalus-tools` `battle_sound`, through `SFX.AdjustSoundIndex`).
- Units: `SFX.InitBattleParty` from the stage (`btl_list` order, party then enemies, eight at
  most). `BTL_DATA_INIT` is 32 bytes (`SFX_INIT_PARAM` is 300); the plugin looks targets up by
  `btl_id` in those records and falls back to slot 0, the caster, when it finds none.
- The host (`tantalus_sfx::player::BattleHost`) answers `SFX.BattleCallback` from the units'
  places this frame: positions, angles, sizes, the party member flags, the background
  intensity, and records the Effect and Figure points and the sounds. Bones are the posed model's `bone<NNN>`
  world matrices (`Unit::bones`, `sfx-battle-bones`).
- Camera: the view's camera, converted back to the PSX camera
  (`battle_effects::psx_camera`, the inverse of `battle_camera_play::unity_camera`), is
  written where `SFX_Update` reads it before every frame, so the packets line up with the
  models. The plugin's own camera script is not run.
- Timing: one plugin frame per battle frame (`SFX.UpdatePlugin` runs once
  per `BattleSubSystem`). The battle machine takes each effect's HP change (the plugin's
  Effect point, callback 23), numbers (Figure point, callback 24) and end from the plugin
  itself (`EffectTable::load_plugin_timing`; each effect is played once, on first use, in
  `command_timing_of`), so the burst and the HP readout land together
  (`sfx-battle-hp-change-timing`).
- Drawing: `tantalus_sfx::render::render_list` turns each frame's packets into the batches
  `SFXRender` draws (opaque, additive and subtractive lists keyed by page, CLUT and filter,
  in `PushCommandBuffer` order, with `playParam` color strength) and the view draws them over
  the scene with `Frame::draw_screen` (`Blend::Add`, `Blend::Subtract` on the CPU and the GPU).
  Batches that sample the screen (the blur texture: 15-bit pages left of page 6) draw from a capture of the frame taken
  before the effect (`Frame::capture`; on the GPU a `Command::Capture` copies the target between passes). The `DR_MOVE`
  screen shots of Slow and Silent Voice are `Batch::capture` markers: the view captures the frame there and the later
  page 7 and 9 batches sample it (`sfx-battle-screenshot-twist`).
  Packets between a `DR_AREA` begin and end are drawn into a persistent 512 x 256 texture (VRAM from 384, 256), which every
  later 15-bit page not otherwise handled reads (`sfx-battle-generated-texture`); Devour's page reads the screen.

`SfxRuntime::load_effect` runs the translated plugin to the effect's end and returns its
`LoadImage` uploads; `loader::effect_uploads` stays for the oracle tests.

Checks:

- `battle_effects::tests::the_plugin_draws_fire_on_the_target` (needs the game data).
- `capture-battle` with `TANTALUS_SFX_DUMP=<file>` writes each played frame's units and camera;
  `cargo test -p tantalus-client --features dll-oracle the_battle_frames_match_the_dll --
  --ignored` replays them on the DLL and on the plugin and compares the render lists frame by
  frame (`TANTALUS_SFX_EFFECT`, `TANTALUS_SFX_FRAMES`, `TANTALUS_SFX_OUT` for side-by-side
  PNGs). In the first battle (336, Fire, Cure and Potion recast onto Zidane's first command, on
  Baku) all 90 frames of each match.

## Known oracle differences

- Photon (299): `SfxUpdateParticles` never clears its stack frame, and the attractor kinds 3 and 4
  read its first quadword (`Pos`) before any particle writes it. A zeroed frame is off by one pixel
  in one vertex from frame 81. The DLL's slot holds 2 at frames 80 and 81 (read at an INT3 hit
  with `probe_hit`), so `Plugin::play` pins 2 for effect 299 only (other effects keep 0). The
  real stack content is not reproducible in general. Logged as `sfx-photon-particle-pos`.
