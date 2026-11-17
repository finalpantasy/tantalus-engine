//! Animated model textures (blinking eyes, talking mouths): `GeoTexAnim` and `GEOTEXHEADER`.
//!
//! A `Models/GeoTexAnim/<geo name>.tab` table lists animations. Each one draws a small
//! rectangle of a sub texture over a rectangle of a main texture, frame after frame.
//! [`GeoTexAnim::service`] is `GeoTexAnim.geoTexAnimService` (30 times a second in the
//! field and in battle) and [`TexAnimPlayer`] keeps the render textures
//! (`GEOTEXHEADER.MultiTexAnimService`): the main texture is redrawn from its original
//! each time an animation changes frame, so two animations sharing a main texture wipe each
//! other's rectangle, as Memoria's do.
//!
//! The index CSV (`EmbeddedAsset/GeoTexAnimIndex/<geo name>.csv`, which says which model
//! material is the main and the sub texture of each animation) ships with Memoria, not with
//! the game: without one `GEOTEXHEADER.InitMultiTexAnim` links animation `i` to material `i`.

use std::collections::BTreeMap;

/// `GEOTEXANIMHEADER`: one animation of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexAnimHeader {
    /// Bit 0 plays, bit 1 plays once.
    pub flags: u8,
    pub num_frames: u8,
    /// Fixed point (12 bits) frames per service.
    pub rate: i16,
    pub rand_min: u16,
    pub rand_range: u16,
    /// Fixed point (12 bits) frame counter; negative while waiting for the next blink.
    pub frame: i32,
    pub last_frame: i16,
    /// `target`: x, y, width, height in the 128 by 128 texture space (x and width doubled).
    pub target: [i32; 4],
    /// Top left of each frame's source rectangle, same space.
    pub coords: Vec<[i32; 2]>,
}

/// `GEOTEXHEADER` without the Unity objects: the animations and their texture links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeoTexAnim {
    pub anims: Vec<TexAnimHeader>,
    /// Material index of each animation's main (drawn on) and sub (drawn from) texture.
    pub main_index: Vec<i32>,
    pub sub_index: Vec<i32>,
    rng: u32,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

impl GeoTexAnim {
    /// `GEOTEXHEADER.ReadTextureAnim` and `GEOTEXANIMHEADER.ReadData`; `None` on a short file.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let count = usize::from(u16_at(bytes, 0)?);
        let mut anims = Vec::with_capacity(count);
        for index in 0..count {
            let at = 4 + index * 20;
            let flags = *bytes.get(at)?;
            let num_frames = *bytes.get(at + 1)?;
            let rate = u16_at(bytes, at + 2)? as i16;
            let rand_min = u16_at(bytes, at + 4)?;
            let rand_range = u16_at(bytes, at + 6)?;
            let frame = u32_at(bytes, at + 8)? as i32;
            let offset = u32_at(bytes, at + 16)? as usize;
            let target = [
                i32::from(u16_at(bytes, offset)?) * 2,
                i32::from(u16_at(bytes, offset + 2)?),
                i32::from(u16_at(bytes, offset + 4)?) * 2,
                i32::from(u16_at(bytes, offset + 6)?),
            ];
            let frames = u32_at(bytes, offset + 8)? as usize;
            let mut coords = Vec::with_capacity(usize::from(num_frames));
            for frame_index in 0..usize::from(num_frames) {
                let at = frames + frame_index * 4;
                coords.push([
                    i32::from(u16_at(bytes, at)? as i16) * 2,
                    i32::from(u16_at(bytes, at + 2)? as i16),
                ]);
            }
            anims.push(TexAnimHeader {
                flags,
                num_frames,
                rate,
                rand_min,
                rand_range,
                frame,
                // `ReadData` overwrites the stored value with 1.
                last_frame: 1,
                target,
                coords,
            });
        }
        let identity: Vec<i32> = (0..count as i32).collect();
        Some(Self {
            anims,
            main_index: identity.clone(),
            sub_index: identity,
            rng: 0x2545_F491,
        })
    }

    /// `GeoTexAnim.SetTexAnimIndexs`: a CSV with a header line, then `main,sub` per animation.
    pub fn set_links(&mut self, csv: &str) {
        let mut main = Vec::new();
        let mut sub = Vec::new();
        for line in csv.lines().skip(1) {
            let mut parts = line.split(',');
            let (Some(a), Some(b)) = (parts.next(), parts.next()) else {
                continue;
            };
            let (Ok(a), Ok(b)) = (a.trim().parse(), b.trim().parse()) else {
                continue;
            };
            main.push(a);
            sub.push(b);
        }
        if main.len() >= self.anims.len() {
            self.main_index = main;
            self.sub_index = sub;
        }
    }

    pub fn rng_seed(&mut self, seed: u32) {
        self.rng = seed | 1;
    }

    /// `UnityEngine.Random.Range(min, range + 1)`, from a xorshift.
    fn random(&mut self, min: i32, range: i32) -> i32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let span = (range - min + 1).max(1) as u32;
        min + (self.rng % span) as i32
    }

    /// `geoTexAnimPlay`.
    pub fn play(&mut self, index: usize) {
        if let Some(anim) = self.anims.get_mut(index) {
            anim.flags |= 1;
            anim.frame = 0;
            anim.last_frame = 4096;
        }
    }

    /// `geoTexAnimPlayOnce`.
    pub fn play_once(&mut self, index: usize) {
        if let Some(anim) = self.anims.get_mut(index) {
            anim.flags |= 3;
            anim.frame = 0;
            anim.last_frame = 4096;
        }
    }

    /// `geoTexAnimStop`.
    pub fn stop(&mut self, index: usize) {
        if let Some(anim) = self.anims.get_mut(index) {
            anim.flags &= !3;
        }
    }

    /// `geoTexAnimService`: one step of every playing animation. Returns the animations that
    /// moved to a new frame, `(animation, frame)`, to redraw.
    pub fn service(&mut self) -> Vec<(usize, usize)> {
        let mut draws = Vec::new();
        for index in 0..self.anims.len() {
            let (flags, num_frames, rate, rand_min, rand_range) = {
                let anim = &self.anims[index];
                (
                    anim.flags,
                    anim.num_frames,
                    anim.rate,
                    anim.rand_min,
                    anim.rand_range,
                )
            };
            if flags & 1 == 0 || num_frames == 0 {
                continue;
            }
            let mut frame_long = self.anims[index].frame;
            let frame_short = (frame_long >> 12) as i16;
            if frame_short >= 0 {
                if frame_short != self.anims[index].last_frame {
                    draws.push((index, frame_short as usize));
                    self.anims[index].last_frame = frame_short;
                }
                frame_long += i32::from(rate);
            } else {
                frame_long += 4096;
            }
            if frame_long >> 12 < i32::from(num_frames) {
                self.anims[index].frame = frame_long;
            } else if rand_range > 0 {
                let wait = self.random(i32::from(rand_min), i32::from(rand_range));
                self.anims[index].frame = -(wait << 12);
            } else if flags & 2 != 0 {
                self.anims[index].flags &= !3;
            } else {
                self.anims[index].frame = 0;
            }
        }
        draws
    }
}

/// Reads the table of `geo_id`'s model from the models bundle (`None` for a model without one).
pub fn load(
    store: &crate::pc::model_assembly::SerializedStore<'_, '_>,
    entries: &[crate::pc::model_assembly::ContainerEntry],
    geo_id: u32,
) -> Option<GeoTexAnim> {
    let name = crate::pc::model_assembly::geo_name(geo_id)?.to_ascii_lowercase();
    let path = format!("assets/resources/models/geotexanim/{name}.tab.bytes");
    let entry = entries.iter().find(|entry| entry.path == path)?;
    GeoTexAnim::parse(&store.text_asset(entry.path_id).ok()??)
}

/// The texture of each renderer of `model`, in renderer order: `smr.material.mainTexture`,
/// which is what the animations' material indexes mean (`GeoTexAnim.Load`).
pub fn material_textures(
    model: &crate::pc::model_assembly::AssembledModel,
) -> Vec<Option<TexRef<'_>>> {
    model
        .parts
        .iter()
        .map(|part| {
            let material = part.submeshes.first()?.material.as_ref()?;
            material.textures.iter().find_map(|texture| {
                let rgba = texture.rgba.as_deref()?;
                let width = usize::try_from(texture.width).ok()?;
                let height = usize::try_from(texture.height).ok()?;
                (width > 0 && height > 0 && rgba.len() >= width * height * 4).then_some(TexRef {
                    width,
                    height,
                    rgba,
                })
            })
        })
        .collect()
}

/// A material's texture: top row first RGBA8.
#[derive(Debug, Clone, Copy)]
pub struct TexRef<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
}

/// The animation state of one model plus the drawn main textures.
#[derive(Debug, Clone)]
pub struct TexAnimPlayer {
    pub anim: GeoTexAnim,
    /// Redrawn main textures by material index.
    canvases: BTreeMap<i32, Vec<u8>>,
    serviced: u64,
    /// `FieldMap.Update`: a field character with two or more animations that has none playing
    /// starts animation 2 (the blink) again, every frame.
    pub field_blink: bool,
}

impl TexAnimPlayer {
    pub fn new(anim: GeoTexAnim) -> Self {
        Self {
            anim,
            canvases: BTreeMap::new(),
            serviced: 0,
            field_blink: false,
        }
    }

    /// Runs the service until it has run `tick` times in all (30 a second) and draws the frames
    /// it changed; `textures[i]` is material `i`'s texture.
    pub fn advance_to(&mut self, tick: u64, textures: &[Option<TexRef<'_>>]) {
        // A table that never moves (all stopped) needs no catching up from a long way back.
        let from = self.serviced.max(tick.saturating_sub(100_000));
        for _ in from..tick {
            if self.field_blink
                && self.anim.anims.len() >= 2
                && self.anim.anims.iter().all(|anim| anim.flags & 1 == 0)
            {
                self.anim.play(2);
            }
            for (index, frame) in self.anim.service() {
                self.draw(index, frame, textures);
            }
        }
        self.serviced = tick;
    }

    /// The drawn texture of material `index`, when an animation has drawn on it.
    pub fn canvas(&self, index: usize) -> Option<&[u8]> {
        self.canvases.get(&(index as i32)).map(Vec::as_slice)
    }

    /// `GEOTEXHEADER.MultiTexAnimService`: the main texture again, then the frame's rectangle
    /// of the sub texture over the target rectangle.
    fn draw(&mut self, index: usize, frame: usize, textures: &[Option<TexRef<'_>>]) {
        let main_at = self.anim.main_index.get(index).copied().unwrap_or(-1);
        let sub_at = self.anim.sub_index.get(index).copied().unwrap_or(-1);
        let texture = |at: i32| usize::try_from(at).ok().and_then(|at| *textures.get(at)?);
        let (Some(main), Some(sub)) = (texture(main_at), texture(sub_at)) else {
            return;
        };
        let anim = &self.anim.anims[index];
        let Some(coords) = anim.coords.get(frame) else {
            return;
        };
        let canvas = self.canvases.entry(main_at).or_default();
        canvas.clear();
        canvas.extend_from_slice(&main.rgba[..main.width * main.height * 4]);
        let (mx, my) = (main.width as f32 / 128.0, main.height as f32 / 128.0);
        let (sx, sy) = (sub.width as f32 / 128.0, sub.height as f32 / 128.0);
        let dest = [
            anim.target[0] as f32 * mx,
            anim.target[1] as f32 * my,
            anim.target[2] as f32 * mx,
            anim.target[3] as f32 * my,
        ];
        // `InitMultiTexAnim`: the source rectangle is inset by half a texel.
        let src_left = coords[0] as f32 * sx + 0.5;
        let src_top = coords[1] as f32 * sy + 0.5;
        let src_width = anim.target[2] as f32 * sx - 1.0;
        let src_height = anim.target[3] as f32 * sy - 1.0;
        let (x0, y0) = (dest[0].round() as i32, dest[1].round() as i32);
        let (x1, y1) = (
            (dest[0] + dest[2]).round() as i32,
            (dest[1] + dest[3]).round() as i32,
        );
        let (dw, dh) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
        for y in y0.max(0)..y1.min(main.height as i32) {
            for x in x0.max(0)..x1.min(main.width as i32) {
                let u = src_left + (x - x0) as f32 / dw * src_width;
                let v = src_top + (y - y0) as f32 / dh * src_height;
                let (u, v) = (u as usize, v as usize);
                if u >= sub.width || v >= sub.height {
                    continue;
                }
                let from = (v * sub.width + u) * 4;
                let to = (y as usize * main.width + x as usize) * 4;
                let alpha = u32::from(sub.rgba[from + 3]);
                for channel in 0..3 {
                    let over = u32::from(sub.rgba[from + channel]) * alpha
                        + u32::from(canvas[to + channel]) * (255 - alpha);
                    canvas[to + channel] = (over / 255) as u8;
                }
                canvas[to + 3] = canvas[to + 3].max(sub.rgba[from + 3]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One animation: 2 frames, target (4, 0) 2 by 2 in halved x, sources at (0, 8) and (0, 10).
    fn table() -> Vec<u8> {
        let mut bytes = vec![1, 0, 0, 0];
        // flags 1, 2 frames, rate 0x1000, randmin 3, randrange 5, frame 0, count 1, tex 0,
        // lastframe 0, offset 24.
        bytes.extend([
            1, 2, 0x00, 0x10, 3, 0, 5, 0, 0, 0, 0, 0, 1, 0, 0, 0, 24, 0, 0, 0,
        ]);
        // target x/2 = 2, y 0, w/2 = 1, h 2 (so 4, 0, 2, 2), frames at 36.
        bytes.extend([2, 0, 0, 0, 1, 0, 2, 0, 36, 0, 0, 0]);
        // frames: (0, 8) and (0, 10) with x halved on disk.
        bytes.extend([0, 0, 8, 0, 0, 0, 10, 0]);
        bytes
    }

    #[test]
    fn the_table_parses() {
        let anim = GeoTexAnim::parse(&table()).expect("parses");
        assert_eq!(anim.anims.len(), 1);
        assert_eq!(anim.anims[0].target, [4, 0, 2, 2]);
        assert_eq!(anim.anims[0].coords, vec![[0, 8], [0, 10]]);
        assert_eq!(anim.anims[0].last_frame, 1);
        assert!(GeoTexAnim::parse(&table()[..30]).is_none());
    }

    #[test]
    fn the_service_walks_the_frames_then_waits_a_random_time() {
        let mut anim = GeoTexAnim::parse(&table()).expect("parses");
        // Frame 0 is new (last frame was 1): drawn.
        assert_eq!(anim.service(), vec![(0, 0)]);
        assert_eq!(anim.service(), vec![(0, 1)]);
        // Past the last frame: a wait of 3 to 5 services, nothing drawn meanwhile.
        let wait = -(anim.anims[0].frame >> 12);
        assert!((3..=5).contains(&wait), "waits {wait}");
        for _ in 0..wait {
            assert!(anim.service().is_empty());
        }
        assert_eq!(anim.service(), vec![(0, 0)]);
    }

    #[test]
    fn a_field_character_blinks_by_itself() {
        let mut anim = GeoTexAnim::parse(&table()).expect("parses");
        // Three animations: the blink is the third.
        anim.anims.push(anim.anims[0].clone());
        anim.anims.push(anim.anims[0].clone());
        let mut player = TexAnimPlayer::new(anim);
        player.field_blink = true;
        for anim in &mut player.anim.anims {
            anim.flags = 0;
        }
        player.advance_to(1, &[]);
        assert_eq!(player.anim.anims[2].flags & 1, 1);
        assert_eq!(player.anim.anims[0].flags & 1, 0);
    }

    #[test]
    fn play_once_stops_after_the_last_frame() {
        let mut anim = GeoTexAnim::parse(&table()).expect("parses");
        anim.anims[0].rand_range = 0;
        anim.play_once(0);
        assert_eq!(anim.service(), vec![(0, 0)]);
        assert_eq!(anim.service(), vec![(0, 1)]);
        assert!(anim.service().is_empty());
        assert_eq!(anim.anims[0].flags & 3, 0);
        assert!(anim.service().is_empty());
    }

    #[test]
    fn a_frame_is_drawn_over_the_main_texture() {
        // A 128 by 128 texture: black with a white strip at rows 8..12.
        let mut rgba = vec![0u8; 128 * 128 * 4];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        for y in 8..12 {
            for x in 0..128 {
                let at = (y * 128 + x) * 4;
                rgba[at..at + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let texture = TexRef {
            width: 128,
            height: 128,
            rgba: &rgba,
        };
        let mut player = TexAnimPlayer::new(GeoTexAnim::parse(&table()).expect("parses"));
        assert!(player.canvas(0).is_none());
        player.advance_to(1, &[Some(texture)]);
        let canvas = player.canvas(0).expect("drawn");
        // Frame 0 copies rows 8.. of the sub texture onto rows 0..2 at x 4..6.
        let at = |x: usize, y: usize| canvas[(y * 128 + x) * 4];
        assert_eq!(at(4, 0), 255);
        assert_eq!(at(5, 1), 255);
        assert_eq!(at(6, 0), 0);
        assert_eq!(at(4, 2), 0);
    }
}
