//! The Chocograph list's cursor and frame, on a book built from event
//! global bytes (no install needed).
use std::collections::BTreeMap;

use tantalus_legacy::chocobo::ChocographBook;
use tantalus_legacy::saves::ff9_tree::EventGlobal;
use tantalus_tools::chocograph_menu::{ChocographEvent, ChocographScreen};
use tantalus_tools::play::pad;

fn screen(found: u32, opened: u32, reach: u8, hintmap: i32) -> ChocographScreen {
    let mut bytes = vec![0u8; EventGlobal::LEN];
    for index in 0..3 {
        bytes[187 + index] = (found >> (8 * index)) as u8;
        bytes[184 + index] = (opened >> (8 * index)) as u8;
    }
    bytes[191] = reach;
    let global = EventGlobal::from_bytes(bytes).unwrap();
    let texts = BTreeMap::from([(10, "Streamside".to_owned()), (16, "Fourth".to_owned())]);
    ChocographScreen::new(ChocographBook::from_event(&global, hintmap), texts)
}

fn text_of(frame: &tantalus_tools::field_menu::MenuFrame, window: usize) -> Vec<String> {
    frame.windows[window]
        .texts
        .iter()
        .map(|run| run.text.clone())
        .collect()
}

#[test]
fn select_a_picture_stores_the_hint_map_and_cancel_clears_it() {
    // Pictures 0 and 12 found, picture 0 opened, a chocobo of reach 2
    // (Field and Reef; Mountain pictures need more).
    let mut screen = screen(0x1001, 1, 2, 0);
    assert_eq!(screen.rows(), [0, 12]);
    // The sub menu: Select enters the list.
    assert_eq!(screen.press(pad::CONFIRM), ChocographEvent::Confirmed);
    assert_eq!(screen.press(pad::DOWN), ChocographEvent::Moved);
    // Picture 12 is out of reach of that chocobo.
    assert_eq!(screen.press(pad::CONFIRM), ChocographEvent::Refused);
    assert_eq!(screen.book.hintmap_id, 0);
    assert_eq!(screen.press(pad::UP), ChocographEvent::Moved);
    assert_eq!(screen.press(pad::CONFIRM), ChocographEvent::Confirmed);
    assert_eq!(screen.book.hintmap_id, 1);
    let frame = screen.frame();
    assert!(text_of(&frame, 0)[0].contains("Streamside"));
    assert_eq!(
        text_of(&frame, 2),
        ["Streamside", "Opened", "Chocograph 13"]
    );
    // Cancel key back to the sub menu, then its Cancel row clears the pick.
    assert_eq!(screen.press(pad::CANCEL), ChocographEvent::Cancelled);
    assert_eq!(screen.press(pad::DOWN), ChocographEvent::Moved);
    assert_eq!(screen.press(pad::CONFIRM), ChocographEvent::Cancelled);
    assert_eq!(screen.book.hintmap_id, 0);
    // Nothing left to clear: refused. The cancel key leaves the list.
    assert_eq!(screen.press(pad::CONFIRM), ChocographEvent::Refused);
    assert_eq!(screen.press(pad::CANCEL), ChocographEvent::Closed);
}

#[test]
fn a_list_without_pictures_refuses_select() {
    let mut screen = screen(0, 0, 2, 0);
    assert_eq!(screen.press(pad::CONFIRM), ChocographEvent::Refused);
    assert_eq!(text_of(&screen.frame(), 2), ["No Chocograph found"]);
}
