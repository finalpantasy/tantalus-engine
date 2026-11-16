# Battle conformance evidence — 2026-11-16

State of the battle subsystem's correctness evidence. Everything below is
reproducible from the committed tree plus an owned PC installation; no
original assets are committed.

## What runs

Three independent engine paths execute real battles from the owned install
(`p0data2.bin` scenes, `p0data7.bin` scripts):

- **Report path** (`run_scene_battle`): assembled battle, script-driven
  enemy AI, outcomes and rewards reported per battle.
- **World hand-off** (`run_world_battle`): party built from scratch field
  world state, script AI, decided-defeat gate, death-driven `set_bonus`,
  exactly-once reward commit through `commit_rewards`, HP/MP carry-back.
- **Chain engine** (Opus's `FieldBattles::fight`): the field-script chain
  path, cross-checked against the world hand-off in unit tests.

## Committed baselines

| Audit | Battles | V / D / E |
|---|---|---|
| `audit/battle-run-all-patterns.json` (report path, seed 0x5EED) | 1054 | 1002 / 38 / 0 |
| `audit/battle-world-run-all-patterns.json` (world hand-off, seed 0x5EED) | 1054 | 1002 / 38 / 0 |

The 38 defeats are the endgame superboss set (the armed level-20 demo party
wipes). The 5 fog battles (`TH_E005`, `WM_9900-9903`) assemble clean and
reach no outcome on both paths; both paths record that as a non-error row.

## Cross-path agreement

`audit compare-sweeps` between the two baselines reports `identical: true`,
zero deltas on outcome, gil, EXP, and error — all 1054 battles. Re-runs are
bit-identical. At seed 48879 (0xBEEF) both paths again produce identical
rows; outcomes, gil, and EXP match the 0x5EED baselines on every row, while
item drops differ on 200 rows and card drops on 4 — variance lives exactly
in the draw stream (`seed ^ 0xD1AA`), confirming the seed is wired in and
the reward channels are deterministic where the reference says they should
be.

## Gates

- `cargo test --workspace` — 0 failures (cross-path conformance, seed loops,
  exactly-once rewards, flee economics, save/reload ledger, error fidelity).
- `tantalusctl audit summary --deny-fail` — both battle corpora gated; any
  sweep error or stale audit fails.
- `tantalusctl audit compare-sweeps <a> <b>` — the standard regression
  check after any behavioral landing.

## Known limits

- Defeat *presentation* (Timing-injected close sequence) is Claude's lane;
  outcomes are reported faithfully via the decided-defeat gate in the
  meantime.
- Battles other than the 5 fog battles always resolve within the tick
  budget; the demo party is deliberately dominant, so hit-roll variance
  does not flip outcomes (verified across two seeds).
- Item/card drops are not cross-path comparable by design: each path owns
  an independent draw stream.
- **Weak-party sweeps (`--battle-weak-party`) currently diverge between
  the two paths: 31 outcome changes / 1054 battles.** Root cause found:
  `BattleScriptDriver::new` takes the party's character-id list and spawns
  one event object per character, but the report path passes `&[0]` and
  the world hand-off passes `&[]` (`crates/tantalus-tools/src/battle_run.rs`),
  so enemy scripts see different player objects and make different
  decisions in long battles. The armed default never surfaces this (battles
  end too fast for AI party-queries to matter) and stays bit-exact across
  paths. Reference-correct party spawning is with the script-host owner;
  until it lands, weak-party mode is a diagnostic, not a conformance gate.
