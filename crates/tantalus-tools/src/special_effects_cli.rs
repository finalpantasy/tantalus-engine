//! `tantalusctl assets inspect-bundle --path <bundle> --special-effects <out json>`: parses every
//! `ef<id>` TextAsset in `resources.assets` with `tantalus_assets::pc::special_effect` and maps
//! the effect ids the battle tables reference to those blobs. Rendering and the effect scripts
//! belong to the native plugin and are not decoded.

use crate::battle_models_cli::{effect_references, file_name, install_file};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tantalus_assets::pc::battle_models::{build_effect_report, special_effect_name};
use tantalus_assets::pc::battle_tables::extract_battle_tables;
use tantalus_assets::pc::managed_il::Assembly;
use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
use tantalus_assets::pc::special_effect::{EffectFile, RecordDetail, SECTOR};

/// `SFX.playParam.Length` in the pinned reference: `SFX.Play` replaces larger ids with `Fire__Multi`.
const PLAY_PARAM_LENGTH: i32 = 511;

pub struct SpecialEffectsRequest {
    pub dll: Option<PathBuf>,
    pub resources_assets: Option<PathBuf>,
    pub out: PathBuf,
}

#[derive(Serialize)]
struct EffectEntry {
    id: i32,
    asset_name: String,
    enum_name: Option<&'static str>,
    sha256: String,
    referenced_by: usize,
    parsed: Option<EffectFile>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ReferencedEffect {
    effect_id: i32,
    enum_name: Option<&'static str>,
    asset_present: bool,
    asset_bytes: Option<usize>,
    referenced_by: usize,
    sources: Vec<String>,
}

#[derive(Serialize)]
struct MissingEffect {
    effect_id: i32,
    enum_name: Option<&'static str>,
    referenced_by: usize,
    sources: Vec<String>,
    reason: String,
}

#[derive(Default, Serialize)]
struct Totals {
    assets: usize,
    parsed: usize,
    failed: usize,
    sizes_multiple_of_sector: usize,
    distinct_referenced: usize,
    referenced_present: usize,
    referenced_missing: usize,
    records: usize,
    record_types: BTreeMap<String, usize>,
    parsed_bytes: usize,
    raw_bytes: usize,
    total_bytes: usize,
    small_file_tables: usize,
    camera_files: usize,
    camera_files_with_unknown_blocks: usize,
    external_slots: usize,
    script_records: usize,
    script_offsets_in_record: usize,
    script_offsets_total: usize,
    image_records: usize,
    image_rects: usize,
    sequence_ops: usize,
    unnamed_sequence_ops: BTreeMap<String, usize>,
}

#[derive(Serialize)]
pub struct Report {
    schema: &'static str,
    sources: Sources,
    format: Vec<&'static str>,
    totals: Totals,
    referenced: Vec<ReferencedEffect>,
    missing: Vec<MissingEffect>,
    effects: Vec<EffectEntry>,
    not_decoded: Vec<&'static str>,
}

#[derive(Serialize)]
struct Sources {
    resources_assets: String,
    assembly: Option<String>,
    assembly_sha256: Option<String>,
    reference: &'static str,
}

type Blobs = BTreeMap<i32, (String, Vec<u8>)>;

fn read_blobs(path: &Path) -> Result<Blobs, String> {
    let mut file = StandaloneSerialized::open(path).map_err(|e| e.to_string())?;
    let assets = file.text_assets().map_err(|e| e.to_string())?;
    let mut blobs = BTreeMap::new();
    for (name, object) in assets {
        let Some(digits) = name.strip_prefix("ef") else {
            continue;
        };
        if digits.len() != 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(id) = digits.parse::<i32>() else {
            continue;
        };
        let asset = file
            .read_text_asset(&object)
            .map_err(|e| format!("{name}: {e}"))?;
        blobs.insert(id, (name, asset.script));
    }
    Ok(blobs)
}

fn tally(totals: &mut Totals, file: &EffectFile) {
    totals.parsed += 1;
    totals.parsed_bytes += file.parsed_bytes;
    totals.raw_bytes += file.raw_bytes;
    totals.sequence_ops += file.sequence.len();
    for op in file.sequence.iter().filter(|op| op.name.is_none()) {
        *totals
            .unnamed_sequence_ops
            .entry(format!("0x{:02X}", op.code))
            .or_default() += 1;
    }
    for record in &file.records {
        totals.records += 1;
        *totals
            .record_types
            .entry(record.record_type_name.to_owned())
            .or_default() += 1;
        match &record.detail {
            RecordDetail::SmallFiles(table) => {
                totals.small_file_tables += 1;
                totals.external_slots += table.external.len();
                for camera in table.files.iter().filter_map(|f| f.camera.as_ref()) {
                    totals.camera_files += 1;
                    if camera.sequences.iter().any(|s| s.aborted_unknown_block) {
                        totals.camera_files_with_unknown_blocks += 1;
                    }
                }
            }
            RecordDetail::Script(script) => {
                totals.script_records += 1;
                if let Some(n) = script.offsets_in_record {
                    totals.script_offsets_in_record += n;
                    totals.script_offsets_total += 16;
                }
            }
            RecordDetail::Images(list) => {
                totals.image_records += 1;
                totals.image_rects += list.images.len();
            }
            RecordDetail::Raw => {}
        }
    }
}

pub fn build_report(battle_path: &Path, request: &SpecialEffectsRequest) -> Result<Report, String> {
    let resources = request
        .resources_assets
        .clone()
        .or_else(|| install_file(battle_path, "x64/FF9_Data/resources.assets"))
        .ok_or_else(|| "no resources.assets: pass --resources-assets <path>".to_owned())?;
    let dll = request
        .dll
        .clone()
        .or_else(|| install_file(battle_path, "x64/FF9_Data/Managed/Assembly-CSharp.dll"));
    let mut assembly_sha = None;
    let tables = match &dll {
        Some(dll) => {
            let bytes = std::fs::read(dll)
                .map_err(|error| format!("cannot read {}: {error}", dll.display()))?;
            assembly_sha = Some(format!("{:x}", Sha256::digest(&bytes)));
            let assembly = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
            Some(extract_battle_tables(&assembly).map_err(|e| e.to_string())?)
        }
        None => None,
    };
    let references = effect_references(tables.as_ref());
    let blobs = read_blobs(&resources)?;
    let sizes: BTreeMap<i32, usize> = blobs.iter().map(|(id, (_, b))| (*id, b.len())).collect();
    let coverage = build_effect_report(&references, &sizes);

    let mut sources_by_id: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    for reference in references.iter().filter(|r| r.effect_id >= 0) {
        sources_by_id
            .entry(reference.effect_id)
            .or_default()
            .push(reference.source.clone());
    }
    let counts: BTreeMap<i32, usize> = coverage
        .effects
        .iter()
        .map(|e| (e.effect_id, e.referenced_by))
        .collect();

    let mut totals = Totals {
        assets: blobs.len(),
        distinct_referenced: coverage.totals.distinct_effects,
        referenced_present: coverage.totals.distinct_effects_present,
        referenced_missing: coverage.totals.distinct_effects
            - coverage.totals.distinct_effects_present,
        ..Totals::default()
    };
    let mut effects = Vec::new();
    for (id, (name, bytes)) in &blobs {
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        if bytes.len() % SECTOR == 0 {
            totals.sizes_multiple_of_sector += 1;
        }
        totals.total_bytes += bytes.len();
        let outcome = EffectFile::parse(bytes, name).and_then(|mut file| {
            file.resolve_small_files(bytes, name)?;
            Ok(file)
        });
        let (parsed, error) = match outcome {
            Ok(file) => {
                tally(&mut totals, &file);
                (Some(file), None)
            }
            Err(error) => {
                totals.failed += 1;
                (None, Some(error.to_string()))
            }
        };
        effects.push(EffectEntry {
            id: *id,
            asset_name: name.clone(),
            enum_name: special_effect_name(*id),
            sha256,
            referenced_by: counts.get(id).copied().unwrap_or(0),
            parsed,
            error,
        });
    }

    let referenced = coverage
        .effects
        .iter()
        .map(|e| ReferencedEffect {
            effect_id: e.effect_id,
            enum_name: e.enum_name,
            asset_present: e.asset_present,
            asset_bytes: e.asset_bytes,
            referenced_by: e.referenced_by,
            sources: sources_by_id.get(&e.effect_id).cloned().unwrap_or_default(),
        })
        .collect();
    let missing = coverage
        .effects
        .iter()
        .filter(|e| !e.asset_present)
        .map(|e| {
            let reason = if e.effect_id >= PLAY_PARAM_LENGTH {
                format!(
                    "id is not below SFX.playParam.Length ({PLAY_PARAM_LENGTH}): `SFX.Play` replaces it with Fire__Multi"
                )
            } else {
                "no `ef<id>` TextAsset in resources.assets; `SFX.SetEffectArgs` passes a null buffer and size 0 to `SFX_Play` when `AssetManager.LoadBytes` returns null, so the plugin starts the effect with no loaded container (what it does then is native code)".to_owned()
            };
            MissingEffect {
                effect_id: e.effect_id,
                enum_name: e.enum_name,
                referenced_by: e.referenced_by,
                sources: sources_by_id.get(&e.effect_id).cloned().unwrap_or_default(),
                reason,
            }
        })
        .collect();

    Ok(Report {
        schema: "tantalus.special-effects.v1",
        sources: Sources {
            resources_assets: file_name(&resources),
            assembly: dll.as_deref().map(file_name),
            assembly_sha256: assembly_sha,
            reference: "Memoria c716e1fa",
        },
        format: vec![
            "sector 0 (0x800 bytes): i16 group count, per group {i16 kind, i16 entry count, entries {u8 type, u8 flag, i16 sectors; type 2 with a nonzero flag adds an i16 extra sector count}} (u0068_SfxIndexChunkTable.cs); the 3-byte sequence stream starts at 0x400 and ends at op 0 (SFXBinaryFile.cs)",
            "records follow from sector 1 in directory order, sectors * 0x800 bytes each; a type 2 record with extra sectors stores the extra sectors first",
            "type 0 images: u32 table offset, u32 image list offset, u32 count, RECT x y w h + w*h u16 pixels each, page table of `flag` 8-byte entries at table + 8 (u0070_SfxLoaderStep.cs, u0067)",
            "type 2 small files: (u16 offset, u16 flags) slots, equal offsets share a file, flags 0xFFFF marks external slots; cameras (SFXDataCamera.cs) and sounds are found through sequence ops 0x23, 0x29 and 0x2D",
            "type 3 script: head u32 PSX address rebased by 0x801E7700 (+0x5000 for odd groups) locates 16 u32 addresses; the script body is MIPS code run by the plugin and stays raw",
            "types 1, 4, 5, 6, 7, 8, 9, 10: kept raw with type name, flag and size",
        ],
        totals,
        referenced,
        missing,
        effects,
        not_decoded: vec![
            "Script bodies (type 3): MIPS machine code interpreted by FF9SpecialEffectPlugin.dll; frame counts of an effect live there",
            "Pixel and model data of types 0, 1, 4, 5, 6, 9 beyond the RECT and page tables",
            "Sound small files (AKAO) and external small files",
            "Sequence ops without a name in SFXBinaryFile.SequenceCode.CODE keep raw arguments; model ops (0x0D, 0x80 to 0x83) have no documented argument meaning",
            "Camera code field values (position, movement) beyond frame number, flags and size",
            "SFX id to effect id: the ids come from the battle tables (AA_DATA vfx_no/vfx2, item_use, player attack_sequence, fixed Steal/Throw ids), not from the container",
        ],
    })
}

pub fn run_special_effects(
    battle_path: &Path,
    request: &SpecialEffectsRequest,
) -> Result<(), String> {
    let report = build_report(battle_path, request)?;
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    if let Some(parent) = request.out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&request.out, format!("{text}\n")).map_err(|e| e.to_string())?;
    let t = &report.totals;
    println!(
        "{} effect assets: {} parsed, {} failed; {} records, {} parsed / {} raw bytes; referenced {} distinct, {} present, {} missing; report written to {}",
        t.assets,
        t.parsed,
        t.failed,
        t.records,
        t.parsed_bytes,
        t.raw_bytes,
        t.distinct_referenced,
        t.referenced_present,
        t.referenced_missing,
        request.out.display()
    );
    Ok(())
}
