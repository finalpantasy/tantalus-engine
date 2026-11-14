# Provenance and evidence register

## Reference identities

- Normative Memoria source snapshot: `c716e1faaeee8c856cc10921234d017f1187b45b`, verified against the local checkout.
- Memoria stable tag: `v2026.11.09`, verified locally at `677c77c6928141bf2fe909df3b3446b4c194893c`.
- The source snapshot and stable tag are distinct references; neither is evidence of the contents of a release binary.
- No release binary has been acquired or hashed.
- No owned FFIX PC installation has been supplied or inspected.

## Evidence rules

- `observed`: directly read from a pinned source or inspected local artifact.
- `plan-supplied`: recorded from the approved master plan but not independently rechecked.
- `inferred`: an engineering interpretation that requires validation.
- `unknown`: unavailable or not yet investigated.

Original assets, extracted proprietary payloads, private saves, and release binaries are not public fixtures. Synthetic tests must use project-owned generated data. Source references do not establish runtime behavior or asset compatibility.
