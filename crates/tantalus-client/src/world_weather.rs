//! Sky and distance fog of the overworld (`ff9.w_nwpChangeStat`,
//! `WorldConfiguration.GetWeatherLight`).
//!
//! The renderer fades the far ground to the fog color (`RenderSettings.fogColor`
//! is the table's `fogDW`) and clears the camera to that same color; the sky
//! dome runs from `skyBgColor` overhead down to the fog color at the horizon.
//! Which of the four weather rows applies depends on where the player stands:
//! daylight everywhere, evening and night around Treno, evening over the
//! south of the Forgotten Continent.

use crate::battle_raster::Frame;

/// Where the haze starts and where it is full, in world units from the
/// camera. `ff9.w_weatherModeCheck_ChangeSettings` puts the radial mist at
/// `100 * MistStartDistance_base` (80 by default) plus the camera-height
/// blend of 64 to 80 past the camera, about 145 units, so the ground and the
/// mountains within a block or two of the player stay clear and the far
/// ones fade. (`WMTweaker._FogStartDistance` 47.9 and 59 only feed the actor
/// shaders.)
pub const FOG_START: f32 = 140.0;
pub const FOG_END: f32 = 250.0;

/// One row of `WeatherColors.csv` after the weather change has finished
/// (`Color[2]`, `[6]`, `[10]`, `[14]`): daylight, evening, night, purple sky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weather {
    /// `fogDW`: the fog and the horizon.
    pub fog: [u8; 3],
    /// `skyBgColor` scaled to bytes: the sky overhead.
    pub sky: [u8; 3],
}

pub const DAYLIGHT: Weather = Weather {
    fog: [205, 217, 247],
    sky: [124, 179, 217],
};
pub const EVENING: Weather = Weather {
    fog: [245, 181, 90],
    sky: [187, 149, 100],
};
pub const NIGHT: Weather = Weather {
    fog: [80, 106, 130],
    sky: [54, 131, 155],
};
pub const PURPLE: Weather = Weather {
    fog: [102, 90, 154],
    sky: [111, 119, 191],
};

/// A disc on the ground that switches the light (`LightModifier`, positions
/// and radii are `ff9.S(fixed)` = fixed / 256).
struct Zone {
    x: f32,
    z: f32,
    radius: f32,
    weather: Weather,
}

/// In the order the reference checks them: a later zone wins.
const ZONES: [Zone; 3] = [
    Zone {
        x: 329_728.0 / 256.0,
        z: -240_896.0 / 256.0,
        radius: 22_016.0 / 256.0,
        weather: EVENING,
    },
    Zone {
        x: 330_496.0 / 256.0,
        z: -237_568.0 / 256.0,
        radius: 15_104.0 / 256.0,
        weather: NIGHT,
    },
    Zone {
        x: 128_000.0 / 256.0,
        z: -202_240.0 / 256.0,
        radius: 46_080.0 / 256.0,
        weather: EVENING,
    },
];

impl Weather {
    /// The weather at a ground position (`GetWeatherLight`).
    pub fn at(x: f32, z: f32) -> Self {
        let mut weather = DAYLIGHT;
        for zone in &ZONES {
            if (x - zone.x).hypot(z - zone.z) < zone.radius {
                weather = zone.weather;
            }
        }
        weather
    }

    /// Fills the pixels nothing drew on with the sky and fades the far
    /// ground into the fog.
    pub fn apply(&self, frame: &mut Frame, horizon: f32) {
        if let Some(recording) = frame.recording_mut() {
            recording.fog(crate::frame_gpu::Fog {
                sky: self.sky,
                fog: self.fog,
                horizon,
                start: FOG_START,
                end: FOG_END,
            });
            return;
        }
        let width = frame.width as usize;
        let height = frame.height as usize;
        for y in 0..height {
            // Overhead at the top row, the fog color at the horizon row.
            let t = if horizon > 0.0 {
                (y as f32 / horizon).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let sky = mix(self.sky, self.fog, t * t);
            for x in 0..width {
                let i = y * width + x;
                let depth = frame.depth[i];
                let color = if depth.is_finite() {
                    let k = ((depth - FOG_START) / (FOG_END - FOG_START)).clamp(0.0, 1.0);
                    if k <= 0.0 {
                        continue;
                    }
                    let k = k * k * (3.0 - 2.0 * k);
                    let base = [
                        frame.rgba[i * 4],
                        frame.rgba[i * 4 + 1],
                        frame.rgba[i * 4 + 2],
                    ];
                    mix(base, self.fog, k)
                } else {
                    sky
                };
                frame.rgba[i * 4..i * 4 + 3].copy_from_slice(&color);
            }
        }
    }
}

fn mix(from: [u8; 3], to: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2]
        .map(|c| (f32::from(from[c]) + (f32::from(to[c]) - f32::from(from[c])) * t).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_follows_the_zone_under_the_player() {
        assert_eq!(Weather::at(1100.0, -800.0), DAYLIGHT);
        assert_eq!(Weather::at(1288.0, -1010.0), EVENING);
        assert_eq!(Weather::at(1291.0, -928.0), NIGHT);
        assert_eq!(Weather::at(500.0, -790.0), EVENING);
    }

    #[test]
    fn empty_pixels_take_the_sky_and_far_ground_the_fog() {
        let mut frame = Frame::new(2, 2, [0, 0, 0, 255]);
        frame.depth[2] = 10.0;
        frame.depth[3] = 400.0;
        frame.rgba[8..12].copy_from_slice(&[10, 10, 10, 255]);
        frame.rgba[12..16].copy_from_slice(&[10, 10, 10, 255]);
        DAYLIGHT.apply(&mut frame, 1.0);
        assert_eq!(&frame.rgba[0..3], &DAYLIGHT.sky);
        assert_eq!(&frame.rgba[8..11], &[10, 10, 10]);
        assert_eq!(&frame.rgba[12..15], &DAYLIGHT.fog);
    }
}
