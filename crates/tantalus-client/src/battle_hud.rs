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

use tantalus_assets::pc::dialog_render::{DialogAssets, FONT_SIZE, Image, measure_baseline};

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
    /// Trance gauge fill 0..=1 and whether in Trance; `None` without a
    /// gauge.
    pub trance: Option<(f32, bool)>,
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
    /// Whether `AllMenuPanel` (party panel, command menu or enemy list) shows.
    pub panels: bool,
}

/// The font and window art, and windows drawn so far.
pub struct BattleHud {
    assets: DialogAssets,
    baseline: i32,
    windows: RefCell<HashMap<(i32, i32), Image>>,
    /// Text lines drawn lately, by text and color.
    texts: RefCell<HashMap<(String, [u8; 3]), Image>>,
    /// The skin's hand cursor pointing right and its width in UI pixels (`ui_skin::hand`);
    /// `None` draws a stand-in arrow.
    hand: Option<(Image, f32)>,
    /// The skin's placement numbers.
    layout: Layout,
    /// The PS1 battle digits (`ui_skin::battle_digits`), when the PS1 skin draws.
    digits: Option<crate::ui_skin::BattleDigits>,
    /// UI pixels of room above a text line's top for glyphs taller than the font size (the PS1
    /// font's 12-line cells).
    text_lift: i32,
}

/// Where the panels go, in UI pixels. The Steam numbers follow `BattleHUD`'s panels by eye
/// (see the module header); the PS1 ones fit the PS1 font, whose glyphs are about one and a
/// half times as wide, in the PS1 screen's 320 by 224 pixels (4.5 UI pixels a PS1 pixel
/// across the 4:3 area, 1080 / 224 down).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layout {
    line: f32,
    margin: f32,
    party_w: f32,
    /// Name, HP, MP and ATB columns, from the party window's left edge.
    columns: [f32; 4],
    header_y: f32,
    first_row_y: f32,
    party_pad_h: f32,
    /// Right edges of the HP and MP numbers, from their columns.
    hp_right: f32,
    mp_right: f32,
    header_shift: f32,
    /// ATB gauge: offset below the row top, width, height.
    atb: (f32, f32, f32),
    left_w: f32,
    command_x: f32,
    side_x: f32,
    rows_pad_h: f32,
    row_y: f32,
    list_text_x: f32,
    list_right_inset: f32,
    enemy_x: f32,
}

impl Layout {
    fn steam() -> Self {
        Self {
            line: 62.0,
            margin: 20.0,
            party_w: 820.0,
            columns: [60.0, 330.0, 520.0, 640.0],
            header_y: 14.0,
            first_row_y: 52.0,
            party_pad_h: 60.0,
            hp_right: 150.0,
            mp_right: 110.0,
            header_shift: 20.0,
            atb: (18.0, 150.0, 22.0),
            left_w: 560.0,
            command_x: 80.0,
            side_x: 360.0,
            rows_pad_h: 40.0,
            row_y: 22.0,
            list_text_x: 80.0,
            list_right_inset: 40.0,
            enemy_x: 50.0,
        }
    }

    fn ps1() -> Self {
        // PS1 pixels to UI pixels, across and down.
        let (x, y) = (4.5, 1080.0 / 224.0);
        Self {
            line: 14.0 * y,
            margin: 4.0 * x,
            party_w: 184.0 * x,
            columns: [10.0 * x, 60.0 * x, 102.0 * x, 136.0 * x],
            header_y: 4.0 * y,
            first_row_y: 16.0 * y,
            party_pad_h: 19.0 * y,
            hp_right: 40.0 * x,
            mp_right: 28.0 * x,
            header_shift: 14.0 * x,
            atb: (5.0 * y, 44.0 * x, 4.0 * y),
            left_w: 124.0 * x,
            command_x: 18.0 * x,
            side_x: 72.0 * x,
            rows_pad_h: 10.0 * y,
            row_y: 4.0 * y,
            list_text_x: 18.0 * x,
            list_right_inset: 8.0 * x,
            enemy_x: 12.0 * x,
        }
    }
}

impl BattleHud {
    /// Loads the font, window and hand art of the UI skin (`ui_skin`).
    pub fn load(game_data: &Path) -> Result<Self, String> {
        let assets = crate::ui_skin::dialog_assets(game_data)?;
        let baseline = measure_baseline(assets.glyphs(), FONT_SIZE);
        let hand = crate::ui_skin::hand(game_data);
        let ps1 = crate::ui_skin::active() == crate::ui_skin::UiSkin::Ps1;
        Ok(Self {
            assets,
            baseline,
            windows: RefCell::new(HashMap::new()),
            texts: RefCell::new(HashMap::new()),
            hand,
            layout: if ps1 { Layout::ps1() } else { Layout::steam() },
            digits: crate::ui_skin::battle_digits(),
            text_lift: if ps1 { 12 } else { 0 },
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
        let lay = self.layout;
        let line = lay.line;
        // `AllMenuPanel` (party panel, command menu, enemy list) is off until
        // `PHASE_MENU_ON` ends and again from `PHASE_MENU_OFF`
        // (`battle.cs` `FF9BMenu_EnableMenu`, `BattleHUD.Scene.cs` `Show`).
        if hud.panels {
            // Party panel, bottom right (`PartyDetailPanel`).
            let party_w = lay.party_w;
            let party_h = lay.party_pad_h + line * hud.party.len().max(1) as f32;
            let party_x = ui_width - party_w - lay.margin;
            let party_y = UI_HEIGHT - party_h - lay.margin;
            ui.window(party_x, party_y, party_w, party_h);
            let columns = lay.columns.map(|column| party_x + column);
            let header_y = party_y + lay.header_y;
            ui.text(columns[1] + lay.header_shift, header_y, "HP", YELLOW);
            ui.text(columns[2] + lay.header_shift, header_y, "MP", YELLOW);
            ui.text(columns[3] + lay.header_shift, header_y, "ATB", YELLOW);
            for (row, member) in hud.party.iter().enumerate() {
                let y = party_y + lay.first_row_y + line * row as f32;
                let color = if member.hp.0 == 0 {
                    RED
                } else if member.hp.0 * 4 <= member.hp.1 {
                    YELLOW
                } else {
                    WHITE
                };
                if member.asked {
                    ui.hand(columns[0] - 10.0, y + 8.0);
                }
                ui.text(columns[0], y, &member.name, color);
                ui.text_right(
                    columns[1] + lay.hp_right,
                    y,
                    &member.hp.0.to_string(),
                    color,
                );
                ui.text_right(
                    columns[2] + lay.mp_right,
                    y,
                    &member.mp.0.to_string(),
                    color,
                );
                // ATB gauge (`BattleHUD.ATBGauge`): blue while it fills,
                // yellow when full.
                let (bx, by, bw, bh) = (columns[3], y + lay.atb.0, lay.atb.1, lay.atb.2);
                ui.rect(bx - 3.0, by - 3.0, bw + 6.0, bh + 6.0, [20, 20, 40], 1.0);
                let fill = if member.atb >= 1.0 {
                    [230, 210, 70]
                } else {
                    [90, 150, 230]
                };
                ui.rect(bx, by, bw * member.atb.clamp(0.0, 1.0), bh, fill, 1.0);
                // Trance gauge under it (`BattleHUD` `TranceSlider`), pink;
                // brighter while in Trance.
                if let Some((trance, active)) = member.trance {
                    let (ty, th) = (by + bh + 4.0, 8.0);
                    ui.rect(bx - 3.0, ty - 2.0, bw + 6.0, th + 4.0, [30, 15, 30], 1.0);
                    let fill = if active {
                        [255, 150, 230]
                    } else {
                        [220, 80, 170]
                    };
                    ui.rect(bx, ty, bw * trance.clamp(0.0, 1.0), th, fill, 1.0);
                }
            }
            // Left: the command menu, or the enemy names.
            let left_w: f32 = lay.left_w;
            let left_x = lay.margin;
            match &hud.menu {
                Some(menu) => {
                    let rows = menu.commands.len().min(4);
                    let h = lay.rows_pad_h + line * rows as f32;
                    let y = UI_HEIGHT - h - lay.margin;
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
                            0..=3 => (index, left_x + lay.command_x),
                            4 => (0, left_x + lay.side_x),
                            _ => (rows.saturating_sub(1), left_x + lay.side_x),
                        };
                        let ty = y + lay.row_y + line * row as f32;
                        let color = if command.enabled { WHITE } else { GRAY };
                        ui.text(x, ty, &command.text, color);
                        if index == menu.command_cursor && menu.list.is_none() && !menu.targeting {
                            ui.hand(x - 10.0, ty + 8.0);
                        }
                    }
                    if let Some((lines, cursor)) = &menu.list {
                        let shown = 6usize;
                        let first = cursor.saturating_sub(shown - 1);
                        let list_h = lay.rows_pad_h + line * shown.min(lines.len()).max(1) as f32;
                        let list_w = left_w;
                        let list_x = left_x;
                        let list_y = UI_HEIGHT - list_h - lay.margin;
                        ui.window(list_x, list_y, list_w, list_h);
                        for (row, entry) in lines.iter().enumerate().skip(first).take(shown) {
                            let ty = list_y + lay.row_y + line * (row - first) as f32;
                            let color = if entry.enabled { WHITE } else { GRAY };
                            ui.text(list_x + lay.list_text_x, ty, &entry.text, color);
                            ui.text_right(
                                list_x + list_w - lay.list_right_inset,
                                ty,
                                &entry.right,
                                color,
                            );
                            if row == *cursor && !menu.targeting {
                                ui.hand(list_x + lay.list_text_x - 10.0, ty + 8.0);
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
                    let h = lay.rows_pad_h + line * shown as f32;
                    let y = UI_HEIGHT - h - lay.margin;
                    ui.window(left_x, y, left_w, h);
                    for (row, name) in hud.enemies.iter().take(shown).enumerate() {
                        let ty = y + lay.row_y + line * row as f32;
                        ui.text(left_x + lay.enemy_x, ty, name, WHITE);
                    }
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
        // The result screen (`BattleResultUI`): the info line in its own
        // window at the top, the page's lines in a window under it.
        if let Some((title, lines)) = &hud.results {
            let info_w = ui.measure(title) + 160.0;
            ui.window((ui_width - info_w) / 2.0, 40.0, info_w, 100.0);
            ui.text((ui_width - ui.measure(title)) / 2.0, 62.0, title, WHITE);
            let widest = lines
                .iter()
                .map(|entry| ui.measure(&entry.text) + ui.measure(&entry.right))
                .fold(0.0, f32::max);
            let w = (widest + 320.0).clamp(1100.0, ui_width - 2.0 * lay.margin);
            let h = 60.0 + line * lines.len() as f32;
            let (x, y) = ((ui_width - w) / 2.0, 170.0);
            ui.window(x, y, w, h);
            for (row, entry) in lines.iter().enumerate() {
                let ty = y + 30.0 + line * row as f32;
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
            if let Some(digits) = self.digits.as_ref() {
                ui.digits(digits, x / scale, y / scale - 40.0 - rise, &text, color);
                continue;
            }
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
        let mut texts = self.hud.texts.borrow_mut();
        // Damage numbers and gauges change the lines; keep the cache small.
        if texts.len() > 256 {
            texts.clear();
        }
        let image = texts
            .entry((text.to_owned(), color))
            .or_insert_with(|| self.text_image(text, color));
        let lift = self.hud.text_lift as f32;
        blit_scaled(self.frame, image, x, y - lift, self.scale);
    }

    /// A damage number in the PS1 battle digits, centered on `x` with its top at `y` (UI
    /// pixels), tinted by `color` (the digits are drawn white).
    fn digits(
        &mut self,
        digits: &crate::ui_skin::BattleDigits,
        x: f32,
        y: f32,
        text: &str,
        color: [u8; 3],
    ) {
        let ui = digits.ui_per_pixel;
        let glyphs: Vec<&Image> = if text == "Miss" {
            vec![&digits.miss]
        } else {
            text.chars()
                .filter_map(|ch| digits.digits.get(ch.to_digit(10)? as usize))
                .collect()
        };
        let width: f32 = glyphs
            .iter()
            .map(|glyph| glyph.width as f32 * ui)
            .sum::<f32>();
        let mut pen = x - width / 2.0;
        for glyph in glyphs {
            let mut tinted = glyph.clone();
            for px in tinted.rgba.chunks_exact_mut(4) {
                for (channel, tint) in px.iter_mut().zip(color) {
                    *channel = (u16::from(*channel) * u16::from(tint) / u16::from(WHITE[0]))
                        .min(255) as u8;
                }
            }
            blit_scaled(self.frame, &tinted, pen / ui, y / ui, self.scale * ui);
            pen += glyph.width as f32 * ui;
        }
    }

    fn text_image(&self, text: &str, color: [u8; 3]) -> Image {
        let glyphs = self.hud.assets.glyphs();
        let width = self.measure(text).ceil() as usize + 8;
        let lift = self.hud.text_lift;
        let height = (FONT_SIZE + 16 + lift) as usize;
        let mut image = Image::new(width.max(1), height);
        for (offset, shade) in [(4, SHADOW), (0, color)] {
            let mut pen = 0;
            for ch in text.chars() {
                let Some(glyph) = glyphs.glyph(ch) else {
                    continue;
                };
                let left = pen + glyph.min_x + offset;
                let top = self.hud.baseline - glyph.max_y() + offset + lift;
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
        image
    }

    fn text_right(&mut self, right: f32, y: f32, text: &str, color: [u8; 3]) {
        let width = self.measure(text);
        self.text(right - width, y, text, color);
    }

    /// The cursor hand pointing right, its tip at `(x, y + 18)` (the middle of a text row
    /// whose top is `y - 8`).
    fn hand(&mut self, x: f32, y: f32) {
        self.hand_at(x, y + 18.0);
    }

    /// The hand cursor pointing right with its fingertip at `(x, y)` (UI pixels): the sprite
    /// when it loaded, else a stand-in arrow.
    fn hand_at(&mut self, x: f32, y: f32) {
        let Some((hand, hand_width)) = self.hud.hand.as_ref() else {
            self.triangle(x, y, [[-34.0, -18.0], [-34.0, 18.0], [0.0, 0.0]]);
            return;
        };
        // UI pixels per texel, so the sprite is `PointerSize` wide; `blit_scaled` takes frame
        // pixels per source texel.
        let ui = hand_width / hand.width.max(1) as f32;
        let (w, h) = (hand.width as f32 * ui, hand.height as f32 * ui);
        blit_scaled(
            self.frame,
            hand,
            (x - w) / ui,
            (y - h / 2.0) / ui,
            self.scale * ui,
        );
    }

    /// The target hand: the same hand pointing right at the targeted unit, its fingertip at
    /// `(x, y)` above the unit.
    fn hand_down(&mut self, x: f32, y: f32) {
        self.hand_at(x, y);
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
    if let Some(recording) = frame.recording_mut() {
        recording.image(x, y, scale, &image.rgba, image.width, image.height);
        return;
    }
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
