//! HP+/MP+ support abilities in the field maximums (`ff9play.FF9Play_Update`,
//! `CharacterAbilityGems.TriggerOnEnable`, `AbilityFeatures.txt` SA 5 to 8).

use std::sync::Arc;

use tantalus_game::field::world::{CharacterStats, FieldWorld, support_max_hp_mp};

fn stats(hp: i32, mp: i32) -> CharacterStats {
    CharacterStats {
        current_hp: hp,
        max_hp: hp,
        current_mp: mp,
        max_mp: mp,
    }
}

/// Each enabled ability runs its formula on the running value in `saExtended` order, the two
/// sizes stack (no boosted hierarchy in `AbilityGems.csv`) and the limits are 9999 and 999.
#[test]
fn hp_and_mp_abilities_stack_in_enable_order_up_to_the_limits() {
    assert_eq!(support_max_hp_mp(1000, 100, &[]), (1000, 100));
    assert_eq!(support_max_hp_mp(1000, 100, &[5, 7]), (1100, 110));
    // 1000 + 100 = 1100, then + 1100 / 5 = 1320; MP 100 + 20 = 120.
    assert_eq!(support_max_hp_mp(1000, 100, &[5, 6, 8]), (1320, 120));
    // Order matters with integer division: 233 + 23 = 256, + 51 = 307;
    // 233 + 46 = 279, + 27 = 306.
    assert_eq!(support_max_hp_mp(233, 0, &[5, 6]).0, 307);
    assert_eq!(support_max_hp_mp(233, 0, &[6, 5]).0, 306);
    assert_eq!(support_max_hp_mp(9000, 990, &[6, 8]), (9999, 999));
    // Other support abilities leave the maximums alone.
    assert_eq!(support_max_hp_mp(500, 50, &[0, 3, 34]), (500, 50));
}

/// The field record's maximums follow the enabled set, the current values are capped when an
/// ability is switched off, and a level-up keeps the bonus.
#[test]
fn the_field_maximums_follow_the_enabled_set_and_level_ups() {
    let mut world = FieldWorld::new();
    world.level_growth = Some(Arc::new(|_, exp| {
        Some(if exp >= 100 {
            stats(300, 40)
        } else {
            stats(200, 30)
        })
    }));
    world.character_stats.insert(0, stats(200, 30));
    world.support_enabled.insert(0, vec![5, 7]);
    world.update_character_max(0);
    let record = world.character_stats[&0];
    assert_eq!((record.max_hp, record.max_mp), (220, 33));
    assert_eq!((record.current_hp, record.current_mp), (200, 30));

    world.character_stats.get_mut(&0).unwrap().current_hp = 220;
    world.support_enabled.insert(0, vec![7]);
    world.update_character_max(0);
    let record = world.character_stats[&0];
    assert_eq!((record.current_hp, record.max_hp), (200, 200));

    world.support_enabled.insert(0, vec![5]);
    world.update_character_max(0);
    world.apply_experience(0, 100);
    let record = world.character_stats[&0];
    assert_eq!((record.max_hp, record.max_mp), (330, 40));
    // The level-up adds the maximum's rise (330 - 220) to the current HP.
    assert_eq!(record.current_hp, 200 + 110);
}
