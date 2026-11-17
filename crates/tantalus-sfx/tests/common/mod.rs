//! Shared by the DLL comparison tests.
#![allow(dead_code)]

use tantalus_sfx::oracle::{INIT_PARAM_SIZE, REQUEST_SIZE};

pub fn install() -> std::path::PathBuf {
    std::env::var_os("TANTALUS_FF9").map_or_else(
        || std::path::PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"),
        std::path::PathBuf::from,
    )
}

/// `SpecialEffects/ef<id>` from `resources.assets`.
pub fn effect_bytes(id: i32) -> Vec<u8> {
    use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
    let mut file = StandaloneSerialized::open(&install().join("x64/FF9_Data/resources.assets"))
        .expect("assets");
    let name = format!("ef{id:03}");
    // The short effects (Player_Attack 100-118, SFX_Attack 313 and on) have
    // no package: the game passes a null buffer (`SFX.PlaySFX`).
    let Some((_, object)) = file
        .text_assets()
        .expect("text assets")
        .into_iter()
        .find(|(asset, _)| *asset == name)
    else {
        return Vec::new();
    };
    file.read_text_asset(&object).expect("read").script
}

/// Zidane (btl_id 1, serial 0) against one enemy (btl_id 0x10), as
/// `SFX.InitBattleParty` fills it.
pub fn init_param() -> [u8; INIT_PARAM_SIZE] {
    let mut param = [0u8; INIT_PARAM_SIZE];
    for (slot, (player, btl_id)) in [(0usize, (1u8, 1u16)), (4, (0, 0x10))] {
        let at = 8 + slot * tantalus_sfx::BTL_DATA_INIT_SIZE;
        param[at] = player;
        param[at + 1] = slot as u8 & 3;
        param[at + 8..at + 12].copy_from_slice(&200i32.to_le_bytes());
        param[at + 12..at + 16].copy_from_slice(&200i32.to_le_bytes());
        param[at + 16..at + 20].copy_from_slice(&400i32.to_le_bytes());
        param[at + 20..at + 22].copy_from_slice(&btl_id.to_le_bytes());
    }
    // Identity camera.
    let cam = 8 + 8 * tantalus_sfx::BTL_DATA_INIT_SIZE;
    for i in [0usize, 4, 8] {
        param[cam + 2 * i..cam + 2 * i + 2].copy_from_slice(&4096i16.to_le_bytes());
    }
    param
}

/// Caster 1 on target 0x10.
pub fn request() -> [u8; REQUEST_SIZE] {
    let mut request = [0u8; REQUEST_SIZE];
    request[0..2].copy_from_slice(&1u16.to_le_bytes());
    request[4..6].copy_from_slice(&1u16.to_le_bytes());
    request[8..10].copy_from_slice(&0x10u16.to_le_bytes());
    request[56] = 1;
    request
}

pub fn dll() -> std::path::PathBuf {
    install().join("x64/FF9_Data/Plugins/FF9SpecialEffectPlugin.dll")
}
