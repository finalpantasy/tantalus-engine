# Backend selection gate

Bevy remains the provisional presentation lead, not the proven final choice. Fyrox is a serious alternative because of its existing editor and animation tooling. A direct wgpu implementation remains a fallback for a demonstrated integration blocker. Godot with Rust bindings is outside the independent Rust-engine direction.

Use one canonical fixture bundle and small candidate adapters. Do not build two game implementations or two workshops. The gate is bounded by one synthetic field, one rigged actor, one action/effect sample, one mixed-cinematic sample, and one artist round-trip per candidate; real-reference probes remain pending until owned assets are available.

Required evidence:

- Original field projection, masks, transparency, animated overlays, and actor placement.
- Real rig attachments and animation timing without giving the renderer authority over hits or movement.
- Effect and movie/live-scene timing, cleanup, and return of control.
- A 3D arena with observable readiness and cancellation.
- Artist edit, undo, save, export, reopen, and reimport without losing canonical IDs or metadata.
- Desktop input, scaling, localization, accessibility, packaging, and lifecycle behavior.
- Custom code, local patches, build/iteration time, copies, allocations, frame times, memory, and unresolved maintenance work.

An observed loss of canonical data or violation of simulation authority is a blocker. An unfinished optional integration is a named risk. Select one production backend after the evidence; do not infer suitability from a graphics-backend label or feature list.

Current status: Bevy 0.19.1 structural checks pass, but visual qualification is failing. Tantalus and the official Bevy headless control produced blank captures on Vulkan and DX12. Fyrox evaluation has not started. No final selection is recorded.
