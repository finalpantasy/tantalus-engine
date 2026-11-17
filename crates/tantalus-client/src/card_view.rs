//! Tetra Master on screen: the board, cards, arrows, cursor, hands, battle numbers and the
//! result, drawn on a CPU frame from the game's card art (`QuadMistUI` sprites in
//! `resources.assets`, textures in `sharedassets2.assets`).

use std::collections::HashMap;
use std::path::Path;

use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
use tantalus_assets::pc::texture::Texture2D;

use tantalus_game::cards::{CardGame, Input, Phase, Slot};

use crate::battle_raster::Frame;

/// `sharedassets2.assets` Texture2D path ids of the two card atlases.
const ATLAS_PATHS: [i64; 2] = [20, 44];

/// One sprite: where it sits in a decoded atlas (top-left origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sprite {
    pub atlas: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The card sprites and the two atlases they live in.
pub struct CardArt {
    atlases: Vec<(u32, u32, Vec<u8>)>,
    sprites: HashMap<String, Sprite>,
}

impl CardArt {
    /// Reads the sprites from `resources.assets` and the atlases from `sharedassets2.assets`.
    pub fn load(game_data: &Path) -> Result<Self, String> {
        let mut shared = StandaloneSerialized::open(&game_data.join("sharedassets2.assets"))
            .map_err(|error| error.to_string())?;
        let objects: Vec<_> = shared.objects().to_vec();
        let mut atlases = Vec::new();
        for path in ATLAS_PATHS {
            let object = objects
                .iter()
                .find(|object| object.class_id == 28 && object.path_id == path)
                .ok_or_else(|| format!("texture {path} missing"))?;
            let bytes = shared.read_object(object).map_err(|e| e.to_string())?;
            let texture = Texture2D::parse(&bytes, "card atlas").map_err(|e| e.to_string())?;
            let rgba = texture.rgba_bytes().map_err(|e| e.to_string())?;
            atlases.push((texture.width as u32, texture.height as u32, rgba));
        }
        let mut resources = StandaloneSerialized::open(&game_data.join("resources.assets"))
            .map_err(|error| error.to_string())?;
        let objects: Vec<_> = resources.objects().to_vec();
        let mut sprites = HashMap::new();
        for object in objects.iter().filter(|object| object.class_id == 213) {
            let bytes = resources.read_object(object).map_err(|e| e.to_string())?;
            if let Some((name, sprite)) = parse_sprite(&bytes, &atlases) {
                sprites.insert(name, sprite);
            }
        }
        if sprites.is_empty() {
            return Err("no card sprites found".to_owned());
        }
        Ok(Self { atlases, sprites })
    }

    pub fn sprite(&self, name: &str) -> Option<Sprite> {
        self.sprites.get(name).copied()
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.sprites.keys().cloned().collect();
        names.sort();
        names
    }

    /// Draws sprite `name` into the box `(x, y, width, height)`, scaled, over `frame`.
    pub fn blit(&self, frame: &mut Frame, name: &str, x: i32, y: i32, width: i32, height: i32) {
        let Some(sprite) = self.sprite(name) else {
            return;
        };
        self.blit_sprite(frame, sprite, (x, y, width, height));
    }

    pub fn blit_sprite(&self, frame: &mut Frame, sprite: Sprite, area: (i32, i32, i32, i32)) {
        let (x, y, width, height) = area;
        let (atlas_width, _, rgba) = &self.atlases[sprite.atlas];
        for row in 0..height.max(0) {
            let target_y = y + row;
            if target_y < 0 || target_y >= frame.height as i32 {
                continue;
            }
            let source_y = sprite.y + (row as u32 * sprite.height / height.max(1) as u32);
            for column in 0..width.max(0) {
                let target_x = x + column;
                if target_x < 0 || target_x >= frame.width as i32 {
                    continue;
                }
                let source_x = sprite.x + (column as u32 * sprite.width / width.max(1) as u32);
                let from = ((source_y * atlas_width + source_x) * 4) as usize;
                let alpha = f32::from(rgba[from + 3]) / 255.0;
                if alpha <= 0.0 {
                    continue;
                }
                let at = (target_y as usize * frame.width as usize + target_x as usize) * 4;
                for channel in 0..3 {
                    let dst = f32::from(frame.rgba[at + channel]);
                    let src = f32::from(rgba[from + channel]);
                    frame.rgba[at + channel] = (dst + (src - dst) * alpha).round() as u8;
                }
            }
        }
    }
}

/// A Unity `Sprite` object: its name and the part of the atlas it covers. The rect sits at
/// the name's end, the texture reference 48 bytes later; Unity's rows count from the bottom.
fn parse_sprite(bytes: &[u8], atlases: &[(u32, u32, Vec<u8>)]) -> Option<(String, Sprite)> {
    let length = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
    let name = String::from_utf8_lossy(bytes.get(4..4 + length)?).into_owned();
    let start = (4 + length).div_ceil(4) * 4;
    let float = |at: usize| -> Option<f32> {
        Some(f32::from_le_bytes(
            bytes.get(start + at..start + at + 4)?.try_into().ok()?,
        ))
    };
    let file = i32::from_le_bytes(bytes.get(start + 48..start + 52)?.try_into().ok()?);
    let path = i64::from_le_bytes(bytes.get(start + 52..start + 60)?.try_into().ok()?);
    if file != 4 {
        return None;
    }
    let atlas = ATLAS_PATHS.iter().position(|id| *id == path)?;
    let (_, atlas_height, _) = atlases.get(atlas)?;
    let (x, y, width, height) = (float(0)?, float(4)?, float(8)?, float(12)?);
    let top = *atlas_height as f32 - y - height;
    Some((
        name,
        Sprite {
            atlas,
            x: x as u32,
            y: top.max(0.0) as u32,
            width: width as u32,
            height: height as u32,
        },
    ))
}

/// The card game in the play window: the art and the window that shows it.
pub struct CardView {
    art: Option<CardArt>,
    window: Option<crate::battle_view::BattleWindow>,
}

impl CardView {
    /// Loads the art from the install's data folder; without it the game still plays, blind.
    pub fn open(game_data: &Path) -> Self {
        let art = CardArt::load(game_data)
            .map_err(|error| println!("!! card art: {error}"))
            .ok();
        Self { art, window: None }
    }

    /// Draws the game as it stands.
    pub fn update(&mut self, world: &mut bevy::ecs::world::World, game: &CardGame) {
        let Some(art) = &self.art else {
            return;
        };
        let frame = art.render(&game.view());
        match &self.window {
            Some(window) => window.show(world, frame),
            None => self.window = Some(crate::battle_view::BattleWindow::open(world, &frame)),
        }
    }

    pub fn close(self, world: &mut bevy::ecs::world::World) {
        if let Some(window) = self.window {
            window.close(world);
        }
    }
}

/// `capture-card --game-data <FF9_Data> --output-dir <dir>`: plays a game with a fixed seed
/// against stage 0 and saves the card choice, a game in progress with the battle numbers up,
/// and the result as PNGs.
pub fn run_capture(args: &[std::ffi::OsString]) -> Result<(), String> {
    let mut game_data = None;
    let mut output = None;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("option needs a value")?;
        match flag.to_str() {
            Some("--game-data") => game_data = Some(std::path::PathBuf::from(value)),
            Some("--output-dir") => output = Some(std::path::PathBuf::from(value)),
            _ => return Err("capture-card takes --game-data and --output-dir".to_owned()),
        }
    }
    let game_data = game_data.ok_or("--game-data is required")?;
    let output = output.ok_or("--output-dir is required")?;
    capture_game(&game_data, &output)
}

/// The captures of [`run_capture`].
pub fn capture_game(game_data: &Path, output: &Path) -> Result<(), String> {
    let art = CardArt::load(game_data)?;
    let tables = tantalus_tools::card_play::load_tables(game_data)?;
    let mut world = tantalus_game::FieldWorld::new();
    // Twelve cards of mixed kinds from across the set.
    world.cards = [0u32, 3, 7, 12, 18, 25, 31, 40, 47, 55, 63, 70].to_vec();
    world.card_seed = 0x1234_5678_9ABC_DEF0;
    let mut game =
        CardGame::start(&mut world, &tables, 20).ok_or("fewer than five cards to play")?;
    let save = |game: &CardGame, name: &str| {
        crate::battle_view::save_png(&art.render(&game.view()), &output.join(name))
    };
    for index in 0..5 {
        game.press(Input::Confirm);
        if index < 4 {
            game.press(Input::Down);
        }
    }
    save(&game, "card_select.png")?;
    while game.view().cursor < game.view().collection.len() {
        game.press(Input::Down);
    }
    game.press(Input::Confirm);
    let mut shot_play = false;
    let mut shot_result = false;
    for _ in 0..200_000 {
        match game.phase().clone() {
            Phase::Done => break,
            Phase::Hand => game.press(Input::Confirm),
            Phase::Cell => {
                let free = game
                    .view()
                    .board
                    .iter()
                    .position(|slot| *slot == Slot::Empty)
                    .ok_or("no free cell")?;
                while game.view().cell_cursor % 4 != free % 4 {
                    game.press(Input::Right);
                }
                while game.view().cell_cursor / 4 != free / 4 {
                    game.press(Input::Down);
                }
                let placed = game
                    .view()
                    .board
                    .iter()
                    .filter(|slot| matches!(slot, Slot::Card { .. }))
                    .count();
                if placed == 5 && !shot_play {
                    save(&game, "card_choose_cell.png")?;
                }
                game.press(Input::Confirm);
            }
            Phase::Target { .. } | Phase::Prize => {
                save(&game, "card_prize.png").ok();
                game.press(Input::Confirm);
            }
            Phase::Resolve => {
                let placed = game
                    .view()
                    .board
                    .iter()
                    .filter(|slot| matches!(slot, Slot::Card { .. }))
                    .count();
                if !shot_play && placed >= 3 && game.view().popup.is_some() {
                    save(&game, "card_game.png")?;
                    shot_play = true;
                }
                game.advance(1);
            }
            Phase::Banner { .. } => {
                if !shot_result {
                    save(&game, "card_result.png")?;
                    shot_result = true;
                }
                game.advance(1);
            }
            _ => game.advance(1),
        }
    }
    if !shot_play || !shot_result {
        return Err(format!(
            "missing captures (game {shot_play}, result {shot_result})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<std::path::PathBuf> {
        std::env::var_os("FFIX_GAME_DATA").map(Into::into)
    }

    #[test]
    fn real_card_sprites_load() {
        let Some(dir) = data() else { return };
        let art = CardArt::load(&dir).unwrap();
        assert_eq!(
            art.names()
                .iter()
                .filter(|n| n.starts_with("card_") && n[5..7].chars().all(|c| c.is_ascii_digit()))
                .count(),
            100
        );
        assert!(art.sprite("card_00.png").is_some());
        assert!(art.sprite("card_back.png").is_some());
    }

    #[test]
    fn dump_sprites() {
        let (Some(dir), Some(out)) = (data(), std::env::var_os("CARD_DUMP")) else {
            return;
        };
        let art = CardArt::load(&dir).unwrap();
        for name in ["card_mg.png", "card_tray_box.png", "card_player_frame.png"] {
            let sprite = art.sprite(name).unwrap();
            let mut frame = Frame::new(sprite.width, sprite.height, [90, 90, 90, 255]);
            art.blit(
                &mut frame,
                name,
                0,
                0,
                sprite.width as i32,
                sprite.height as i32,
            );
            crate::battle_view::save_png(&frame, &std::path::Path::new(&out).join(name)).unwrap();
        }
    }
}
