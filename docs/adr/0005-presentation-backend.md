# Bevy presentation backend prototype

Bevy 0.19.1 is pinned for the initial client because it supports the project's Rust 1.96 toolchain and provides 2D/3D rendering, glTF/animation infrastructure, and image-target capture. Bevy 0.20.1 requires a newer Rust release and was not selected.

The headless app disables Winit and creates no primary window. It renders a private Bevy scene into an image and copies render-target bytes through Bevy's documented image-copy render-graph path. No FFIX assets are required for the synthetic scenes.

`PresentationSnapshot` uses engine IDs and fixed-point positions. Bevy entities, handles, worlds, and component layouts do not enter public contracts. The prototype rebuilds private presentation entities on `apply`; this is not an incremental-rendering performance claim.

The adapter uses sprites, circles, and cuboids to validate composition and backend behavior. Original FFIX field projection, skinned actors, hybrid visual fidelity, and real asset comparison remain unverified.

## Current visual result

The visual acceptance check is failing. Tantalus 2D/3D captures and the unmodified official Bevy 0.19.1 headless renderer control produced blank images on both Vulkan and DirectX 12. The control source was removed after the experiment; generated captures remain ignored under `local/`. This does not establish visual support or isolate the defect to Tantalus. The ignored conformance test and `tasks/T05.md` preserve the open failure.
