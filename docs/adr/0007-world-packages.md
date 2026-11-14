# Additive packages and persistent world actors

Packages stage schema-validated JSON records into the shared registry. The content hash covers the actual ordered records and their data, not just manifest paths. No original record is reused to create the synthetic raptor, rig, attack, behavior, weapon, or spawn region.

World actors retain stable identities through cell unload/reload. Defeat, cancellation, and fleeing are distinct encounter outcomes. Respawn uses the region's declared tick delay; inactive cells do not silently reset persistent state.

World updates use fixed-point positions and deterministic integer movement. Perception, population ownership, encounter transitions, combat rules, and presentation remain separate. The current package proves declarative content only; custom executable AI belongs to the later mod host.
