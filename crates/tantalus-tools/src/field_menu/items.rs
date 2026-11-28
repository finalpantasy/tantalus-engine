//! The Items screen after Memoria's `ItemUI` (`Global/ItemUI.cs`): a Use / Arrange / Key Items
//! row, the two-column item list with icons and counts, the help line of the item under the
//! cursor, the target party panel when an item is used, and the Key Items list with its detail
//! card. Placement of the windows is by eye in PSX pixels (the prefab was not measured).

use super::*;
use tantalus_legacy::party_menu::item::can_toggle_multi;

/// `FF9FITEM_RARE_NONE` (`ItemUI.cs:45`): the placeholder row of an empty key item list.
pub(super) const KEY_NONE: i32 = 255;
/// Key item ids are item ids from 256 (`ff9item.FF9Item_Add_Generic`: `id % 1000` 256..511).
const KEY_BASE: i32 = 256;
/// `RegularItem.GysahlGreens` (`ItemUI.cs:323`): only usable on the world map near a chocobo.
const GYSAHL_GREENS: u16 = 251;
/// The Tent's field script; it spans the party.
const TENT_SCRIPT: u8 = 76;

const TABS: [&str; 3] = ["Use", "Arrange", "Key Items"];
const TAB_X: [f32; 3] = [20.0, 72.0, 136.0];
const ARRANGE_CHOICES: [&str; 2] = ["Auto", "Manual"];

const LEFT_X: f32 = 6.0;
const FULL_W: f32 = 308.0;
const TABS_Y: f32 = 8.0;
const LIST_Y: f32 = 34.0;
const LIST_LINES: usize = 8;
const CELL_W: f32 = 154.0;
const HELP_Y: f32 = 152.0;
const HELP_LINES: usize = 3;
/// Letters that fit one help line of the window.
const HELP_CHARS: usize = 40;
/// A cell's hand, icon, name and count positions from its left edge.
const CELL_HAND: f32 = 8.0;
const CELL_ICON: f32 = 20.0;
const CELL_NAME: f32 = 38.0;
const CELL_COUNT: f32 = 150.0;
const ICON_SIZE: f32 = 14.0;

/// What the cursor is on (`ButtonGroupState.ActiveGroup`: `SubMenuGroupButton`,
/// `ItemGroupButton`, `ArrangeMenuGroupButton`, `ItemArrangeGroupButton`,
/// `KeyItemGroupButton`, the key item card, `TargetGroupButton`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tabs,
    List,
    ArrangeMenu,
    /// Manual arrange: the item at `from` is picked up, the cursor chooses where it goes.
    Swap {
        from: usize,
    },
    Keys,
    /// `DisplayKeyItemSkin(true)`: the card with the key item's long text.
    KeyDetail,
    Target {
        cursor: usize,
        multi: bool,
        can_multi: bool,
    },
}

/// The Items screen's state; the item stacks themselves live in the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ItemState {
    /// `SubMenu`: 0 Use, 1 Arrange, 2 Key Items.
    tab: usize,
    focus: Focus,
    /// Index in the item list (row * 2 + column) and in the key item list.
    cursor: usize,
    key_cursor: usize,
    arrange: usize,
    /// `_currentArrangeMode`: 0 none, 1 auto, 2 manual.
    mode: u8,
    /// `_usedItemIdList`: stacks spent this visit stay listed at 0, grayed.
    used: Vec<u16>,
}

/// One row of the item list (`FieldItemListData`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Entry {
    pub id: u16,
    pub count: i32,
    /// `Enable`: the item type has the `Usable` bit (`ItemUI.DisplayItem`).
    pub enable: bool,
}

fn sort_key(tables: &MenuTables, id: i32) -> f32 {
    u16::try_from(id)
        .ok()
        .and_then(|id| tables.db.get(id))
        .map_or(0.0, |data| data.sort)
}

fn held(world: &FieldWorld, id: i32) -> i32 {
    world.items.get(&id).copied().unwrap_or(0)
}

/// Regular item ids (below `NoItem`, 255) with a stack, or spent this visit.
fn candidates(world: &FieldWorld, used: &[u16]) -> Vec<i32> {
    let mut ids: Vec<i32> = world
        .items
        .iter()
        .filter(|(id, count)| (0..i32::from(NO_ITEM)).contains(*id) && **count > 0)
        .map(|(id, _)| *id)
        .collect();
    for id in used {
        let id = i32::from(*id);
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// The stacks in list order: the arranged order (`FF9.item`) first, then the others by
/// `FF9ITEM_DATA.sort` (the order the game's own stacks start in is its insertion order, which
/// the world's id map does not keep).
fn ordered_ids(world: &FieldWorld, tables: &MenuTables, used: &[u16]) -> Vec<i32> {
    let live = candidates(world, used);
    let mut ids: Vec<i32> = Vec::new();
    for id in &world.item_order {
        if live.contains(id) && !ids.contains(id) {
            ids.push(*id);
        }
    }
    let mut rest: Vec<i32> = live.into_iter().filter(|id| !ids.contains(id)).collect();
    rest.sort_by(|a, b| {
        sort_key(tables, *a)
            .total_cmp(&sort_key(tables, *b))
            .then(a.cmp(b))
    });
    ids.extend(rest);
    ids
}

/// `ItemUI.DisplayItem`: a row per stack with a count, plus the stacks spent this visit.
pub(super) fn entries(world: &FieldWorld, tables: &MenuTables, used: &[u16]) -> Vec<Entry> {
    ordered_ids(world, tables, used)
        .into_iter()
        .filter_map(|id| {
            let id16 = u16::try_from(id).ok()?;
            Some(Entry {
                id: id16,
                count: held(world, id).max(0),
                enable: tables
                    .db
                    .get(id16)
                    .is_some_and(|data| data.kind & item_type::USABLE != 0)
                    && held(world, id) > 0,
            })
        })
        .collect()
}

/// `ItemUI.ArrangeAuto` (`ItemUI.cs:934-947`): the empty stacks go, the rest sort by
/// `FF9ITEM_DATA.sort`, ties by item id.
pub(super) fn arrange_auto(world: &mut FieldWorld, tables: &MenuTables) {
    let mut ids = candidates(world, &[]);
    ids.sort_by(|a, b| {
        sort_key(tables, *a)
            .total_cmp(&sort_key(tables, *b))
            .then(a.cmp(b))
    });
    world.item_order = ids;
}

/// The manual swap (`ItemUI.cs:459-461`): the two stacks trade places in the list.
pub(super) fn arrange_swap(
    world: &mut FieldWorld,
    tables: &MenuTables,
    used: &[u16],
    from: usize,
    to: usize,
) {
    let mut ids = ordered_ids(world, tables, used);
    if from < ids.len() && to < ids.len() {
        ids.swap(from, to);
        world.item_order = ids;
    }
}

/// `ItemUI.DisplayKeyItem`: the key items obtained (the source enumerates a `HashSet` in
/// insertion order; the world keeps ids ascending), or the one placeholder row.
pub(super) fn key_ids(world: &FieldWorld) -> Vec<i32> {
    let ids: Vec<i32> = world
        .items
        .iter()
        .filter(|(id, count)| (KEY_BASE..2 * KEY_BASE).contains(*id) && **count > 0)
        .map(|(id, _)| *id - KEY_BASE)
        .collect();
    if ids.is_empty() { vec![KEY_NONE] } else { ids }
}

/// The cursor after an arrow press in a two-column list (`ScrollItemKeyNavigation`): up and down
/// move a row, left and right a cell.
fn grid_step(cursor: usize, len: usize, pressed: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let mut next = cursor.min(len - 1);
    if pressed & pad::UP != 0 {
        if next >= 2 {
            next -= 2;
        }
    } else if pressed & pad::DOWN != 0 {
        if next + 2 < len {
            next += 2;
        } else if next / 2 < (len - 1) / 2 {
            next = len - 1;
        }
    } else if pressed & pad::LEFT != 0 {
        if next % 2 == 1 {
            next -= 1;
        }
    } else if pressed & pad::RIGHT != 0 && next.is_multiple_of(2) && next + 1 < len {
        next += 1;
    }
    next
}

/// Lines of at most `width` letters, each source line wrapped on its own.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        for word in line.split_whitespace() {
            if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        out.push(current);
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// `FF9TextTool.ItemHelpDescription` / `ImportantItemHelpDescription` for the row under the
/// cursor.
fn help_text(st: &ItemState, world: &FieldWorld, tables: &MenuTables) -> String {
    if st.showing_keys() {
        return key_ids(world)
            .get(st.key_cursor)
            .filter(|id| **id != KEY_NONE)
            .and_then(|id| tables.key_help.get(id))
            .cloned()
            .unwrap_or_default();
    }
    entries(world, tables, &st.used)
        .get(st.cursor)
        .and_then(|entry| tables.item_help.get(&i32::from(entry.id)))
        .cloned()
        .unwrap_or_default()
}

impl ItemState {
    /// `ItemUI.Show`: the cursor starts on the item list when there are items, else on the
    /// tabs (`Show_delay`).
    pub(super) fn open(world: &FieldWorld, tables: &MenuTables) -> Self {
        let has_items = !entries(world, tables, &[]).is_empty();
        Self {
            tab: 0,
            focus: if has_items { Focus::List } else { Focus::Tabs },
            cursor: 0,
            key_cursor: 0,
            arrange: 0,
            mode: 0,
            used: Vec::new(),
        }
    }

    fn showing_keys(&self) -> bool {
        matches!(self.focus, Focus::Keys | Focus::KeyDetail)
            || (self.tab == 2 && matches!(self.focus, Focus::Tabs))
    }
}

impl FieldMenu {
    /// One press on the Items screen (`ItemUI.OnKeyConfirm`, `OnKeyCancel`, `OnItemSelect`).
    pub(super) fn press_items(
        &mut self,
        mut st: ItemState,
        pressed: i32,
        world: &mut FieldWorld,
        tables: &MenuTables,
    ) {
        let confirm = pressed & (pad::CONFIRM | pad::CIRCLE) != 0;
        let cancel = pressed & (pad::CANCEL | pad::CROSS) != 0;
        let party = world.party.len().min(4);
        match st.focus {
            Focus::Tabs => {
                if pressed & pad::LEFT != 0 {
                    st.tab = st.tab.saturating_sub(1);
                } else if pressed & pad::RIGHT != 0 {
                    st.tab = (st.tab + 1).min(2);
                }
                if cancel {
                    self.message = None;
                    self.screen = Screen::Main;
                    return;
                }
                if confirm {
                    let list_len = entries(world, tables, &st.used).len();
                    match st.tab {
                        0 if list_len > 0 => {
                            st.mode = 0;
                            st.focus = Focus::List;
                        }
                        1 => {
                            st.focus = Focus::ArrangeMenu;
                            st.arrange = 0;
                        }
                        2 if key_ids(world)[0] != KEY_NONE => st.focus = Focus::Keys,
                        _ => self.refused = true,
                    }
                }
            }
            Focus::List => {
                let list = entries(world, tables, &st.used);
                st.cursor = grid_step(st.cursor, list.len(), pressed);
                if cancel {
                    self.message = None;
                    st.focus = Focus::Tabs;
                } else if confirm && let Some(entry) = list.get(st.cursor).copied() {
                    if st.mode == 2 {
                        st.focus = Focus::Swap { from: st.cursor };
                    } else {
                        self.start_item_use(&mut st, world, tables, entry, party);
                    }
                }
            }
            Focus::ArrangeMenu => {
                if pressed & (pad::UP | pad::DOWN) != 0 {
                    st.arrange = 1 - st.arrange;
                }
                if cancel {
                    st.mode = 0;
                    st.focus = Focus::Tabs;
                } else if confirm {
                    if st.arrange == 0 {
                        arrange_auto(world, tables);
                        st.cursor = 0;
                        st.mode = 1;
                        st.focus = Focus::Tabs;
                    } else {
                        st.mode = 2;
                        st.focus = if entries(world, tables, &st.used).is_empty() {
                            Focus::Tabs
                        } else {
                            Focus::List
                        };
                    }
                }
            }
            Focus::Swap { from } => {
                let len = entries(world, tables, &st.used).len();
                st.cursor = grid_step(st.cursor, len, pressed);
                if cancel {
                    st.cursor = from;
                    st.focus = Focus::List;
                } else if confirm {
                    arrange_swap(world, tables, &st.used, from, st.cursor);
                    st.focus = Focus::List;
                }
            }
            Focus::Keys => {
                let ids = key_ids(world);
                st.key_cursor = grid_step(st.key_cursor, ids.len(), pressed);
                if cancel {
                    st.focus = Focus::Tabs;
                } else if confirm {
                    if ids.get(st.key_cursor).is_some_and(|id| *id != KEY_NONE) {
                        st.focus = Focus::KeyDetail;
                    } else {
                        self.refused = true;
                    }
                }
            }
            Focus::KeyDetail => {
                if confirm || cancel {
                    // `DisplayKeyItemSkin(false)`: closing the card marks the key item used.
                    if let Some(id) = key_ids(world).get(st.key_cursor).copied() {
                        world.key_used.insert(id);
                    }
                    st.focus = Focus::Keys;
                }
            }
            Focus::Target {
                cursor,
                multi,
                can_multi,
            } => {
                let list = entries(world, tables, &st.used);
                let Some(entry) = list.get(st.cursor).copied() else {
                    st.focus = Focus::List;
                    self.screen = Screen::Items(st);
                    return;
                };
                let moved = if multi {
                    cursor
                } else {
                    step(cursor, party, pressed)
                };
                // `ToggleMultipleTarget` on L1 or R1.
                let now_multi = if pressed & (pad::L1 | pad::R1) != 0 && can_multi {
                    !multi
                } else {
                    multi
                };
                st.focus = Focus::Target {
                    cursor: moved,
                    multi: now_multi,
                    can_multi,
                };
                if cancel {
                    st.focus = Focus::List;
                } else if confirm {
                    if multi {
                        self.use_item_on_party(world, tables, entry.id);
                    } else if let Some(member) = world.party.get(moved).copied() {
                        self.use_item(world, tables, entry.id, member);
                    }
                    if held(world, i32::from(entry.id)) <= 0 {
                        st.used.push(entry.id);
                        st.focus = Focus::List;
                    }
                }
            }
        }
        self.screen = Screen::Items(st);
    }

    /// `ItemUI.OnKeyConfirm` on an item row in the Use list: the item needs an effect and the
    /// `Usable` type and must not be spent; then the target panel opens (the multi flag from
    /// the effect's target type: `All`, `AllAlly`, `AllEnemy` and `Everyone` start on the whole
    /// party, the `Many*` types can toggle with L1 or R1).
    fn start_item_use(
        &mut self,
        st: &mut ItemState,
        world: &FieldWorld,
        tables: &MenuTables,
        entry: Entry,
        party: usize,
    ) {
        let tent = tables
            .effects
            .get(&entry.id)
            .is_some_and(|effect| effect.script_id == TENT_SCRIPT);
        if !entry.enable
            || entry.count <= 0
            || !usable(tables, entry.id)
            || party == 0
            || entry.id == GYSAHL_GREENS
        {
            self.refused = true;
            return;
        }
        // A Tent is for save points only.
        if tent && !world.save_point {
            self.refused = true;
            self.message = Some("That cannot be used here.".into());
            return;
        }
        let target = tables.item_targets.get(&entry.id).map_or(0, |t| t.0);
        let all = matches!(target, 6..=8 | 12) || tent;
        self.message = None;
        st.focus = Focus::Target {
            cursor: 0,
            multi: all,
            can_multi: !tent && can_toggle_multi(target, party),
        };
    }

    /// The Items screen's windows (`ItemUI`).
    pub(super) fn items_frame(
        &self,
        st: &ItemState,
        frame: &mut MenuFrame,
        world: &FieldWorld,
        tables: &MenuTables,
    ) {
        // The tabs.
        let mut texts = Vec::new();
        for (index, label) in TABS.iter().enumerate() {
            let tint = if index == st.tab {
                Tint::Yellow
            } else {
                Tint::White
            };
            texts.push(at(TAB_X[index], 0.0, *label, tint, false));
        }
        frame
            .windows
            .push(window(LEFT_X, TABS_Y, FULL_W, 1.0, texts));
        let list_top = LIST_Y + 3.0;
        // The list under the tabs: items, or key items.
        if st.showing_keys() {
            self.key_list(st, frame, world, tables);
        } else {
            self.item_list(st, frame, world, tables, list_top);
        }
        // The help line (the target panel and the key item card cover it).
        if !matches!(st.focus, Focus::Target { .. } | Focus::KeyDetail) {
            let texts = wrap(&help_text(st, world, tables), HELP_CHARS)
                .into_iter()
                .take(HELP_LINES)
                .enumerate()
                .map(|(row, line)| at(6.0, row as f32 * ROW, line, Tint::White, false))
                .collect();
            frame
                .windows
                .push(window(LEFT_X, HELP_Y, FULL_W, HELP_LINES as f32, texts));
        }
        match st.focus {
            Focus::Tabs => {
                frame.pointer = Some((LEFT_X + TAB_X[st.tab] - HAND_GAP, TABS_Y + 3.0 + 0.5 * ROW));
            }
            Focus::ArrangeMenu => {
                let texts = ARRANGE_CHOICES
                    .iter()
                    .enumerate()
                    .map(|(row, label)| at(20.0, row as f32 * ROW, *label, Tint::White, false))
                    .collect();
                frame
                    .windows
                    .push(window(72.0, LIST_Y + 30.0, 90.0, 2.0, texts));
                frame.pointer = Some((
                    72.0 + 20.0 - HAND_GAP,
                    LIST_Y + 30.0 + 3.0 + (st.arrange as f32 + 0.5) * ROW,
                ));
            }
            Focus::List => {
                frame.pointer = Some(cell_point(st.cursor, list_top));
            }
            Focus::Swap { from } => {
                frame.pointer = Some(cell_point(st.cursor, list_top));
                if from != st.cursor {
                    frame.more_pointers.push(cell_point(from, list_top));
                }
            }
            Focus::Keys => {
                frame.pointer = Some(cell_point(st.key_cursor, list_top));
            }
            Focus::KeyDetail => self.key_card(st, frame, world, tables),
            Focus::Target { cursor, multi, .. } => {
                // `ItemUI.OnKeyConfirm`: an item in the left column slides the panel in from the
                // right, one in the right column from the left. The panel is the main screen's
                // party panel (portraits, name, level, HP and MP).
                let panel_x = panel_x(st);
                let (mut panel, mut faces) = main_party(world, tables);
                let (dx, dy) = (panel_x - panel.x, LIST_Y - panel.y);
                panel.x += dx;
                panel.y += dy;
                for face in &mut faces {
                    face.x += dx;
                    face.y += dy;
                }
                frame.windows.push(panel);
                frame.images.extend(faces);
                let party = world.party.len().min(4);
                let point = |member: usize| {
                    (
                        panel_x - 2.0,
                        LIST_Y + member as f32 * MEMBER_H + FACE_TOP + FACE_SIZE / 2.0,
                    )
                };
                if multi {
                    frame.pointer = Some(point(0));
                    frame.more_pointers.extend((1..party).map(point));
                } else {
                    frame.pointer = Some(point(cursor.min(party.saturating_sub(1))));
                }
            }
        }
    }

    fn item_list(
        &self,
        st: &ItemState,
        frame: &mut MenuFrame,
        world: &FieldWorld,
        tables: &MenuTables,
        list_top: f32,
    ) {
        let list = entries(world, tables, &st.used);
        let first = first_row(st.cursor);
        // The target panel covers part of the list; what is under it is not drawn (the windows
        // are see-through), a name cut short by it keeps the letters that fit.
        let covered = matches!(st.focus, Focus::Target { .. }).then(|| {
            let x = panel_x(st) - LEFT_X;
            (x, x + PARTY_W)
        });
        let mut texts = Vec::new();
        for (index, entry) in list.iter().enumerate() {
            let (row, column) = (index / 2, index % 2);
            if row < first || row >= first + LIST_LINES {
                continue;
            }
            let x = column as f32 * CELL_W;
            let y = (row - first) as f32 * ROW;
            let tint = if entry.enable { Tint::White } else { Tint::Dim };
            let clear =
                |from: f32, to: f32| covered.is_none_or(|(start, end)| to <= start || from >= end);
            let mut name = tables.item_name(entry.id);
            if let Some((start, end)) = covered {
                let name_x = x + CELL_NAME;
                if name_x >= start && name_x < end {
                    name.clear();
                } else if name_x < start {
                    name = name.chars().take(letters(start - name_x - 4.0)).collect();
                }
            }
            if !name.is_empty() {
                texts.push(at(x + CELL_NAME, y, name, tint, false));
            }
            if clear(x + CELL_COUNT - 14.0, x + CELL_COUNT) {
                texts.push(at(x + CELL_COUNT, y, entry.count.to_string(), tint, true));
            }
            if !clear(x + CELL_ICON, x + CELL_ICON + ICON_SIZE) {
                continue;
            }
            // `FF9UIDataTool.DisplayItem`: `item<shape>_<color>`, color 15 and half alpha when
            // the item cannot be used.
            let (shape, color) = tables.item_icons.get(&entry.id).copied().unwrap_or((0, 0));
            frame.images.push(MenuImage {
                x: LEFT_X + x + CELL_ICON,
                y: list_top + y,
                w: ICON_SIZE,
                h: ICON_SIZE,
                art: MenuArt::Icon {
                    shape,
                    color: if entry.enable { color } else { 15 },
                    dim: !entry.enable,
                },
            });
        }
        frame
            .windows
            .push(window(LEFT_X, LIST_Y, FULL_W, LIST_LINES as f32, texts));
    }

    fn key_list(
        &self,
        st: &ItemState,
        frame: &mut MenuFrame,
        world: &FieldWorld,
        tables: &MenuTables,
    ) {
        let ids = key_ids(world);
        let first = first_row(st.key_cursor);
        let mut texts = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            let (row, column) = (index / 2, index % 2);
            if *id == KEY_NONE || row < first || row >= first + LIST_LINES {
                continue;
            }
            let x = column as f32 * CELL_W;
            let y = (row - first) as f32 * ROW;
            let name = tables
                .key_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("Key item {id}"));
            texts.push(at(x + CELL_ICON, y, name, Tint::White, false));
            // `icon_new_exclamation`: shown until the key item's card has been opened.
            if !world.key_used.contains(id) {
                texts.push(at(x + CELL_COUNT, y, "!", Tint::Yellow, true));
            }
        }
        frame
            .windows
            .push(window(LEFT_X, LIST_Y, FULL_W, LIST_LINES as f32, texts));
    }

    /// `DisplayKeyItemSkin`: the key item's name and long text on a card over the list.
    fn key_card(
        &self,
        st: &ItemState,
        frame: &mut MenuFrame,
        world: &FieldWorld,
        tables: &MenuTables,
    ) {
        let id = key_ids(world)
            .get(st.key_cursor)
            .copied()
            .unwrap_or(KEY_NONE);
        let mut texts = vec![at(
            8.0,
            0.0,
            tables.key_names.get(&id).cloned().unwrap_or_default(),
            Tint::Yellow,
            false,
        )];
        let body = tables.key_skin.get(&id).cloned().unwrap_or_default();
        for (row, line) in wrap(&body, HELP_CHARS).into_iter().take(10).enumerate() {
            texts.push(at(8.0, (row as f32 + 1.5) * ROW, line, Tint::White, false));
        }
        frame
            .windows
            .push(window(LEFT_X, LIST_Y, FULL_W, 12.0, texts));
    }
}

/// Where the target panel stands: an item in the left column slides it in from the right, one
/// in the right column from the left (`ItemUI.OnKeyConfirm`).
fn panel_x(st: &ItemState) -> f32 {
    if st.cursor.is_multiple_of(2) {
        LEFT_X + FULL_W - PARTY_W
    } else {
        LEFT_X
    }
}

/// Letters of the PS1 font's average width that fit `width` pixels.
fn letters(width: f32) -> usize {
    (width / 7.0).max(0.0) as usize
}

/// The first list row on screen when the cursor is on `index`.
fn first_row(index: usize) -> usize {
    (index / 2).saturating_sub(LIST_LINES - 1)
}

/// Where the hand stands for list cell `index` (the list's top is `top`).
fn cell_point(index: usize, top: f32) -> (f32, f32) {
    let (row, column) = (index / 2, index % 2);
    (
        LEFT_X + column as f32 * CELL_W + CELL_HAND,
        top + ((row - first_row(index)) as f32 + 0.5) * ROW,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(tables: &mut MenuTables, id: u16, sort: f32, kind: u8) {
        tables.db.items.insert(
            id,
            ItemData {
                price: 0,
                selling_price: 0,
                equip: 0,
                kind,
                sort,
                eq_lv: 0.0,
                bonus: 0,
                ability: vec![],
                weapon_power: None,
                armor: None,
            },
        );
    }

    fn tables() -> MenuTables {
        let mut tables = MenuTables::default();
        // Sort values out of id order: 10 sorts before 3, 4 and 5 tie with 3.
        stack(&mut tables, 3, 20.0, item_type::ITEM | item_type::USABLE);
        stack(&mut tables, 4, 20.0, item_type::ITEM | item_type::USABLE);
        stack(&mut tables, 5, 20.0, item_type::WEAPON);
        stack(&mut tables, 10, 5.0, item_type::ITEM | item_type::USABLE);
        tables.item_names.insert(10, "Ten".into());
        tables.item_help.insert(10, "Heals ten\nlines apart".into());
        tables.key_names.insert(0, "Ticket".into());
        tables.key_help.insert(0, "A ticket.".into());
        tables.key_skin.insert(0, "A long card text.".into());
        tables
    }

    fn world() -> FieldWorld {
        let mut world = FieldWorld::new();
        world.party = vec![0];
        for id in [5, 3, 10, 4] {
            world.items.insert(id, 2);
        }
        world
    }

    fn ids(world: &FieldWorld, tables: &MenuTables, used: &[u16]) -> Vec<u16> {
        entries(world, tables, used).iter().map(|e| e.id).collect()
    }

    #[test]
    fn the_list_starts_in_sort_order_with_ties_by_id() {
        let (tables, world) = (tables(), world());
        assert_eq!(ids(&world, &tables, &[]), vec![10, 3, 4, 5]);
    }

    #[test]
    fn auto_arrange_sorts_by_sort_then_id_and_drops_empty_stacks() {
        let (tables, mut world) = (tables(), world());
        world.items.insert(4, 0);
        world.item_order = vec![5, 4, 3, 10];
        arrange_auto(&mut world, &tables);
        assert_eq!(world.item_order, vec![10, 3, 5]);
        assert_eq!(ids(&world, &tables, &[]), vec![10, 3, 5]);
        // An item picked up afterwards joins the end, as the game's stacks do.
        world.items.insert(4, 1);
        assert_eq!(ids(&world, &tables, &[]), vec![10, 3, 5, 4]);
    }

    #[test]
    fn the_manual_swap_trades_two_places() {
        let (tables, mut world) = (tables(), world());
        arrange_swap(&mut world, &tables, &[], 0, 3);
        assert_eq!(ids(&world, &tables, &[]), vec![5, 3, 4, 10]);
        arrange_swap(&mut world, &tables, &[], 1, 1);
        assert_eq!(ids(&world, &tables, &[]), vec![5, 3, 4, 10]);
        // Out of range does nothing.
        arrange_swap(&mut world, &tables, &[], 0, 9);
        assert_eq!(ids(&world, &tables, &[]), vec![5, 3, 4, 10]);
    }

    #[test]
    fn a_spent_stack_stays_listed_at_zero_and_grayed() {
        let (tables, mut world) = (tables(), world());
        world.items.remove(&3);
        let list = entries(&world, &tables, &[3]);
        let row = list.iter().find(|e| e.id == 3).unwrap();
        assert_eq!((row.count, row.enable), (0, false));
        assert_eq!(ids(&world, &tables, &[3]), vec![10, 3, 4, 5]);
        // Without the visit's list it is gone.
        assert_eq!(ids(&world, &tables, &[]), vec![10, 4, 5]);
    }

    #[test]
    fn a_weapon_is_listed_but_not_enabled() {
        let (tables, world) = (tables(), world());
        let list = entries(&world, &tables, &[]);
        assert!(list.iter().find(|e| e.id == 5).is_some_and(|e| !e.enable));
        assert!(list.iter().find(|e| e.id == 10).is_some_and(|e| e.enable));
    }

    #[test]
    fn key_items_list_the_obtained_ids_or_the_placeholder() {
        let mut world = FieldWorld::new();
        assert_eq!(key_ids(&world), vec![KEY_NONE]);
        world.items.insert(KEY_BASE + 7, 1);
        world.items.insert(KEY_BASE + 2, 1);
        world.items.insert(KEY_BASE + 9, 0);
        world.items.insert(3, 4);
        assert_eq!(key_ids(&world), vec![2, 7]);
    }

    #[test]
    fn the_help_line_is_the_text_of_the_item_or_key_item_under_the_cursor() {
        let (tables, mut world) = (tables(), world());
        let mut st = ItemState::open(&world, &tables);
        assert_eq!(st.focus, Focus::List);
        assert_eq!(help_text(&st, &world, &tables), "Heals ten\nlines apart");
        assert_eq!(
            wrap(&help_text(&st, &world, &tables), 40),
            vec!["Heals ten".to_owned(), "lines apart".to_owned()]
        );
        st.cursor = 1;
        assert_eq!(help_text(&st, &world, &tables), "");
        world.items.insert(KEY_BASE, 1);
        st.focus = Focus::Keys;
        assert_eq!(help_text(&st, &world, &tables), "A ticket.");
    }

    #[test]
    fn wrap_breaks_on_words_and_keeps_source_lines() {
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert_eq!(wrap("a\n\nb", 9), vec!["a", "", "b"]);
        assert!(wrap("", 9).is_empty());
    }

    #[test]
    fn the_grid_moves_a_row_for_up_and_down_and_a_cell_for_left_and_right() {
        assert_eq!(grid_step(0, 5, pad::DOWN), 2);
        assert_eq!(grid_step(2, 5, pad::DOWN), 4);
        assert_eq!(grid_step(3, 5, pad::DOWN), 4);
        assert_eq!(grid_step(4, 5, pad::DOWN), 4);
        assert_eq!(grid_step(2, 5, pad::UP), 0);
        assert_eq!(grid_step(0, 5, pad::UP), 0);
        assert_eq!(grid_step(0, 5, pad::RIGHT), 1);
        assert_eq!(grid_step(4, 5, pad::RIGHT), 4);
        assert_eq!(grid_step(1, 5, pad::LEFT), 0);
    }

    fn potion_tables() -> MenuTables {
        let mut tables = tables();
        tables.effects.insert(
            3,
            FieldEffect {
                script_id: 69,
                power: 10,
                status: 0,
            },
        );
        tables
    }

    #[test]
    fn using_an_item_opens_the_target_panel_and_uses_it_on_the_member() {
        use tantalus_game::field::world::CharacterStats;
        let tables = potion_tables();
        let mut world = world();
        world.character_stats.insert(
            0,
            CharacterStats {
                current_hp: 10,
                max_hp: 500,
                current_mp: 0,
                max_mp: 0,
            },
        );
        let mut menu = FieldMenu::new();
        menu.screen = Screen::Items(ItemState::open(&world, &tables));
        // Cursor on 10 (no effect): refused.
        menu.press(pad::CONFIRM, &mut world, &tables);
        assert!(menu.refused);
        // Move to item 3 (right of 10), confirm: the target panel with the party's portrait.
        menu.press(pad::RIGHT, &mut world, &tables);
        menu.press(pad::CONFIRM, &mut world, &tables);
        let frame = menu.frame(&world, &tables);
        assert_eq!(
            frame.images.last().map(|i| i.art),
            Some(MenuArt::Face {
                character: 0,
                dim: false
            })
        );
        assert!(frame.pointer.is_some());
        // Confirm on the member: it heals and the stack drops by one.
        menu.press(pad::CONFIRM, &mut world, &tables);
        assert_eq!(world.character_stats[&0].current_hp, 110);
        assert_eq!(world.items[&3], 1);
        // The second one empties the stack: it stays listed at 0, the cursor is back on the list.
        menu.press(pad::CONFIRM, &mut world, &tables);
        assert_eq!(world.items.get(&3).copied().unwrap_or(0), 0);
        let Screen::Items(st) = &menu.screen else {
            panic!("left the items screen");
        };
        assert_eq!((st.focus, st.used.clone()), (Focus::List, vec![3]));
        // Cancel: tabs, then the main screen.
        menu.press(pad::CANCEL, &mut world, &tables);
        menu.press(pad::CANCEL, &mut world, &tables);
        assert!(matches!(menu.screen, Screen::Main));
    }

    #[test]
    fn arrange_manual_picks_up_one_item_and_swaps_it() {
        let (tables, mut world) = (tables(), world());
        let mut menu = FieldMenu::new();
        menu.screen = Screen::Items(ItemState::open(&world, &tables));
        menu.press(pad::CANCEL, &mut world, &tables);
        // Tabs: right to Arrange, confirm opens the dialog, down to Manual, confirm.
        menu.press(pad::RIGHT, &mut world, &tables);
        menu.press(pad::CONFIRM, &mut world, &tables);
        menu.press(pad::DOWN, &mut world, &tables);
        menu.press(pad::CONFIRM, &mut world, &tables);
        // Pick the first (10), move one cell right (3), swap.
        menu.press(pad::CONFIRM, &mut world, &tables);
        menu.press(pad::RIGHT, &mut world, &tables);
        let frame = menu.frame(&world, &tables);
        assert_eq!(frame.more_pointers.len(), 1);
        menu.press(pad::CONFIRM, &mut world, &tables);
        assert_eq!(ids(&world, &tables, &[]), vec![3, 10, 4, 5]);
    }

    #[test]
    fn arrange_auto_from_the_menu_sorts_the_list() {
        let (tables, mut world) = (tables(), world());
        world.item_order = vec![5, 4, 3, 10];
        let mut menu = FieldMenu::new();
        menu.screen = Screen::Items(ItemState::open(&world, &tables));
        menu.press(pad::CANCEL, &mut world, &tables);
        menu.press(pad::RIGHT, &mut world, &tables);
        menu.press(pad::CONFIRM, &mut world, &tables);
        // The dialog opens on Auto.
        menu.press(pad::CONFIRM, &mut world, &tables);
        assert_eq!(world.item_order, vec![10, 3, 4, 5]);
    }

    #[test]
    fn the_key_item_card_opens_and_closing_it_clears_the_new_mark() {
        let (tables, mut world) = (tables(), world());
        world.items.insert(KEY_BASE, 1);
        let mut menu = FieldMenu::new();
        menu.screen = Screen::Items(ItemState::open(&world, &tables));
        menu.press(pad::CANCEL, &mut world, &tables);
        menu.press(pad::RIGHT, &mut world, &tables);
        menu.press(pad::RIGHT, &mut world, &tables);
        let marks = |frame: &MenuFrame| {
            frame
                .windows
                .iter()
                .flat_map(|w| w.texts.iter())
                .filter(|t| t.text == "!")
                .count()
        };
        assert_eq!(marks(&menu.frame(&world, &tables)), 1);
        menu.press(pad::CONFIRM, &mut world, &tables);
        menu.press(pad::CONFIRM, &mut world, &tables);
        let card = menu.frame(&world, &tables);
        assert!(
            card.windows
                .iter()
                .flat_map(|w| w.texts.iter())
                .any(|t| t.text == "A long card text.")
        );
        menu.press(pad::CONFIRM, &mut world, &tables);
        assert!(world.key_used.contains(&0));
        assert_eq!(marks(&menu.frame(&world, &tables)), 0);
    }

    #[test]
    fn the_real_help_texts_load_from_the_install() {
        let (Some(data), Some(memoria)) = (
            std::env::var_os("FFIX_GAME_DATA"),
            std::env::var_os("FFIX_MEMORIA"),
        ) else {
            return;
        };
        let source = std::fs::read_to_string(
            std::path::Path::new(&memoria)
                .join("Assembly-CSharp/Global/Event/Engine/EventEngineUtils.cs"),
        )
        .unwrap();
        let text = crate::field_data::FieldText::open(
            std::path::Path::new(&data),
            &source,
            "us".to_owned(),
        )
        .unwrap();
        // Item 236 is the Potion (`itm_help`), key item 0 the Silver Pendant.
        let help = text.menu_text("item/itm_help").unwrap();
        assert!(
            help[&236].starts_with("Restores 150 HP."),
            "{:?}",
            help[&236]
        );
        let names = text.menu_text("keyitem/imp_name").unwrap();
        assert_eq!(names[&0], "Silver Pendant");
        let key_help = text.menu_text("keyitem/imp_help").unwrap();
        assert!(key_help[&0].ends_with("key item."), "{:?}", key_help[&0]);
        assert!(key_help[&0].starts_with("Dagger"), "{:?}", key_help[&0]);
        let skin = text.menu_text("keyitem/imp_skin").unwrap();
        assert!(!skin[&0].is_empty());
    }
}
