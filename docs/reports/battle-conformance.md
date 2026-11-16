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
| `audit/battle-run-all-patterns-weak.json` (report path, unarmed level-8 party) | 1054 | 460 / 544 / 0 |
| `audit/battle-world-run-all-patterns-weak.json` (world hand-off, weak party) | 1054 | 460 / 544 / 0 |
| `audit/battle-run-all-patterns-flee.json` (report path, flee-key at tick 0) | 1054 | 3 / 387 / 593 escapes |
| `audit/battle-world-run-all-patterns-flee.json` (world hand-off, flee-first) | 1054 | 3 / 387 / 593 escapes |

The 38 armed defeats are the endgame superboss set (the armed level-20 demo
party wipes). The weak pair deliberately loses most battles (544 defeats) so
the defeat path — the decided-defeat gate, reward suppression, HP carry-back
under mass defeat — is covered at corpus scale, not just by the superboss
set. The 5 fog battles (`TH_E005`, `WM_9900-9903`) assemble clean and reach
no outcome on both paths; both paths record that as a non-error row.

## Cross-path agreement

`audit compare-sweeps` between the armed baselines reports `identical: true`,
zero deltas on outcome, gil, EXP, and error — all 1054 battles. Re-runs are
bit-identical. At seed 48879 (0xBEEF) both paths again produce identical
rows; outcomes, gil, and EXP match the 0x5EED baselines on every row, while
item drops differ on 200 rows and card drops on 4 — variance lives exactly
in the draw stream (`seed ^ 0xD1AA`), confirming the seed is wired in and
the reward channels are deterministic where the reference says they should
be.

The weak pair is also cross-path identical (zero deltas over 1054
mass-defeat battles), and the flee-first pair is cross-path identical over
593 escapes + 387 defeats — the escape family (flee-key roll, SysEscape
fade, escape-pays-nothing economics, no HP/MP loss beyond battle damage) is
covered at corpus scale. Getting there required fixing two real divergences
the weak mode exposed: (1) the two paths passed different battle-script
party lists (`&[0]` vs `&[]`) so enemy scripts saw different player
objects — both now pass the machine units' character ids (Zidane..Steiner,
per the reference `btl_init` CharacterId binding, `ObjTable.ReadData`
vanilla links); (2) the report path's demo units never set defensive
stats while the world base closure did — now aligned (armed 10/5/12/6,
weak 4/2/5/2). After both fixes the weak dual sweep converged from 31
outcome deltas to zero, and the armed baselines reproduce bit-for-bit
(the armed party kills too fast for either difference to matter — which
is exactly why the weak gate exists).

## Gates

- `cargo test --workspace` — 0 failures (cross-path conformance, seed loops,
  exactly-once rewards, flee economics, save/reload ledger, error fidelity).
- `tantalusctl audit summary --deny-fail` — all six battle corpora gated
  (armed / weak / flee-first × both paths); any sweep error or stale audit
  fails.
- `tantalusctl audit compare-sweeps <a> <b>` — pairwise delta check.
- `tantalusctl assets inspect-bundle --path p0data2.bin
  --battle-run-everything <out dir> --scripts-bundle p0data7.bin` — the
  standard post-landing triage: regenerates all six corpora in one
  process and prints each against its committed baseline. A clean tree
  prints `identical=true` on all six lines.

## Performance

Absolute capacity numbers (no comparison claim; Memoria has no headless
equivalent harness). Windows 11, release build, 2026-11-16:

- Full armed all-patterns sweep (1054 battles incl. scene/script parse):
  ~37 s cold, ~21 s warm file cache — roughly 20 ms per battle average.
- The world-hand-off path (party from world state, exactly-once commit)
  runs at the same order of magnitude; flee-first sweeps run longer
  because failed flee rolls let battles play out to defeat.

These are simulation-through-reward numbers with script-driven enemy AI,
not presentation frame rates.

## Seed sensitivity

`--battle-seed` controls all three random streams: script AI
(`SplitMix64(seed)`), drop draws (`SplitMix64(seed ^ 0xD1AA)`), and the
calculator stream through `GameServices` (the battle world's RNG, seeded
`seed ^ 0xC0FFEE11DEADBEEF`). Before the world stream was seed-controlled,
hit rolls and flee rolls used a fixed stream regardless of the knob.
Wide draws (the flee roll asks for 65_536) compose whole bytes from the
world RNG, matching the reference's 16-bit draw width exactly (65_536 is
a power of two); narrow draws stay one byte.

Outcomes are seed-sensitive to a degree that depends on the policy:
the armed party flips only a couple of marginal battles between seeds;
the weak party flips a small set (its survival is stochastic); flee-first
flips several hundred (the flee roll runs through the calculator, so
re-seeding changes escape success per battle). None of this is noise --
it is the engine's stochasticity finally visible on every stream.

The invariant that holds at EVERY seed is cross-path agreement: both
engine paths flip the same battles in lockstep, so the corpora compare
bit-identical between report and world under any seed. Consequence: all
committed baselines are seed-specific (0x5EED); always regenerate or
compare at the same seed (`--battle-seed` defaults to 0x5EED), which is
what the gate and the run-everything triage do.

## Known limits

- Defeat *presentation* (Timing-injected close sequence) is Claude's lane;
  outcomes are reported faithfully via the decided-defeat gate in the
  meantime.
- Battles other than the 5 fog battles always resolve within the tick
  budget; the demo party is deliberately dominant, so hit-roll variance
  does not flip outcomes (verified across two seeds).
- Item/card drops are not cross-path comparable by design: each path owns
  an independent draw stream.
- Spawn semantics inside `BattleScriptDriver::new` (all entries vs entry 0
  + player-linked) are still Claude's open question from the 05:30 mailbox
  note; the tool paths now feed reference-shaped party ids, and the weak
  gate will catch any spawn-side change that moves outcomes.
