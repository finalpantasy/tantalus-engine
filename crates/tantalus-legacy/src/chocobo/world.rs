//! World map chocobo predicates, the Gysahl Greens gate, and the Chocobo Hot
//! and Cold HUD switches (`Global/Event/EventCollision.cs`, `Global/ItemUI.cs`,
//! `Global/ff9/ff9.cs`, `Global/Event/EventHUD.cs`, `Global/EMinigame.cs`,
//! `Global/ETb/ETb.cs`). Hot and Cold itself (distance bands, scoring, the
//! dig) is not C#: only the HUD that surrounds it is.

use crate::saves::ff9_tree::EventGlobal;

/// `Obj.OBJINDEX_CHOCOBO_*` (`Global/Objects/Obj.cs:491-495`) with the texture
/// set names from `WMBeeChocobo.SetType` (`WM/WMBee/WMBeeChocobo.cs:26-52`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChocoboKind {
    /// Index 3, "Normal" materials.
    Yellow,
    /// Index 4, "Asase" materials.
    Teal,
    /// Index 5, "Yama" materials.
    Red,
    /// Index 6, "Umi" materials.
    Blue,
    /// Index 7, "Sora" materials.
    Gold,
}

impl ChocoboKind {
    pub fn from_obj_index(index: u8) -> Option<Self> {
        Some(match index {
            3 => Self::Yellow,
            4 => Self::Teal,
            5 => Self::Red,
            6 => Self::Blue,
            7 => Self::Gold,
            _ => return None,
        })
    }

    pub fn obj_index(self) -> u8 {
        match self {
            Self::Yellow => 3,
            Self::Teal => 4,
            Self::Red => 5,
            Self::Blue => 6,
            Self::Gold => 7,
        }
    }
}

/// `EventCollision.IsRidingChocobo` (`EventCollision.cs:287-290`): transport
/// number 1..=6.
pub fn is_riding_chocobo(control_no: i32) -> bool {
    (1..=6).contains(&control_no)
}

fn in_forest_topograph(topograph: i32) -> bool {
    (36..=38).contains(&topograph)
}

/// `IsChocoboWalkingOrFlyingInForestArea` (`EventCollision.cs:277-280`);
/// `status_no` is `WMUIData.StatusNo`, `topograph` is `ff9.m_GetIDTopograph`.
pub fn is_walking_or_flying_in_forest(control_no: i32, status_no: u8, topograph: i32) -> bool {
    (control_no == 5 || control_no == 6) && status_no == 7 && in_forest_topograph(topograph)
}

/// `IsChocoboWalkingInForestArea` (`EventCollision.cs:282-285`).
pub fn is_walking_in_forest(control_no: i32, topograph: i32) -> bool {
    control_no == 5 && in_forest_topograph(topograph)
}

/// `ff9.w_frameChocoboCheck` (`ff9.cs:4353-4370`): the player character
/// stands on a chocobo track topograph (3, 18, 21, 22 or 28).
pub fn chocobo_check(is_player_character: bool, topograph: i32) -> bool {
    is_player_character && matches!(topograph, 3 | 18 | 21 | 22 | 28)
}

/// `ItemUI.FF9FItem_Vegetable` (`ItemUI.cs:958-969`), the state part. Gysahl
/// Greens work only in world mode (`mode == 3`), with a chocobo kind set
/// (byte 191 not 0), byte 181 clear, and on a chocobo track. On success byte
/// 181 becomes 1 and the call returns true. The dialog, 1.5 s wait and fade
/// are UI. What script reads byte 181 afterwards is a field script, not
/// modeled.
pub fn use_gysahl_greens(
    mode: u8,
    global: &mut EventGlobal,
    is_player_character: bool,
    topograph: i32,
) -> bool {
    let bytes = global.as_bytes();
    if mode != 3
        || bytes[191] == 0
        || bytes[181] != 0
        || !chocobo_check(is_player_character, topograph)
    {
        return false;
    }
    global.as_bytes_mut()[181] = 1;
    true
}

/// `MinigameHUD` (`Global/MinigameHUD.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinigameHud {
    None,
    Chanbara,
    Auction,
    MogTutorial,
    JumpingRope,
    Telescope,
    RacingHippaul,
    SwingACage,
    GetTheKey,
    ChocoHot,
    ChocoHotInstruction,
    PandoniumElevator,
}

/// What `EventHUD` does each frame in the chocobo forest fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForestHud {
    /// `CloseSpecialHUD(ChocoHotInstruction)` then `OpenSpecialHUD(ChocoHot)`.
    OpenHot,
    /// `CloseSpecialHUD(ChocoHot)`.
    CloseHot,
    Nothing,
}

/// `EventHUD.cs:384-395`: fields 2950 (Forest), 2951 (Lagoon), 2952 (Air
/// Garden). `TimerUI.Play && Time > 0` opens the Hot and Cold HUD, a timer
/// that is merely enabled closes it.
pub fn forest_hud(
    fld_map_no: i32,
    timer_play: bool,
    timer_time: f32,
    timer_enable: bool,
) -> ForestHud {
    if !matches!(fld_map_no, 2950..=2952) {
        return ForestHud::Nothing;
    }
    if timer_play && timer_time > 0.0 {
        ForestHud::OpenHot
    } else if timer_enable {
        ForestHud::CloseHot
    } else {
        ForestHud::Nothing
    }
}

/// `EventHUD.cs:183-189`: in field zone 945 ("Chocobo Places") messages 34
/// ("How to play Chocobo Hot & Cold") and 35 ("Choco's Cries") open the
/// instruction HUD. Other messages in this zone pick the Auction HUD by
/// language (not ported).
pub fn special_hud_for_message(field_zone_id: i32, mes_id: i32) -> Option<MinigameHud> {
    (field_zone_id == 945 && matches!(mes_id, 34 | 35)).then_some(MinigameHud::ChocoHotInstruction)
}

/// `EMinigame.ChocoboBeakLV99Achievement` (`EMinigame.cs:261-285`): in fields
/// 2950-2952, "Choco's beak became stronger!" or "Time's Up" reports the
/// beak level (Global Byte 139) to the Lv99 achievement. Message ids per
/// language symbol: JP 325 or 277, US/UK 320 or 277, any other 326 or 278.
/// Returns the value reported, `None` when the check does not fire.
pub fn beak_achievement_level(
    fld_map_no: i32,
    lang_symbol: &str,
    mes_id: i32,
    global: &EventGlobal,
) -> Option<u8> {
    if !matches!(fld_map_no, 2950..=2952) {
        return None;
    }
    let fires = match lang_symbol {
        "JP" => mes_id == 325 || mes_id == 277,
        "US" | "UK" => mes_id == 320 || mes_id == 277,
        _ => mes_id == 326 || mes_id == 278,
    };
    fires.then(|| global.as_bytes()[139])
}

/// `ETb.FixChocoAccidenlyFly` (`ETb.cs:458-462`): on the world map, while
/// walking in the forest, dialogs 54..=62 turn the dialog collider off.
pub fn forest_dialog_disables_collider(
    g_mode: u8,
    walking_in_forest: bool,
    dialog_text_id: i32,
) -> bool {
    g_mode == 3 && walking_in_forest && (54..=62).contains(&dialog_text_id)
}

/// `EMinigame.SetFatChocoboId` (`EMinigame.cs:362-366`): in Chocobo's
/// Paradise (field 2955) the object with uid 9 is registered as the Tetra
/// Master opponent.
pub fn fat_chocobo_opponent(fld_map_no: i32, uid: u8) -> bool {
    fld_map_no == 2955 && uid == 9
}
