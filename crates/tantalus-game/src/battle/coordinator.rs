//! Encounter coordination: field-queued battle requests drive one active
//! battle at a time, and battle rewards commit into field world state
//! exactly once.

use super::{AnyRuleset, BattleOutcome, CombatInput, GameServices, RewardBundle};
use crate::field::world::{EncounterRequest, FieldWorld, MAX_GIL};
use tantalus_contracts::{ContentId, EngineError, ErrorCategory};

#[derive(Debug)]
pub struct ActiveEncounter {
    pub request: EncounterRequest,
    pub battle: AnyRuleset,
}

#[derive(Debug, Default)]
pub struct EncounterCoordinator {
    active: Option<ActiveEncounter>,
}

impl EncounterCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn active_request(&self) -> Option<&EncounterRequest> {
        self.active.as_ref().map(|encounter| &encounter.request)
    }

    /// The active ruleset, when a battle is running.
    pub fn active_ruleset(&self) -> Option<&AnyRuleset> {
        self.active.as_ref().map(|encounter| &encounter.battle)
    }

    /// Mutable access to the active ruleset (event drains, state probes).
    pub fn active_ruleset_mut(&mut self) -> Option<&mut AnyRuleset> {
        self.active.as_mut().map(|encounter| &mut encounter.battle)
    }

    /// Begin a battle for one encounter request. A second begin while a
    /// battle is active is a conflict; the caller must finish first.
    pub fn begin(
        &mut self,
        request: EncounterRequest,
        battle: impl Into<AnyRuleset>,
    ) -> Result<(), EngineError> {
        if self.active.is_some() {
            return Err(EngineError::new(
                ErrorCategory::Conflict,
                "an encounter is already active",
                "tantalus-game/encounter",
                true,
            ));
        }
        self.active = Some(ActiveEncounter {
            request,
            battle: battle.into(),
        });
        Ok(())
    }

    /// Advance the active battle. Returns the outcome once the battle ends;
    /// the encounter stays active until [`Self::finish`] so rewards and
    /// field-return handling complete before the next battle can begin.
    pub fn advance(
        &mut self,
        input: &CombatInput,
        services: &mut dyn GameServices,
    ) -> Result<Option<BattleOutcome>, EngineError> {
        let Some(encounter) = &mut self.active else {
            return Err(EngineError::new(
                ErrorCategory::UnknownContent,
                "no active encounter to advance",
                "tantalus-game/encounter",
                true,
            ));
        };
        let update = encounter.battle.advance(input, services)?;
        Ok(update.outcome)
    }

    /// End the active encounter after outcome handling and return its request
    /// for field-return bookkeeping.
    pub fn finish(&mut self) -> Option<EncounterRequest> {
        self.active.take().map(|encounter| encounter.request)
    }
}

/// `GameServices` bridge from the battle rules into field world state.
///
/// Randomness comes from the world's deterministic generator. Reward commits
/// are exactly-once per nonzero `resolution_id`: the first commit applies gil
/// (clamped like the reference) and records the bundle; a duplicate commit is
/// accepted and ignored so retries cannot double-apply.
/// Maps a reward item's content identity to the legacy inventory item id
/// used by field scripts. Runtime behavior, not world state.
pub type ItemIdMap = crate::field::world::ItemIdMap;

#[derive(Debug)]
pub struct WorldGameServices<'a> {
    pub world: &'a mut FieldWorld,
    pub committed: Vec<RewardBundle>,
    /// Items granted but not applied because no id mapping was available.
    pub unmapped_items: Vec<ContentId>,
}

impl<'a> WorldGameServices<'a> {
    pub fn new(world: &'a mut FieldWorld) -> Self {
        Self {
            world,
            committed: Vec::new(),
            unmapped_items: Vec::new(),
        }
    }

    /// Apply a reward's items to the world inventory through the legacy id
    /// mapping. Returns the number of items applied; unmapped items are
    /// recorded on `unmapped_items` for diagnostics.
    pub fn apply_reward_items(&mut self, bundle: &RewardBundle) -> usize {
        let Some(map) = self.world.item_id_map.clone() else {
            self.unmapped_items.extend(bundle.items.iter().cloned());
            return 0;
        };
        let mut applied = 0;
        for item in &bundle.items {
            match map(item) {
                Some(legacy_id) => {
                    self.world.add_item_amount(legacy_id, 1);
                    applied += 1;
                }
                None => self.unmapped_items.push(item.clone()),
            }
        }
        applied
    }
}

/// `BattleResultUI.AnalyzeArgument` EXP split (`BattleResultUI.cs:514`):
/// each eligible party line receives `total / eligible_count`, integer
/// division, remainder lost; no eligible line means no shares. Wrap the
/// result in `Some` when building a [`RewardBundle`] so a battle that paid
/// nobody does not fall back to the full-grant path.
pub fn split_experience(
    total: u64,
    party: &[i32],
    eligible: &dyn Fn(i32) -> bool,
) -> std::collections::BTreeMap<i32, u64> {
    let eligible_members: Vec<i32> = party.iter().copied().filter(|&m| eligible(m)).collect();
    if eligible_members.is_empty() {
        return std::collections::BTreeMap::new();
    }
    let share = total / eligible_members.len() as u64;
    eligible_members
        .into_iter()
        .map(|member| (member, share))
        .collect()
}

impl GameServices for WorldGameServices<'_> {
    fn random_below(&mut self, upper: u32) -> u32 {
        if upper <= 1 {
            return 0;
        }
        if upper <= 256 {
            return u32::from(self.world.next_random_u8()) % upper;
        }
        // Wider draws compose bytes from the world RNG, matching the
        // reference's draw width: `random16()` asks for 65_536 and gets a
        // full 16-bit value (exact, a power of two) instead of one u8 %
        // 65536. Powers of two are exact; other uppers keep the small
        // modulo bias the reference's own `%` has.
        let bits = 32 - upper.leading_zeros();
        let bytes = bits.div_ceil(8);
        let mut value = 0u32;
        for _ in 0..bytes {
            value = (value << 8) | u32::from(self.world.next_random_u8());
        }
        value % upper
    }

    fn commit_rewards(&mut self, rewards: &RewardBundle) -> Result<(), EngineError> {
        if rewards.resolution_id != 0 && !self.world.apply_resolution(rewards.resolution_id) {
            // Duplicate resolution: accept without re-applying.
            return Ok(());
        }
        self.world
            .add_gil(rewards.gil.min(u64::from(MAX_GIL)) as u32);
        self.apply_reward_items(rewards);
        match &rewards.shares {
            // The battle engine distributed experience per character.
            Some(shares) => {
                for (member, experience) in shares {
                    self.world.apply_experience(*member, *experience);
                }
            }
            // v1 fallback: full grant to every party member.
            None => {
                for member in self.world.party.clone() {
                    self.world.apply_experience(member, rewards.experience);
                }
            }
        }
        self.committed.push(rewards.clone());
        Ok(())
    }
}
