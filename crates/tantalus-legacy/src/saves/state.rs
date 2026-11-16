//! Typed in-memory FF9 save state (milestone G8) and its conversion to and from the legacy save
//! tree ([`Ff9SaveTree`]).
//!
//! The state is device-free: no file, clock or random source. [`SaveState::to_tree`] and
//! [`SaveState::from_tree`] are exact inverses, so `from_tree(to_tree(s)) == s` and
//! `to_tree(from_tree(t)) == t` for every valid tree. Field meaning comes from pinned Memoria
//! (`JsonParser.cs`, `EventState.cs`, `FF9StateGlobal.cs`); `audit/save-tree-schema.md` lists the
//! line references and what is not source-backed.
//!
//! What the source does not save (so this state cannot carry it): the player's field position,
//! facing, vehicle or chocobo placement. `JsonParser.ParseFromFF9StateSystem` stores only
//! `fldMapNo`, `fldLocNo`, `wldMapNo`, `wldLocNo` and the world camera block; on load the engine
//! re-enters the field through the field entrance (`gEventGlobal` bytes 2-3). Do not add a position
//! here without a source for it.
//!
//! Sections the source defines but this state does not interpret are kept verbatim in
//! [`CarriedSections`] and listed by [`SaveState::unmodeled_report`]. Nothing is guessed.

use super::LegacySaveError;
use super::ff9_tree::{
    Achievement98000, AchievementSection, CardSlot, Common94000, CommonSection, EventGlobal,
    EventSection, Ff9SaveTree, KeyOrder, MiniGameSection, PlayerRecord, Resources, Setting95000,
    SettingSection, StateSection, WorldSection,
};
use sha2::{Digest, Sha256};

/// `10000_State.mode`, `prevMode` and the countdown timer (`JsonParser.cs:415-466`). The mode
/// numbers are raw: the enum is not mapped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModeBlock {
    pub mode: u8,
    pub prev_mode: u8,
    /// `TimerUI.Time`; the loader truncates it to whole seconds (`JsonParser.cs:408`).
    pub timer_seconds: f32,
    pub timer_control: bool,
    pub timer_display: bool,
}

/// Field and battle map ids (`10000_State`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldLocation {
    pub fld_map_no: i16,
    pub fld_loc_no: i16,
    pub btl_map_no: i16,
    pub btl_sub_map_no: i8,
}

/// World map location and camera block (`10000_State` ids and `70000_World`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldBlock {
    pub wld_map_no: i16,
    pub wld_loc_no: i16,
    /// `data.cameraState.*` and `data.hintmap`. The loader reads `hintmap` from `rotationRev`
    /// (`JsonParser.cs:1637`); the tree stores both, so both are kept.
    pub camera: WorldSection,
}

/// One `info` block of a player (`JsonParser.cs:1022-1098`); all raw bytes, enums not mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CharacterInfo {
    pub slot_no: u8,
    pub serial_no: u8,
    pub row: u8,
    pub win_pose: u8,
    pub party: u8,
    pub menu_type: u8,
}

/// Base stats and stat bonuses, kept as named raw values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CharacterBasis {
    pub max_hp: i16,
    pub max_mp: i16,
    /// dex, str, mgc, wpr.
    pub stats: [u8; 4],
    /// `bonus`: dex, str, mgc, wpr.
    pub bonus: [u16; 4],
    /// `elem`: dex, str, mgc, wpr.
    pub elem: [u8; 4],
    /// `defence`: p_def, p_ev, m_def, m_ev.
    pub defence: [u8; 4],
}

/// One of the nine saved characters, in legacy slot order (`SelectOldSaveSlot`; the mapping from
/// character ids to these slots is not implemented).
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterState {
    pub name: String,
    pub category: u8,
    pub level: u8,
    pub exp: u32,
    /// Current HP, MP and ATB fields.
    pub cur: Resources,
    /// Maximum HP, MP and ATB fields.
    pub max: Resources,
    /// Equipment item ids, five slots; slot names are not mapped.
    pub equip: [u8; 5],
    /// Status byte as stored (the writer casts the full status to Byte).
    pub status: u8,
    pub trance: u8,
    /// Saved under the key `web_bone` (the loader reads `wep_bone`, so it never loads back).
    pub web_bone: u8,
    /// `pa`: 48 ability progress bytes.
    pub ability_ap: Vec<u8>,
    /// `sa`: two supporting-ability bit fields.
    pub support_abilities: [u32; 2],
    pub info: CharacterInfo,
    pub basis: CharacterBasis,
    /// `94000_Common.00001_player_bonus` for this slot (`bonus.cap`).
    pub bonus_cap: u32,
}

/// One inventory stack (`items` entry: `id` and `count` Bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStack {
    pub id: u8,
    pub count: u8,
}

/// Party, characters, inventory and money (`40000_Common`).
#[derive(Debug, Clone, PartialEq)]
pub struct PartyState {
    /// Four party member slots; `None` is the writer's 255.
    pub members: [Option<u8>; 4],
    /// Nine characters in legacy slot order.
    pub characters: Vec<CharacterState>,
    pub gil: u32,
    /// 256 inventory slots; `None` is the writer's empty marker (id 255, count 0).
    pub items: Vec<Option<ItemStack>>,
    /// 64 bytes of rare-item flags (bit 2k obtained, 2k+1 used for ids 4i..4i+3).
    pub rare_item_flags: Vec<u8>,
    pub escape_no: u16,
    pub summon_flag: u16,
    pub frog_no: i16,
    pub steal_no: i16,
    pub dragon_no: i16,
}

/// Tetra Master card collection and record (`30000_MiniGame`).
#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    pub win: i16,
    pub lose: i16,
    pub draw: i16,
    /// 100 slots; `None` is the all-255 empty slot.
    pub cards: Vec<Option<CardSlot>>,
}

/// Sections the source defines but this state does not interpret. They are copied verbatim so a
/// save and load never drops them.
#[derive(Debug, Clone, PartialEq)]
pub struct CarriedSections {
    pub setting: SettingSection,
    pub achievement: AchievementSection,
    pub reserved_91000: Vec<i32>,
    pub reserved_92000: Vec<i32>,
    pub reserved_93000: Vec<i32>,
    pub reserved_94000: Vec<i32>,
    pub reserved_95000: Vec<i32>,
    pub sound_96000: Vec<i32>,
    pub world_97000: Vec<i32>,
    pub achievement_98000: Achievement98000,
    pub other_99000: Vec<i32>,
}

/// A named group of bytes the state keeps but does not interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmodeledEntry {
    pub name: &'static str,
    pub bytes: usize,
    pub reason: &'static str,
}

/// The whole typed save state.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveState {
    pub mode: ModeBlock,
    pub field: FieldLocation,
    pub world: WorldBlock,
    /// The 2048-byte `gEventGlobal` global variable array. Scenario counter and field entrance
    /// (bytes 0-3) live here and are read through [`EventGlobal`] accessors.
    pub variables: EventGlobal,
    /// `gStepCount`.
    pub step_count: i32,
    pub party: PartyState,
    pub cards: CardState,
    /// `95000_Setting.00001_time` (`FF9StateSystem.Settings.time`, seconds).
    pub play_time_seconds: f64,
    pub auto_save_bgm_id: i32,
    pub carried: CarriedSections,
}

impl SaveState {
    /// State of an all-zero tree (no party, empty inventory and card slots).
    pub fn empty() -> Self {
        Self::from_tree(&Ff9SaveTree::blank())
    }

    /// Field entrance from `gEventGlobal` bytes 2-3.
    pub fn field_entrance(&self) -> u16 {
        self.variables.field_entrance()
    }

    /// Scenario counter from `gEventGlobal` bytes 0-1.
    pub fn scenario_counter(&self) -> u16 {
        self.variables.scenario_counter()
    }

    /// Builds the state from a decoded tree.
    pub fn from_tree(tree: &Ff9SaveTree) -> Self {
        let st = &tree.state;
        let c = &tree.common;
        let bonus_caps = tree.common_94000.player_bonus;
        Self {
            mode: ModeBlock {
                mode: st.mode,
                prev_mode: st.prev_mode,
                timer_seconds: st.time_counter,
                timer_control: st.timer_control,
                timer_display: st.timer_display,
            },
            field: FieldLocation {
                fld_map_no: st.fld_map_no,
                fld_loc_no: st.fld_loc_no,
                btl_map_no: st.btl_map_no,
                btl_sub_map_no: st.btl_sub_map_no,
            },
            world: WorldBlock {
                wld_map_no: st.wld_map_no,
                wld_loc_no: st.wld_loc_no,
                camera: tree.world.clone(),
            },
            variables: tree.event.event_global.clone(),
            step_count: tree.event.step_count,
            party: PartyState {
                members: c.slot.map(|s| (s != 255).then_some(s)),
                characters: c
                    .players
                    .iter()
                    .zip(bonus_caps)
                    .map(|(p, cap)| CharacterState {
                        name: p.name.clone(),
                        category: p.category,
                        level: p.level,
                        exp: p.exp,
                        cur: p.cur,
                        max: p.max,
                        equip: p.equip,
                        status: p.status,
                        trance: p.trance,
                        web_bone: p.web_bone,
                        ability_ap: p.pa.clone(),
                        support_abilities: p.sa,
                        info: CharacterInfo {
                            slot_no: p.info[0],
                            serial_no: p.info[1],
                            row: p.info[2],
                            win_pose: p.info[3],
                            party: p.info[4],
                            menu_type: p.info[5],
                        },
                        basis: CharacterBasis {
                            max_hp: p.basis_max_hp,
                            max_mp: p.basis_max_mp,
                            stats: p.basis_stats,
                            bonus: p.bonus,
                            elem: p.elem,
                            defence: p.defence,
                        },
                        bonus_cap: cap,
                    })
                    .collect(),
                gil: c.gil,
                items: c
                    .items
                    .iter()
                    .map(|&(id, count)| {
                        (!(id == 255 && count == 0)).then_some(ItemStack { id, count })
                    })
                    .collect(),
                rare_item_flags: c.rare_items.clone(),
                escape_no: c.escape_no,
                summon_flag: c.summon_flag,
                frog_no: c.frog_no,
                steal_no: c.steal_no,
                dragon_no: c.dragon_no,
            },
            cards: CardState {
                win: tree.mini_game.s_win,
                lose: tree.mini_game.s_lose,
                draw: tree.mini_game.s_draw,
                cards: tree
                    .mini_game
                    .cards
                    .iter()
                    .map(|&c| (c != CardSlot::EMPTY).then_some(c))
                    .collect(),
            },
            play_time_seconds: tree.setting_95000.time,
            auto_save_bgm_id: tree.auto_save_bgm_id,
            carried: CarriedSections {
                setting: tree.setting.clone(),
                achievement: tree.achievement.clone(),
                reserved_91000: tree.reserved_91000.clone(),
                reserved_92000: tree.reserved_92000.clone(),
                reserved_93000: tree.reserved_93000.clone(),
                reserved_94000: tree.common_94000.reserved.clone(),
                reserved_95000: tree.setting_95000.reserved.clone(),
                sound_96000: tree.sound_96000.clone(),
                world_97000: tree.world_97000.clone(),
                achievement_98000: tree.achievement_98000.clone(),
                other_99000: tree.other_99000.clone(),
            },
        }
    }

    /// Builds the tree to encode. Fails when a collection has the wrong length.
    pub fn to_tree(&self) -> Result<Ff9SaveTree, LegacySaveError> {
        let p = &self.party;
        if p.characters.len() != CommonSection::PLAYERS {
            return Err(LegacySaveError::new(
                0,
                "party.characters must hold 9 entries",
            ));
        }
        if p.items.len() != CommonSection::ITEMS {
            return Err(LegacySaveError::new(0, "party.items must hold 256 slots"));
        }
        if p.rare_item_flags.len() != CommonSection::RARE_ITEMS {
            return Err(LegacySaveError::new(
                0,
                "party.rare_item_flags must hold 64 bytes",
            ));
        }
        if self.cards.cards.len() != MiniGameSection::CARDS {
            return Err(LegacySaveError::new(0, "cards.cards must hold 100 slots"));
        }
        let mut tree = Ff9SaveTree::blank();
        tree.state = StateSection {
            mode: self.mode.mode,
            prev_mode: self.mode.prev_mode,
            fld_map_no: self.field.fld_map_no,
            fld_loc_no: self.field.fld_loc_no,
            btl_map_no: self.field.btl_map_no,
            btl_sub_map_no: self.field.btl_sub_map_no,
            wld_map_no: self.world.wld_map_no,
            wld_loc_no: self.world.wld_loc_no,
            time_counter: self.mode.timer_seconds,
            timer_control: self.mode.timer_control,
            timer_display: self.mode.timer_display,
        };
        tree.event = EventSection {
            step_count: self.step_count,
            event_global: self.variables.clone(),
        };
        let mut caps = [0u32; 9];
        let mut players = Vec::with_capacity(CommonSection::PLAYERS);
        for (i, ch) in p.characters.iter().enumerate() {
            caps[i] = ch.bonus_cap;
            players.push(PlayerRecord {
                name: ch.name.clone(),
                category: ch.category,
                level: ch.level,
                exp: ch.exp,
                cur: ch.cur,
                max: ch.max,
                trance: ch.trance,
                web_bone: ch.web_bone,
                elem: ch.basis.elem,
                defence: ch.basis.defence,
                basis_max_hp: ch.basis.max_hp,
                basis_max_mp: ch.basis.max_mp,
                basis_stats: ch.basis.stats,
                info: [
                    ch.info.slot_no,
                    ch.info.serial_no,
                    ch.info.row,
                    ch.info.win_pose,
                    ch.info.party,
                    ch.info.menu_type,
                ],
                status: ch.status,
                equip: ch.equip,
                bonus: ch.basis.bonus,
                pa: ch.ability_ap.clone(),
                sa: ch.support_abilities,
            });
        }
        tree.common = CommonSection {
            players,
            slot: p.members.map(|m| m.unwrap_or(255)),
            escape_no: p.escape_no,
            summon_flag: p.summon_flag,
            gil: p.gil,
            frog_no: p.frog_no,
            steal_no: p.steal_no,
            dragon_no: p.dragon_no,
            items: p
                .items
                .iter()
                .map(|s| s.map_or((255, 0), |s| (s.id, s.count)))
                .collect(),
            rare_items: p.rare_item_flags.clone(),
        };
        tree.mini_game = MiniGameSection {
            s_win: self.cards.win,
            s_lose: self.cards.lose,
            s_draw: self.cards.draw,
            cards: self
                .cards
                .cards
                .iter()
                .map(|c| c.unwrap_or(CardSlot::EMPTY))
                .collect(),
        };
        let k = &self.carried;
        tree.setting = k.setting.clone();
        tree.auto_save_bgm_id = self.auto_save_bgm_id;
        tree.world = self.world.camera.clone();
        tree.achievement = k.achievement.clone();
        tree.reserved_91000 = k.reserved_91000.clone();
        tree.reserved_92000 = k.reserved_92000.clone();
        tree.reserved_93000 = k.reserved_93000.clone();
        tree.common_94000 = Common94000 {
            player_bonus: caps,
            reserved: k.reserved_94000.clone(),
        };
        tree.setting_95000 = Setting95000 {
            time: self.play_time_seconds,
            reserved: k.reserved_95000.clone(),
        };
        tree.sound_96000 = k.sound_96000.clone();
        tree.world_97000 = k.world_97000.clone();
        tree.achievement_98000 = k.achievement_98000.clone();
        tree.other_99000 = k.other_99000.clone();
        Ok(tree)
    }

    /// Flattens to the `SAVE` payload bytes (without the 4-byte magic), ordinal key order.
    pub fn encode(&self) -> Result<Vec<u8>, LegacySaveError> {
        self.to_tree()?.encode_payload(KeyOrder::Ordinal)
    }

    /// Inverse of [`Self::encode`].
    pub fn decode(payload: &[u8]) -> Result<Self, LegacySaveError> {
        Ok(Self::from_tree(&Ff9SaveTree::decode_payload(
            payload,
            KeyOrder::Ordinal,
        )?))
    }

    /// SHA-256 of the canonical payload bytes; equal states hash equal and the hash survives a
    /// save and load.
    pub fn state_hash(&self) -> Result<[u8; 32], LegacySaveError> {
        Ok(Sha256::digest(self.encode()?).into())
    }

    /// Bytes the state keeps verbatim without interpreting, with the reason. Sizes follow the
    /// base-tree schema (`audit/save-tree-schema.md`).
    pub fn unmodeled_report(&self) -> Vec<UnmodeledEntry> {
        let k = &self.carried;
        let ints = |v: &Vec<i32>| v.len() * 4;
        let entry = |name, bytes, reason| UnmodeledEntry {
            name,
            bytes,
            reason,
        };
        vec![
            entry("50000_Setting", 1444, "configuration; not game progress"),
            entry(
                "80000_Achievement",
                2848,
                "achievement counters and unlock lists; meaning not interpreted",
            ),
            entry(
                "91000_State",
                ints(&k.reserved_91000),
                "reserved, zeros in the writer; original contents unknown",
            ),
            entry(
                "92000_Event",
                ints(&k.reserved_92000),
                "reserved, zeros in the writer; original contents unknown",
            ),
            entry(
                "93000_MiniGame",
                ints(&k.reserved_93000),
                "reserved, zeros in the writer; original contents unknown",
            ),
            entry(
                "94000_Common.99999_ReservedBuffer",
                ints(&k.reserved_94000),
                "reserved",
            ),
            entry(
                "95000_Setting.99999_ReservedBuffer",
                ints(&k.reserved_95000),
                "reserved",
            ),
            entry("96000_Sound", ints(&k.sound_96000), "reserved"),
            entry("97000_World", ints(&k.world_97000), "reserved"),
            entry(
                "98000_Achievement",
                512,
                "abnormal-status and summon flags; meaning not interpreted",
            ),
            entry("99000_Other", ints(&k.other_99000), "reserved"),
            entry(
                "20000_Event.gEventGlobal bytes 4-2047",
                2044,
                "event-script globals; meaning lives in field scripts, kept as the raw variable array",
            ),
        ]
    }
}
