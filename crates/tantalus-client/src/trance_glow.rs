//! The Trance glow on a party member's battle model, after Memoria's
//! `btl_stat.SetStatusVfx` (status glow kind 2) and
//! `btl_stat.GeoAddColor2DrawPacket`, drawn by `PSX/BattleMap_StatusEffect`.
//!
//! While a living member is in Trance the model's `_Color` pulses with the
//! battle counter: `strength` rises 0 to 8 and falls back every 16 counts,
//! and `_Color = chr + max(0, (chr + glow - 128) * strength >> 3)` per
//! channel (`chr` is the background's `BBGINFO.chr_*`, `glow` the
//! character's `TranceGlowingColor`), capped at 255. The shader draws
//! `texel + _Color - 0.5` (per channel, clamped), so mid gray is no
//! change and the glow lifts the texel toward the character's color.

use tantalus_tools::battle_stage::BATTLE_TICKS_PER_SECOND;

/// `ff9Battle.btl_cnt` at `seconds` into the battle.
pub fn battle_counter(seconds: f64) -> u32 {
    (seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as u32
}

/// The glow strength for a battle counter: 0 to 8 and back every 16.
pub fn strength(counter: u32) -> i32 {
    let counter = (counter % 16) as i32;
    if counter >= 8 { 16 - counter } else { counter }
}

/// The `_Color` bytes (`SetStatusVfx` then `GeoAddColor2DrawPacket`) for
/// background character color `chr`, glow color `glow` at `counter`.
pub fn glow_color(chr: [u8; 3], glow: [u8; 3], counter: u32) -> [u8; 3] {
    let strength = strength(counter);
    [0, 1, 2].map(|channel| {
        let add = ((i32::from(chr[channel]) + i32::from(glow[channel]) - 128) * strength) >> 3;
        (i32::from(chr[channel]) + add.max(0)).min(255) as u8
    })
}

/// `rgba` as `BattleMap_StatusEffect` draws it: each color channel plus
/// `_Color - 0.5`, clamped; alpha is kept.
pub fn tint(rgba: &[u8], color: [u8; 3]) -> Vec<u8> {
    let offset = color.map(|value| f32::from(value) - 127.5);
    let mut out = rgba.to_vec();
    for texel in out.chunks_exact_mut(4) {
        for channel in 0..3 {
            texel[channel] = (f32::from(texel[channel]) + offset[channel])
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_pulses_up_and_down_every_sixteen_counts() {
        let run: Vec<i32> = (0..17).map(strength).collect();
        assert_eq!(run, [0, 1, 2, 3, 4, 5, 6, 7, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(strength(16 * 5 + 3), 3);
    }

    #[test]
    fn the_glow_color_adds_the_scaled_glow_to_the_background_color() {
        // Zidane's (255, 96, 96) over a neutral 128 background.
        assert_eq!(glow_color([128; 3], [255, 96, 96], 0), [128; 3]);
        assert_eq!(glow_color([128; 3], [255, 96, 96], 8), [255, 224, 224]);
        // Halfway: 4/8 of the added color.
        assert_eq!(glow_color([128; 3], [255, 96, 96], 4), [255, 176, 176]);
        // A glow darker than 128 over a dark background adds nothing.
        assert_eq!(glow_color([10; 3], [100, 100, 100], 8), [10; 3]);
    }

    #[test]
    fn the_shader_adds_color_minus_half_to_the_texel() {
        let out = tint(&[100, 100, 100, 77, 250, 0, 40, 255], [128, 228, 128]);
        // 128 adds 0.5 and 228 adds 100.5; both round up.
        assert_eq!(out, [101, 201, 101, 77, 251, 101, 41, 255]);
    }
}
