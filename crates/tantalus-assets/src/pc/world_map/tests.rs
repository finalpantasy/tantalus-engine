use super::unity::analyze_world_map;
use super::*;
use crate::pc::serialized::SerializedFile;
use crate::pc::unity_raw::UnityRawBundle;
use std::path::Path;

fn le16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Builds a pack from segments; the last segment is padded with `padding` zeros.
fn pack(segments: &[Vec<u8>], padding: usize) -> Vec<u8> {
    let mut out = Vec::new();
    le32(&mut out, segments.len() as u32);
    let mut offset = 4 + 4 * segments.len() as u32;
    for segment in segments {
        le32(&mut out, offset);
        offset += segment.len() as u32;
    }
    for segment in segments {
        out.extend_from_slice(segment);
    }
    out.extend(std::iter::repeat_n(0, padding));
    out
}

fn tim(width_words: u16, height: u16, with_clut: bool) -> Vec<u8> {
    let mut out = Vec::new();
    le32(&mut out, 0x10);
    le32(&mut out, if with_clut { 8 | 1 } else { 2 });
    if with_clut {
        le32(&mut out, 12 + 16 * 2);
        for value in [0_u16, 480, 16, 1] {
            le16(&mut out, value);
        }
        out.extend(std::iter::repeat_n(0xAA, 32));
    }
    le32(
        &mut out,
        12 + u32::from(width_words) * u32::from(height) * 2,
    );
    for value in [320_u16, 0, width_words, height] {
        le16(&mut out, value);
    }
    out.extend(std::iter::repeat_n(
        0x55,
        usize::from(width_words) * usize::from(height) * 2,
    ));
    out
}

fn image(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut directory = Vec::new();
    let mut body = Vec::new();
    let mut sector = 1_u32;
    for entry in entries {
        let sectors = entry.len().div_ceil(SECTOR_BYTES) as u32;
        le32(&mut directory, sector);
        le32(&mut directory, sectors);
        sector += sectors;
        body.extend_from_slice(entry);
        body.extend(std::iter::repeat_n(
            0,
            sectors as usize * SECTOR_BYTES - entry.len(),
        ));
    }
    directory.resize(SECTOR_BYTES, 0);
    directory.extend(body);
    directory
}

#[test]
fn disc_image_directory_parses_and_checks_ranges() {
    let bytes = image(&[vec![1; 100], vec![2; 3000]]);
    let parsed = DiscImage::parse(&bytes, "t").expect("image");
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[1].start_sector, 2);
    assert_eq!(parsed.entries[1].sector_count, 2);
    assert_eq!(parsed.uncovered_sectors, 0);
    assert_eq!(parsed.entry(&bytes, 0).expect("slot 0").len(), SECTOR_BYTES);

    // An entry that runs past the file is rejected.
    let mut broken = bytes.clone();
    broken[4..8].copy_from_slice(&100_u32.to_le_bytes());
    assert!(DiscImage::parse(&broken, "t").is_err());
    // Not sector aligned.
    assert!(DiscImage::parse(&bytes[..bytes.len() - 1], "t").is_err());
}

#[test]
fn pack_rejects_bad_tables() {
    let good = pack(&[vec![1; 8], vec![2; 4]], 0);
    let parsed = Pack::parse(&good, "t").expect("pack");
    assert_eq!(parsed.count(), 2);
    assert_eq!(parsed.segment(0).expect("segment"), &[1; 8]);
    assert_eq!(parsed.segment(1).expect("last segment"), &[2; 4]);

    let mut descending = good.clone();
    descending[4..8].copy_from_slice(&20_u32.to_le_bytes());
    assert!(Pack::parse(&descending, "t").is_err());
    let mut wrong_start = good;
    wrong_start[4..8].copy_from_slice(&99_u32.to_le_bytes());
    assert!(Pack::parse(&wrong_start, "t").is_err());
    assert!(Pack::parse(&[0, 0, 0, 0], "t").is_err());
}

#[test]
fn tim_with_and_without_clut() {
    let with = tim(4, 3, true);
    let parsed = Tim::parse(&with, "t").expect("tim");
    assert_eq!(parsed.bits_per_pixel, 8);
    assert_eq!(parsed.clut, Some([0, 480, 16, 1]));
    assert_eq!(parsed.image, [320, 0, 4, 3]);
    assert_eq!(parsed.consumed_bytes, with.len());

    let without = tim(2, 2, false);
    let parsed = Tim::parse(&without, "t").expect("tim");
    assert_eq!(parsed.bits_per_pixel, 16);
    assert!(parsed.clut.is_none());

    // Block size that disagrees with the rectangle is an error.
    let mut broken = without.clone();
    broken[8..12].copy_from_slice(&0_u32.to_le_bytes());
    assert!(Tim::parse(&broken, "t").is_err());
    assert!(Tim::parse(&without[..without.len() - 1], "t").is_err());
}

#[test]
fn encounter_special_and_texture_project_sizes() {
    let mut encounters = Vec::new();
    for index in 0..ENCOUNTER_RECORDS as u16 {
        for scene in 0..4 {
            le16(&mut encounters, index + scene);
        }
        encounters.push((index % 7) as u8);
        encounters.push(0);
    }
    encounters.extend_from_slice(&[0, 0]);
    let (records, left) = parse_encounters(&encounters, "t").expect("encounters");
    assert_eq!(records.len(), 355);
    assert_eq!(records[10].scene, [10, 11, 12, 13]);
    assert_eq!(records[10].pattern, 3);
    assert_eq!(left, 2);
    assert!(parse_encounters(&encounters[..3000], "t").is_err());

    let mut special = Vec::new();
    for group in 0..9_u16 {
        for area in 0..12_u16 {
            le16(&mut special, if area == 0 { group + 1 } else { 0 });
        }
    }
    let (groups, left) = parse_special_encounters(&special, "t").expect("special");
    assert_eq!(groups.len(), 9);
    assert_eq!(groups[4][0], 5);
    assert_eq!(left, 0);
    // An area beyond the encounter table is rejected.
    special[0..2].copy_from_slice(&356_u16.to_le_bytes());
    assert!(parse_special_encounters(&special, "t").is_err());

    let project = vec![7_u8; 300];
    let (parsed, left) = parse_texture_project(&project, "t").expect("project");
    assert_eq!(
        (
            parsed.pixel_scroll.len(),
            parsed.pixel_anime.len(),
            parsed.palette_scroll.len()
        ),
        (8, 4, 18)
    );
    assert_eq!(parsed.pixel_scroll[0], [0x0707; 5]);
    assert_eq!(left, 0);
    assert!(parse_texture_project(&project[..299], "t").is_err());
}

fn effect(no: i16, x: i32) -> Vec<u8> {
    let mut out = Vec::new();
    for value in [x, 2, 3, 4, 5, 6] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in [1_i16, 2, 3, 4, 5, 6, no, 7, 8, 9] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend(std::iter::repeat_n(0, 20));
    assert_eq!(out.len(), EFFECT_RECORD_BYTES);
    out
}

#[test]
fn effect_list_stops_at_terminator() {
    let mut bytes = effect(3, 100);
    bytes.extend(effect(8, -5));
    bytes.extend(effect(-1, 0));
    bytes.extend_from_slice(&[0; 5]);
    let (list, left) = parse_effect_list(&bytes, "t").expect("list");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].position, [100, 2, 3]);
    assert_eq!(list[1].effect, 8);
    assert_eq!(list[1].size, 7);
    assert_eq!(left, 5);

    // Terminator only: an empty list (as in packs 44 and 52).
    let (empty, left) = parse_effect_list(&effect(-1, 0), "t").expect("empty list");
    assert!(empty.is_empty());
    assert_eq!(left, 0);
    // Missing terminator runs off the end.
    assert!(parse_effect_list(&effect(3, 1), "t").is_err());
}

/// One NWB block with two cells of a single triangle each and a texture region.
fn nwb_block(texture: &[u8]) -> Vec<u8> {
    let header = 8 + 4 * 2;
    let cell0 = header;
    let cell1 = cell0 + NWB_CELL_HEADER_BYTES + 3 * 8 + NWB_SURFACE_BYTES;
    let texture_offset = cell1 + NWB_CELL_HEADER_BYTES + 3 * 8 + NWB_SURFACE_BYTES;
    let mut out = vec![2_u8, 1, 0, 0];
    le32(&mut out, texture_offset as u32);
    le32(&mut out, cell0 as u32);
    le32(&mut out, cell1 as u32);
    for cell in [cell0, cell1] {
        out.push(3);
        out.push(9);
        le16(&mut out, 1);
        le32(&mut out, (cell + NWB_CELL_HEADER_BYTES) as u32);
        le32(&mut out, (cell + NWB_CELL_HEADER_BYTES + 24) as u32);
        out.extend(std::iter::repeat_n(0x11, 20));
        out.push(17);
        out.extend_from_slice(&[1, 2, 3]);
        for vertex in 0..3_i16 {
            for value in [vertex * 100, 0, -vertex, 390] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.extend_from_slice(&[
            0, 1, 2, 1, 10, 20, 0x51, 0x3d, 30, 40, 0xba, 0, 50, 60, 0xe4, 0,
        ]);
    }
    assert_eq!(out.len(), texture_offset);
    out.extend_from_slice(texture);
    out
}

#[test]
fn nwb_block_parses_cells_and_accounts_bytes() {
    let bytes = nwb_block(&[0; 64]);
    let block = NwbBlock::parse(&bytes, "t").expect("block");
    assert_eq!(block.cell_count, 2);
    assert_eq!(block.cells.len(), 2);
    assert_eq!(block.cells[0].vertices[2], [200, 0, -2, 390]);
    assert_eq!(block.cells[1].surfaces[0].vertex, [0, 1, 2]);
    assert_eq!(
        block.cells[1].surfaces[0].uv,
        [[10, 20], [30, 40], [50, 60]]
    );
    assert_eq!(block.cells[1].surfaces[0].tail, [0x3d51, 0x00ba, 0x00e4]);
    assert_eq!(block.texture_region_bytes, 64);
    assert_eq!(block.texture_region_nonzero, 0);
    assert_eq!(
        block.structured_bytes + block.texture_region_bytes,
        bytes.len()
    );
}

#[test]
fn nwb_block_rejects_bad_indices_and_ranges() {
    let mut bytes = nwb_block(&[0; 16]);
    // Surface vertex index 3 with only 3 vertices.
    let first_surface = 16 + NWB_CELL_HEADER_BYTES + 24;
    bytes[first_surface + 2] = 3;
    assert!(NwbBlock::parse(&bytes, "t").is_err());

    let mut bytes = nwb_block(&[0; 16]);
    // Cell offset beyond the buffer.
    bytes[8..12].copy_from_slice(&9999_u32.to_le_bytes());
    assert!(NwbBlock::parse(&bytes, "t").is_err());

    let mut bytes = nwb_block(&[0; 16]);
    // Negative cell count.
    bytes[0] = 0xFF;
    assert!(NwbBlock::parse(&bytes, "t").is_err());
}

fn frame_pack_fixture() -> Vec<u8> {
    let mut segments: Vec<Vec<u8>> = (0..SEA_BLOCK_PACK).map(|_| vec![0x5A; 8]).collect();
    let mut encounters = vec![0_u8; ENCOUNTER_RECORDS * ENCOUNTER_RECORD_BYTES];
    encounters.extend_from_slice(&[0, 0]);
    segments[ENCOUNTER_PACK] = encounters;
    segments[SPECIAL_ENCOUNTER_PACK] = vec![0; 216];
    segments[TEXTURE_PROJECT_PACK] = vec![0; 300];
    for index in EFFECT_LIST_PACKS {
        let mut list = effect(1, 5);
        list.extend(effect(-1, 0));
        segments[index] = list;
    }
    segments.push(nwb_block(&[0; 32]));
    pack(&segments, 0)
}

#[test]
fn frame_pack_asset_accounts_every_byte() {
    let bytes = frame_pack_fixture();
    let report = analyze_wmap_asset("x/fx.ntp.bytes", &bytes);
    assert!(report.parsed_ok, "{:?}", report.failures);
    assert_eq!(report.kind, "frame_pack");
    assert_eq!(report.counts["encounter_records"], 355);
    assert_eq!(report.counts["effect_lists"], 12);
    assert_eq!(report.counts["effect_records"], 12);
    assert_eq!(report.counts["nwb_cells"], 2);
    assert_eq!(report.account.total(), bytes.len());
    assert_eq!(report.account.unexplained, 0);
}

#[test]
fn disc_image_asset_and_tim_pack_report() {
    let tims = pack(&[tim(4, 3, true), tim(2, 2, false)], 100);
    let raw = vec![9_u8; 500];
    let bytes = image(&[tims.clone(), frame_pack_fixture(), raw]);
    let report = analyze_wmap_asset("x/discmr.img.bytes", &bytes);
    assert!(report.parsed_ok, "{:?}", report.failures);
    assert_eq!(report.kind, "disc_image");
    assert_eq!(report.counts["tim_images"], 2);
    assert_eq!(report.counts["nwb_cells"], 2);
    assert_eq!(report.account.total(), bytes.len());
    assert_eq!(report.account.unexplained, 0);
    assert!(report.account.raw_named >= 500);

    // Non-zero bytes after a TIM are unexplained, not silently ignored.
    let mut dirty = pack(&[tim(4, 3, true)], 10);
    let last = dirty.len() - 1;
    dirty[last] = 0xEE;
    let report = analyze_wmap_asset("x/static.ntp.bytes", &dirty);
    assert_eq!(report.account.unexplained, 10);
    assert!(!report.parsed_ok);
}

#[test]
fn map_bin_is_a_24_by_20_grid() {
    let mut bytes = vec![0_u8; MAP_GRID_WIDTH * MAP_GRID_HEIGHT];
    bytes[5] = 16;
    let report = analyze_wmap_asset("x/map.bin.bytes", &bytes);
    assert!(report.parsed_ok);
    assert_eq!(report.counts["max_value"], 16);
    let short = analyze_wmap_asset("x/map.bin.bytes", &bytes[..100]);
    assert!(!short.parsed_ok);
}

fn p0data3() -> Option<UnityRawBundle> {
    let path =
        Path::new("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets/p0data3.bin");
    if !path.is_file() {
        eprintln!("skipping: FFIX install not present");
        return None;
    }
    Some(UnityRawBundle::open(path).expect("open p0data3"))
}

#[test]
fn real_p0data3_world_map_is_fully_classified_and_parsed() {
    let Some(bundle) = p0data3() else {
        return;
    };
    let bytes = bundle.read_entry(0, 600 * 1024 * 1024).expect("entry");
    let file = SerializedFile::parse(&bytes, "p0data3").expect("serialized");
    let report = analyze_world_map(&file, "p0data3").expect("report");
    assert_eq!(report.container_entries, 3439);
    assert_eq!(
        report.unclassified_entries, 0,
        "{:?}",
        report.unclassified_samples
    );
    assert_eq!(report.classified_entries, 3439);
    assert_eq!(report.block_meshes_failed, 0, "{:?}", report.failures);
    assert_eq!(report.block_meshes_decoded, 2178);
    assert_eq!(report.block_mesh_row_mismatches, 0);
    assert_eq!(report.block_mesh_name_path_mismatches, 0);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.wmap_assets.len(), 25);
    assert_eq!(report.wmap_failures, 0);
    assert_eq!(report.wmap_bytes_unexplained, 0);
    let frame = report
        .wmap_assets
        .iter()
        .find(|asset| asset.path.ends_with("disc1/fx.ntp.bytes"))
        .expect("frame pack");
    assert_eq!(frame.counts["encounter_records"], 355);
    assert_eq!(frame.counts["nwb_cells"], 4);
    assert_eq!(frame.counts["effect_lists"], 12);
    assert_eq!(report.discs.len(), 2);
    for disc in &report.discs {
        // Every prefab slot points at its own cell's mesh or the shared sea mesh.
        assert_eq!(
            disc.prefab_slots,
            disc.prefab_slots_matching_block + disc.prefab_slots_shared_sea_mesh,
            "disc {}",
            disc.disc
        );
        assert_eq!(disc.block_prefabs, 480, "disc {}", disc.disc);
        assert_eq!(disc.prefab_slots_shared_sea_mesh, 410, "disc {}", disc.disc);
        // The unreferenced meshes are all form-2 variants.
        assert_eq!(disc.meshes_not_in_any_prefab, 38, "disc {}", disc.disc);
        assert!(
            disc.unreferenced_mesh_kinds
                .keys()
                .all(|key| key.starts_with("form2"))
        );
        assert_eq!(disc.prefab_initial_xy_mismatch, 0, "disc {}", disc.disc);
    }
    // map.bin is zero for every sea cell.
    let disc1 = &report.discs[0];
    assert_eq!(disc1.map_values_for_sea_cells.len(), 1);
    assert_eq!(disc1.map_values_for_sea_cells[&0], 205);
}
