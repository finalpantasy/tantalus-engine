//! Memoria `KeepRestTimeInBattle` is an opt-in: off by default (vanilla),
//! switched on by `set_keep_rest_time_default` (the `--keep-rest-time` flag).
//! Kept in its own test binary because the switch is process-wide.

use tantalus_legacy::battle::machine::{BattleConfig, set_keep_rest_time_default};

#[test]
fn keep_rest_time_is_off_until_opted_in() {
    assert!(!BattleConfig::default().keep_rest_time);
    set_keep_rest_time_default(true);
    assert!(BattleConfig::default().keep_rest_time);
    set_keep_rest_time_default(false);
    assert!(!BattleConfig::default().keep_rest_time);
}
