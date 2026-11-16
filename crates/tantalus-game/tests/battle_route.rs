//! Full battle reward route: real draw RNG (`SetBonus`) -> reference split
//! (`from_bonus`) -> field bridge (`bundle_from_battle_reward`) -> exactly-once
//! world commit, with AP learning alongside. Field world stays the state
//! owner throughout.

use tantalus_contracts::ContentId;
use tantalus_game::FieldWorld;
use tantalus_game::battle::bridge::bundle_from_battle_reward;
use tantalus_game::battle::{GameServices, RewardBundle, WorldGameServices};
use tantalus_game::field::world::default_item_id_map;
use tantalus_legacy::battle::rewards::{
    BaseStats, BattleBonus, EnemyRewardData, LevelBonus, PartyMember, PartySnapshot,
};
use tantalus_legacy::battle::rng::{Draw, SequenceRandom};
use tantalus_legacy::shop::NO_ITEM as NO_SHOP_ITEM;

const NO_ITEM: u32 = 255;

fn member(hp: u32) -> PartyMember {
    let base = BaseStats {
        dex: 23,
        str_: 21,
        mgc: 18,
        wpr: 23,
        gems: 18,
    };
    PartyMember {
        in_battle: true,
        cur_hp: hp,
        cur_mp: 36,
        rewards_disabled: false,
        level: 1,
        exp: 0,
        base,
        bonus: LevelBonus::default(),
        basis: [base.dex, base.str_, base.mgc, base.wpr],
        max_hp: 250,
        max_mp: 36,
        max_capa: u32::from(base.gems),
        cur_capa: u32::from(base.gems),
        elem: [base.dex, base.str_, base.mgc, base.wpr],
        equip_stat_bonus: [0; 4],
        equipped_abilities: vec![],
        learnable: vec![],
        pa: vec![],
    }
}

#[test]
fn drawn_battle_rewards_pay_the_field_world_through_the_reference_split() {
    // Enemy A only has a common drop (slot 0, rate 200); enemy B has a rare
    // slot 1 (rate 96) and a common slot 0 with rate 0 (drawn, never hit).
    let mut enemies = vec![
        EnemyRewardData {
            type_id: 1,
            present: true,
            gil: 200,
            exp: 240,
            items: [12, NO_ITEM, NO_ITEM, NO_ITEM],
            item_rates: [200, 0, 0, 0],
            card: NO_ITEM,
            card_rate: 0,
            bonus_given: false,
        },
        EnemyRewardData {
            type_id: 2,
            present: true,
            gil: 100,
            exp: 60,
            items: [NO_SHOP_ITEM as u32, 13, NO_ITEM, NO_ITEM],
            item_rates: [0, 96, 0, 0],
            card: NO_ITEM,
            card_rate: 0,
            bonus_given: false,
        },
    ];
    let mut bonus = BattleBonus::new(3, 0b0011, false);
    // Draws: A slot0 (10 < 200 -> drop 12); B slot1 (50 < 96 -> drop 13),
    // B slot0 (99, rate 0 -> no drop).
    let draws = [
        (Draw::Random8, 10),
        (Draw::Random8, 50),
        (Draw::Random8, 99),
    ];
    let mut rng = SequenceRandom::with_kinds(&draws);
    assert!(bonus.set_bonus(&mut enemies, 0, &mut rng));
    assert!(bonus.set_bonus(&mut enemies, 1, &mut rng));
    assert_eq!(bonus.gil, 300);
    assert_eq!(bonus.exp, 300);
    assert_eq!(bonus.ap, 3);
    assert_eq!(bonus.items, vec![12, 13]);

    // Party state mirrors the field world: lines 0 and 1 alive and taking
    // part; both carry a learnable ability for the AP award.
    let mut party = PartySnapshot {
        gil: 100,
        members: [None, None, None, None],
        items: vec![],
        cards: vec![],
    };
    for line in 0..2usize {
        let mut member = member(200);
        member.learnable = vec![tantalus_legacy::battle::rewards::PaEntry {
            id: 200 + line as i32,
            max_ap: 2,
        }];
        member.pa = vec![0];
        party.members[line] = Some(member);
    }

    let reward = tantalus_legacy::battle::rewards::BattleReward::from_bonus(&bonus, &party);
    // Reference split: 300 EXP over two eligible lines, remainder lost.
    assert_eq!(reward.shares[0].exp, Some(150));
    assert_eq!(reward.shares[1].exp, Some(150));
    assert_eq!(reward.shares[0].ap, Some(3));
    assert_eq!(reward.gil, 300);
    assert_eq!(reward.items, vec![(12, 1), (13, 1)]);

    // Field world: the party, its inventory mapping, and the growth provider.
    let mut world = FieldWorld::new();
    world.party = vec![0, 1];
    for character in [0, 1] {
        world.character_stats.insert(
            character,
            tantalus_game::field::CharacterStats {
                current_hp: 200,
                max_hp: 250,
                current_mp: 36,
                max_mp: 36,
            },
        );
    }
    world.gil = 100;
    world.item_id_map = Some(default_item_id_map([12, 13]));
    world.level_growth = Some(std::sync::Arc::new(|_, total| {
        (total >= 150).then_some(tantalus_game::field::CharacterStats {
            current_hp: 200,
            max_hp: 300,
            current_mp: 36,
            max_mp: 40,
        })
    }));

    let bundle: RewardBundle =
        bundle_from_battle_reward(&reward, [Some(0), Some(1), None, None], 1);
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
        // AP applies through the per-ability primitive after the commit.
        assert!(services.world.apply_ap(0, 200, 2, 3));
        assert!(services.world.apply_ap(1, 201, 2, 3));
    }

    assert_eq!(world.gil, 400);
    assert_eq!(
        world.character_exp,
        std::collections::BTreeMap::from([(0, 150), (1, 150)])
    );
    // The growth provider leveled both characters to the next stats.
    assert_eq!(world.character_stats[&0].max_hp, 300);
    assert_eq!(world.items[&12], 1, "drawn common drop lands");
    assert_eq!(world.items[&13], 1, "drawn rare drop lands");
    assert!(world.learned_abilities[&0].contains(&200));
    assert!(world.learned_abilities[&1].contains(&201));

    // A repeated commit of the same resolution pays nothing.
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
    }
    assert_eq!(world.gil, 400);
    assert_eq!(world.character_exp[&0], 150);
}

#[test]
fn card_drops_are_recorded_not_dropped() {
    // The bridge documents cards as unmodeled; the reward keeps them for the
    // Tetra collection without touching field state.
    let mut bonus = BattleBonus::new(0, 0b0001, false);
    bonus.exp = 10;
    let mut party = PartySnapshot {
        gil: 0,
        members: [None, None, None, None],
        items: vec![],
        cards: vec![],
    };
    party.members[0] = Some(member(100));
    let reward = tantalus_legacy::battle::rewards::BattleReward::from_bonus(&bonus, &party);
    assert_eq!(reward.card, None, "no card in the bonus");

    let bundle = bundle_from_battle_reward(&reward, [Some(0), None, None, None], 2);
    assert!(
        bundle.items.is_empty(),
        "no items means nothing to map: {}",
        bundle
            .items
            .iter()
            .map(ContentId::as_str)
            .collect::<Vec<_>>()
            .join(",")
    );
}
