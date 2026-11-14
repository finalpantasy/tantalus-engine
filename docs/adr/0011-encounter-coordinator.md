# Encounter coordinator and presentation ownership

`EncounterCoordinator` owns the lifecycle shared by original-arena, current-scene, and prepared-arena encounters. Plans pin the session, content revision, generation, participants, rules, presentation, pause policy, resources, and spatial mapping. Resource readiness is staged and externally reported.

Participants and resources use explicit leases. Only a ready encounter may transition into combat. Cancellation and resource failure before outcome commitment release ownership without granting rewards. After combat commits a resolution ID, generic cancellation is rejected; the encounter must complete its return/recovery path. Stale, duplicate, and out-of-order events fail without partially changing coordinator state.

`SpaceTransform` uses checked fixed-point arithmetic for world/arena mapping. `EncounterPresenter` translates commands into camera, UI, and input actions but cannot decide combat outcomes or progression.

Current evidence is synthetic. Real camera/asset integration, transition frame-time capture, and original FFIX reference comparisons remain pending; the separate headless visual failure in T05 is unchanged.
