//! The battle HUD drawn with the game's own font and window art
//! (`tantalus_assets::pc::dialog_render`, the dialog loader): the party
//! panel (name, HP, MP, ATB gauge), the enemy list, the command menu and
//! its ability/item list, the target cursor and damage numbers.
//!
//! Layout is in UI pixels of the 1080-line UI (`UIManager.UIContentSize`)
//! and scaled to the frame, as the game scales its NGUI root. The panels
//! follow `BattleHUD`'s `PartyDetailPanel` (bottom right), `CommandPanel`
//! (bottom left) and `AbilityPanel`/`ItemPanel` in placement; sizes and
//! colors are close, not measured from the prefabs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use tantalus_assets::pc::dialog_render::{
    DialogAssets, FONT_SIZE, GlyphSource, Image, WindowAtlasChoice, measure_baseline,
};

use crate::battle_raster::Frame;

/// UI lines the frame's height stands for.
const UI_HEIGHT: f32 = 1080.0;
/// `FF9TextTool.White` and the label shadow (`dialog_render`).
pub const WHITE: [u8; 3] = [200, 200, 200];
const SHADOW: [u8; 3] = [40, 40, 40];
/// Grayed-out entries (`FF9TextTool.Gray`).
pub const GRAY: [u8; 3] = [118, 118, 118];
/// Low HP (`FF9TextTool.Yellow`) and KO (`FF9TextTool.Red`).
pub const YELLOW: [u8; 3] = [200, 186, 22];
pub const RED: [u8; 3] = [200, 50, 50];
/// Heal numbers (`FF9TextTool.Green`).
pub const GREEN: [u8; 3] = [80, 200, 90];

/// One party row.
#[derive(Debug, Clone, PartialEq)]
pub struct PartyRow {
    pub name: String,
    pub hp: (u32, u32),
    pub mp: (u32, u32),
    /// Gauge fill 0..=1.
    pub atb: f32,
    /// Whose menu is open.
    pub asked: bool,
}

/// One list line: text, right column (MP cost, count) and whether it can
/// be picked.
#[derive(Debug, Clone, PartialEq)]
pub struct ListLine {
    pub text: String,
    pub right: String,
    pub enabled: bool,
}

/// The menu as drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuDraw {
    /// The command rows and whether each can be picked: Attack, the two
    /// ability commands and Item, then Defend and Change (the side
    /// column, beside the first and last rows).
    pub commands: Vec<ListLine>,
    pub command_cursor: usize,
    /// The open ability or item list and its cursor.
    pub list: Option<(Vec<ListLine>, usize)>,
    /// Whether the target cursor is up (the hand leaves the windows).
    pub targeting: bool,
    /// The help line (`BattleHUD` help window): the target's name or the
    /// entry's.
    pub help: Option<String>,
}

/// Everything the HUD shows in one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HudDraw {
    pub party: Vec<PartyRow>,
    /// Enemy names still standing.
    pub enemies: Vec<String>,
    pub menu: Option<MenuDraw>,
    /// Frame pixels above the targeted units.
    pub target: Vec<[f32; 2]>,
    /// Frame pixel, amount (negative heals, 0 a miss) and seconds since.
    pub numbers: Vec<([f32; 2], i64, f64)>,
    /// A line across the top (the battle's end, Steal's take).
    pub banner: Option<String>,
    /// The result screen: a title and its lines.
    pub results: Option<(String, Vec<ListLine>)>,
}

/// The font and window art, and windows drawn so far.
pub struct BattleHud {
    assets: DialogAssets,
    baseline: i32,
    windows: RefCell<HashMap<(i32, i32), Image>>,
}

impl BattleHud {
    /// Loads the font and window art from `FF9_Data/resources.assets`.
    pub fn load(game_data: &Path) -> Result<Self, String> {
        let assets =
            DialogAssets::load(&game_data.join("resources.assets"), WindowAtlasChoice::Blue)
                .map_err(|error| error.to_string())?;
        let baseline = measure_baseline(assets.glyphs(), FONT_SIZE);
        Ok(Self {
            assets,
            baseline,
            windows: RefCell::new(HashMap::new()),
        })
    }

    /// Draws `hud` over `frame`.
    pub fn draw(&self, frame: &mut Frame, hud: &HudDraw) {
        let scale = frame.height as f32 / UI_HEIGHT;
        let ui_width = frame.width as f32 / scale;
        let mut ui = Ui {
            hud: self,
            frame,
            scale,
        };
        let line = 62.0;
        // Party panel, bottom right (`PartyDetailPanel`).
        let party_w = 820.0;
        let party_h = 60.0 + line * hud.party.len().max(1) as f32;
        let party_x = ui_width - party_w - 20.0;
        let party_y = UI_HEIGHT - party_h - 20.0;
        ui.window(party_x, party_y, party_w, party_h);
        let columns = [
            party_x + 60.0,
            party_x + 330.0,
            party_x + 520.0,
            party_x + 640.0,
        ];
        ui.text(columns[1] + 20.0, party_y + 14.0, "HP", YELLOW);
        ui.text(columns[2] + 20.0, party_y + 14.0, "MP", YELLOW);
        ui.text(columns[3] + 20.0, party_y + 14.0, "ATB", YELLOW);
        for (row, member) in hud.party.iter().enumerate() {
            let y = party_y + 52.0 + line * row as f32;
            let color = if member.hp.0 == 0 {
                RED
            } else if member.hp.0 * 4 <= member.hp.1 {
                YELLOW
            } else {
                WHITE
            };
            if member.asked {
                ui.hand(columns[0] - 50.0, y + 8.0);
            }
            ui.text(columns[0], y, &member.name, color);
            ui.text_right(columns[1] + 150.0, y, &member.hp.0.to_string(), color);
            ui.text_right(columns[2] + 110.0, y, &member.mp.0.to_string(), color);
            // ATB gauge (`BattleHUD.ATBGauge`): blue while it fills,
            // yellow when full.
            let (bx, by, bw, bh) = (columns[3], y + 18.0, 150.0, 22.0);
            ui.rect(bx - 3.0, by - 3.0, bw + 6.0, bh + 6.0, [20, 20, 40], 1.0);
            let fill = if member.atb >= 1.0 {
                [230, 210, 70]
            } else {
                [90, 150, 230]
            };
            ui.rect(bx, by, bw * member.atb.clamp(0.0, 1.0), bh, fill, 1.0);
        }
        // Left: the command menu, or the enemy names.
        let left_w: f32 = 560.0;
        let left_x = 20.0;
        match &hud.menu {
            Some(menu) => {
                let rows = menu.commands.len().min(4);
                let h = 40.0 + line * rows as f32;
                let y = UI_HEIGHT - h - 20.0;
                // The list replaces the command window (`AbilityPanel`
                // over `CommandPanel`).
                let commands = if menu.list.is_some() {
                    &[][..]
                } else {
                    ui.window(left_x, y, left_w, h);
                    &menu.commands[..]
                };
                for (index, command) in commands.iter().enumerate() {
                    // Side column: Defend beside the first row, Change
                    // beside the last.
                    let (row, x) = match index {
                        0..=3 => (index, left_x + 80.0),
                        4 => (0, left_x + 360.0),
                        _ => (rows.saturating_sub(1), left_x + 360.0),
                    };
                    let ty = y + 22.0 + line * row as f32;
                    let color = if command.enabled { WHITE } else { GRAY };
                    ui.text(x, ty, &command.text, color);
                    if index == menu.command_cursor && menu.list.is_none() && !menu.targeting {
                        ui.hand(x - 50.0, ty + 8.0);
                    }
                }
                if let Some((lines, cursor)) = &menu.list {
                    let shown = 6usize;
                    let first = cursor.saturating_sub(shown - 1);
                    let list_h = 40.0 + line * shown.min(lines.len()).max(1) as f32;
                    let list_w = left_w;
                    let list_x = left_x;
                    let list_y = UI_HEIGHT - list_h - 20.0;
                    ui.window(list_x, list_y, list_w, list_h);
                    for (row, entry) in lines.iter().enumerate().skip(first).take(shown) {
                        let ty = list_y + 22.0 + line * (row - first) as f32;
                        let color = if entry.enabled { WHITE } else { GRAY };
                        ui.text(list_x + 80.0, ty, &entry.text, color);
                        ui.text_right(list_x + list_w - 40.0, ty, &entry.right, color);
                        if row == *cursor && !menu.targeting {
                            ui.hand(list_x + 30.0, ty + 8.0);
                        }
                    }
                }
                if let Some(help) = &menu.help {
                    let w = (ui.measure(help) + 120.0).max(400.0);
                    ui.window((ui_width - w) / 2.0, 20.0, w, 100.0);
                    ui.text((ui_width - ui.measure(help)) / 2.0, 42.0, help, WHITE);
                }
            }
            None => {
                let shown = hud.enemies.len().clamp(1, 4);
                let h = 40.0 + line * shown as f32;
                let y = UI_HEIGHT - h - 20.0;
                ui.window(left_x, y, left_w, h);
                for (row, name) in hud.enemies.iter().take(shown).enumerate() {
                    ui.text(left_x + 50.0, y + 22.0 + line * row as f32, name, WHITE);
                }
            }
        }
        if let Some(banner) = &hud.banner {
            let w = ui.measure(banner) + 160.0;
            ui.window((ui_width - w) / 2.0, 20.0, w, 100.0);
            ui.text((ui_width - ui.measure(banner)) / 2.0, 42.0, banner, WHITE);
        }
        // Target cursor: a hand pointing down at each unit.
        for [x, y] in &hud.target {
            ui.hand_down(x / scale, y / scale);
        }
        // The result screen (`BattleResultUI`), centered.
        if let Some((title, lines)) = &hud.results {
            let w = 1100.0;
            let h = 150.0 + line * lines.len() as f32;
            let (x, y) = ((ui_width - w) / 2.0, (UI_HEIGHT - h) / 2.0 - 80.0);
            ui.window(x, y, w, h);
            ui.text(
                (ui_width - ui.measure(title)) / 2.0,
                y + 30.0,
                title,
                YELLOW,
            );
            for (row, entry) in lines.iter().enumerate() {
                let ty = y + 110.0 + line * row as f32;
                let color = if entry.enabled { WHITE } else { YELLOW };
                ui.text(x + 80.0, ty, &entry.text, color);
                ui.text_right(x + w - 80.0, ty, &entry.right, color);
            }
        }
        // Damage numbers bounce up and settle (`btl2d` digits).
        for ([x, y], amount, since) in &hud.numbers {
            let (text, color) = match amount {
                0 => ("Miss".to_owned(), WHITE),
                amount if *amount < 0 => ((-amount).to_string(), GREEN),
                amount => (amount.to_string(), WHITE),
            };
            let rise = (since * 8.0).min(1.0) as f32 * 60.0;
            let ux = x / scale - ui.measure(&text) / 2.0;
            ui.text(ux, y / scale - 40.0 - rise, &text, color);
        }
    }
}

/// Drawing in UI pixels onto the frame.
struct Ui<'a> {
    hud: &'a BattleHud,
    frame: &'a mut Frame,
    scale: f32,
}

impl Ui<'_> {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 3], alpha: f32) {
        let s = self.scale;
        let (x0, y0) = ((x * s).round() as i32, (y * s).round() as i32);
        let (x1, y1) = (((x + w) * s).round() as i32, ((y + h) * s).round() as i32);
        self.frame.rect(x0, y0, x1 - x0, y1 - y0, color, alpha);
    }

    /// A window: the dialog body tiled and its border sliced.
    fn window(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let size = (w.round() as i32, h.round() as i32);
        let mut windows = self.hud.windows.borrow_mut();
        let image = windows.entry(size).or_insert_with(|| {
            let window = self.hud.assets.window();
            let mut image = Image::new(size.0.max(1) as usize, size.1.max(1) as usize);
            let pad = 18;
            window.draw_tiled(
                &mut image,
                (pad, pad, size.0 - 2 * pad, size.1 - 2 * pad),
                (size.0 / 2, size.1 / 2),
            );
            window.draw_sliced(&mut image, (0, 0, size.0, size.1));
            image
        });
        blit_scaled(self.frame, image, x, y, self.scale);
    }

    fn measure(&self, text: &str) -> f32 {
        let glyphs = self.hud.assets.glyphs();
        text.chars()
            .filter_map(|ch| glyphs.glyph(ch))
            .map(|glyph| glyph.advance as f32)
            .sum()
    }

    /// `text` with its top at `y` (UI pixels), the label shadow under it.
    fn text(&mut self, x: f32, y: f32, text: &str, color: [u8; 3]) {
        let glyphs = self.hud.assets.glyphs();
        let width = self.measure(text).ceil() as usize + 8;
        let height = (FONT_SIZE + 16) as usize;
        let mut image = Image::new(width.max(1), height);
        for (offset, shade) in [(4, SHADOW), (0, color)] {
            let mut pen = 0;
            for ch in text.chars() {
                let Some(glyph) = glyphs.glyph(ch) else {
                    continue;
                };
                let left = pen + glyph.min_x + offset;
                let top = self.hud.baseline - glyph.max_y() + offset;
                for row in 0..glyph.height {
                    for column in 0..glyph.width {
                        let coverage = glyph.coverage[row * glyph.width + column];
                        if coverage > 0 {
                            image.blend(
                                left + column as i32,
                                top + row as i32,
                                shade,
                                f32::from(coverage) / 255.0,
                            );
                        }
                    }
                }
                pen += glyph.advance;
            }
        }
        blit_scaled(self.frame, &image, x, y, self.scale);
    }

    fn text_right(&mut self, right: f32, y: f32, text: &str, color: [u8; 3]) {
        let width = self.measure(text);
        self.text(right - width, y, text, color);
    }

    /// The cursor hand pointing right, its tip at `(x, y)`.
    fn hand(&mut self, x: f32, y: f32) {
        self.triangle(x, y + 18.0, [[-34.0, -18.0], [-34.0, 18.0], [0.0, 0.0]]);
    }

    /// The target hand pointing down, its tip at `(x, y)`.
    fn hand_down(&mut self, x: f32, y: f32) {
        self.triangle(x, y, [[-20.0, -40.0], [20.0, -40.0], [0.0, 0.0]]);
    }

    fn triangle(&mut self, x: f32, y: f32, corners: [[f32; 2]; 3]) {
        let s = self.scale;
        let points = corners.map(|[cx, cy]| [(x + cx) * s, (y + cy) * s]);
        let min_x = points.iter().map(|p| p[0]).fold(f32::MAX, f32::min).floor() as i32;
        let max_x = points.iter().map(|p| p[0]).fold(f32::MIN, f32::max).ceil() as i32;
        let min_y = points.iter().map(|p| p[1]).fold(f32::MAX, f32::min).floor() as i32;
        let max_y = points.iter().map(|p| p[1]).fold(f32::MIN, f32::max).ceil() as i32;
        let edge = |a: [f32; 2], b: [f32; 2], p: [f32; 2]| {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        for (grow, color) in [(1.5, [20, 20, 20]), (0.0, [240, 240, 240])] {
            let center = [
                (points[0][0] + points[1][0] + points[2][0]) / 3.0,
                (points[0][1] + points[1][1] + points[2][1]) / 3.0,
            ];
            let grown = points.map(|p| {
                let (dx, dy) = (p[0] - center[0], p[1] - center[1]);
                let length = dx.hypot(dy).max(1.0);
                [p[0] + dx / length * grow, p[1] + dy / length * grow]
            });
            let area = edge(grown[0], grown[1], grown[2]);
            for py in min_y - 2..=max_y + 2 {
                for px in min_x - 2..=max_x + 2 {
                    let p = [px as f32 + 0.5, py as f32 + 0.5];
                    let w = [
                        edge(grown[1], grown[2], p),
                        edge(grown[2], grown[0], p),
                        edge(grown[0], grown[1], p),
                    ];
                    let inside = if area > 0.0 {
                        w.iter().all(|&v| v >= 0.0)
                    } else {
                        w.iter().all(|&v| v <= 0.0)
                    };
                    if inside {
                        self.frame.rect(px, py, 1, 1, color, 1.0);
                    }
                }
            }
        }
    }
}

/// Draws `image` (UI pixels, straight alpha) with its top left at `(x, y)`
/// UI pixels, scaled by `scale` with a box filter.
fn blit_scaled(frame: &mut Frame, image: &Image, x: f32, y: f32, scale: f32) {
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    let x0 = (x * scale).floor() as i32;
    let y0 = (y * scale).floor() as i32;
    let x1 = ((x + image.width as f32) * scale).ceil() as i32;
    let y1 = ((y + image.height as f32) * scale).ceil() as i32;
    let inverse = 1.0 / scale;
    for dy in y0.max(0)..y1.min(fh) {
        let sy0 = (dy as f32 * inverse - y).max(0.0);
        let sy1 = ((dy + 1) as f32 * inverse - y).min(image.height as f32);
        if sy1 <= sy0 {
            continue;
        }
        for dx in x0.max(0)..x1.min(fw) {
            let sx0 = (dx as f32 * inverse - x).max(0.0);
            let sx1 = ((dx + 1) as f32 * inverse - x).min(image.width as f32);
            if sx1 <= sx0 {
                continue;
            }
            // Premultiplied box average over the covered source texels.
            let mut sum = [0.0f32; 4];
            let mut weight = 0.0f32;
            let mut sy = sy0;
            while sy < sy1 {
                let row = sy.floor();
                let h = (row + 1.0).min(sy1) - sy;
                let mut sx = sx0;
                while sx < sx1 {
                    let column = sx.floor();
                    let w = (column + 1.0).min(sx1) - sx;
                    let at = (row as usize * image.width + column as usize) * 4;
                    let alpha = f32::from(image.rgba[at + 3]) / 255.0;
                    let area = w * h;
                    for (channel, total) in sum.iter_mut().take(3).enumerate() {
                        *total += f32::from(image.rgba[at + channel]) * alpha * area;
                    }
                    sum[3] += alpha * area;
                    weight += area;
                    sx = column + 1.0;
                }
                sy = row + 1.0;
            }
            if weight <= 0.0 || sum[3] <= 0.0 {
                continue;
            }
            // The covered part of the frame pixel is the source's share.
            let full = inverse * inverse;
            let alpha = (sum[3] / full).clamp(0.0, 1.0);
            let color = [0, 1, 2].map(|channel| (sum[channel] / sum[3]).round() as u8);
            frame.rect(dx, dy, 1, 1, color, alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scaled_blit_keeps_coverage() {
        let mut frame = Frame::new(4, 4, [0, 0, 0, 255]);
        let mut image = Image::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                image.blend(x, y, [255, 255, 255], 1.0);
            }
        }
        // 4x4 UI texels into 2x2 frame pixels at half scale, fully covered.
        blit_scaled(&mut frame, &image, 0.0, 0.0, 0.5);
        assert_eq!(&frame.rgba[0..3], &[255, 255, 255]);
        assert_eq!(&frame.rgba[(4 + 1) * 4..(4 + 1) * 4 + 3], &[255, 255, 255]);
        assert_eq!(&frame.rgba[2 * 4..2 * 4 + 3], &[0, 0, 0]);
    }
}
