# Battle action and effect ownership

Combat rules own costs, targets, action timing, hit resolution, lifecycle, victory/defeat, and reward submission. Semantic effect, audio, hit, and completion markers are shared contract data. The client consumes those markers without deciding damage or progression.

Actions advance on the authoritative tick schedule. Multiple markers crossed by one step are emitted once in order. Action completion is separate from battle completion, so nonlethal and missed actions finish without error.

Rewards carry a resolution ID and are submitted through `GameServices` as one commit. The ruleset rolls back its internal step on failure; host implementations must provide atomic reward commits. Full save/journal recovery belongs to T10.

The current battle is synthetic. Original ability formulas, effect resources, media timing, and real reference comparisons remain compatibility work.
