//! The loader's record handlers on synthetic records (the DLL comparison is
//! in `oracle.rs`).

use tantalus_sfx::loader::{Loader, RecordData};
use tantalus_sfx::vram::{Rect, Vram};

fn le32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn le16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// A type 0 record: one 2x1 image, then a table with one page entry of
/// height 256 (used twice) and two 16 KB pages, as `SfxLoaderStepImages`
/// and `SfxUploadTexturePages` read them.
#[test]
fn images_then_texture_pages() {
    let mut record = Vec::new();
    le32(&mut record, 24); // table offset
    le32(&mut record, 12); // image list
    le32(&mut record, 1); // image count
    for word in [10u16, 20, 2, 1, 0x1111, 0x2222] {
        le16(&mut record, word);
    }
    // Table at 24: pixel offset, pad, then the page entry.
    le32(&mut record, 40);
    le32(&mut record, 0);
    for word in [128u16, 256, 64, 256] {
        le16(&mut record, word);
    }
    record.resize(40, 0);
    record.extend(std::iter::repeat_n(0xAB, 0x4000));
    record.extend(std::iter::repeat_n(0xCD, 0x4000));
    let uploads = Loader::default().record(RecordData {
        record_type: 0,
        flag: 2,
        data: &record,
    });
    let rects: Vec<Rect> = uploads.iter().map(|upload| upload.rect).collect();
    assert_eq!(
        rects,
        vec![
            Rect {
                x: 10,
                y: 20,
                w: 2,
                h: 1
            },
            Rect {
                x: 128,
                y: 256,
                w: 64,
                h: 128
            },
            Rect {
                x: 128,
                y: 384,
                w: 64,
                h: 128
            },
        ]
    );
    assert_eq!(uploads[0].pixels, vec![0x1111, 0x2222]);
    assert!(uploads[1].pixels.iter().all(|&p| p == 0xABAB));
    assert!(uploads[2].pixels.iter().all(|&p| p == 0xCDCD));
}

/// Type 9: flag bit 0 selects pages 0 and 1 (x 768, y 256 and 384), bit 2
/// page 4 (x 320 - ...).
#[test]
fn vram_pages_follow_the_flag_bits() {
    let data = vec![0u8; 3 * 0x4000];
    let mut loader = Loader::default();
    let uploads = loader.record(RecordData {
        record_type: 9,
        flag: 0b101,
        data: &data,
    });
    let rects: Vec<(i16, i16)> = uploads.iter().map(|u| (u.rect.x, u.rect.y)).collect();
    assert_eq!(rects, vec![(768, 256), (768, 384), (320, 256)]);
    assert!(loader.touched_effect_area);
}

/// A 4-bit page samples its CLUT; index 0 of a zero CLUT is transparent.
#[test]
fn four_bit_page_reads_the_clut() {
    let mut vram = Vram::default();
    vram.load_image(
        Rect {
            x: 0,
            y: 240,
            w: 2,
            h: 1,
        },
        &[0, 0x7FFF],
    );
    vram.load_image(
        Rect {
            x: 640,
            y: 0,
            w: 1,
            h: 1,
        },
        &[0x0010],
    );
    let (width, rgba) = vram.page_rgba(640, 0, 0, (0, 240));
    assert_eq!(width, 256);
    assert_eq!(rgba[..4], [0, 0, 0, 0]);
    assert_eq!(rgba[4..8], [255, 255, 255, 255]);
}
