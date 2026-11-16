//! The overworld's assets for play: walk meshes (foot movement), render
//! meshes and textures per block, and the random encounter table, all read
//! from `p0data3.bin`.
//!
//! Render meshes load on demand: a block prefab (`worldmap/prefabs/
//! worlddiscN/rR/block[x][y]`) assembles with the battle background code,
//! which already resolves `MeshFilter`, `MeshRenderer`, materials and
//! textures. Block geometry is local to the block; its place in the world is
//! `block_origin(x, y)` (`WMWorld.OnInitialize`).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use tantalus_assets::pc::battle_background::{BattleBackground, assemble_background};
use tantalus_assets::pc::model_assembly::SerializedStore;
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_assets::pc::world_map::unity::{
    WalkMeshData, analyze_world_map, extract_walk_meshes, extract_wmap_asset,
};
use tantalus_assets::pc::world_map::{
    DiscImage, ENCOUNTER_PACK, FRAME_PACK_SLOT, Pack, SPECIAL_ENCOUNTER_PACK, parse_encounters,
    parse_special_encounters,
};
use tantalus_legacy::world::{EncounterRecord, EncounterTable, WorldWalk};

const MAX_BUNDLE_BYTES: usize = 600 * 1024 * 1024;

/// Meshes that belong to block form 2 only (`WMWorld.LoadBlock`).
const FORM2_ONLY: [&str; 7] = [
    "Terrain2",
    "Object2",
    "VolcanoCrater2",
    "VolcanoLava2",
    "Sea3_2",
    "Sea4_2",
    "Sea5_2",
];

/// Whether the mesh called `name` draws in the initial block form (form 1).
pub fn mesh_in_form1(name: &str) -> bool {
    !FORM2_ONLY.contains(&name)
}

/// Meshes only form 1 shows; form 2 swaps them for the `*2` ones.
const FORM1_ONLY: [&str; 4] = ["Terrain", "Object", "VolcanoCrater", "VolcanoLava"];
/// The block that swaps its seas as well (number 219, `WMWorld.LoadBlock`).
const WATER_SHRINE_BLOCK: (usize, usize) = (3, 9);

/// Whether a block in `form` draws the mesh called `name`.
pub fn mesh_in_form(name: &str, form: u8, block: (usize, usize)) -> bool {
    if form != 2 {
        return mesh_in_form1(name);
    }
    if FORM1_ONLY.contains(&name) {
        return false;
    }
    !(block == WATER_SHRINE_BLOCK && ["Sea3", "Sea4", "Sea5"].contains(&name))
}

/// The blocks that show their second form for the story point `scenario`
/// (`Global` UInt16 0) and the discovery bits `event101` (`gEventGlobal(101)`)
/// on disc 1 (`WorldConfiguration.UsePlaceAlternateForm`,
/// `ff9.w_worldChangeBlockSet`).
pub fn alternate_form_blocks(scenario: i32, event101: i32) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut add = |places: &[(usize, usize)]| blocks.extend_from_slice(places);
    if (2990..6990).contains(&scenario) {
        add(&[(18, 14), (17, 12)]); // South Gate destroyed
    }
    if scenario >= 8800 {
        add(&[(19, 10), (19, 11), (20, 10), (20, 11)]); // Alexandria
    }
    if (10600..10700).contains(&scenario) {
        add(&[(7, 1), (8, 1), (14, 15)]); // Fire Shrine opened
    }
    if scenario >= 5598 {
        add(&[(13, 16), (13, 17), (14, 16), (14, 17)]); // Lindblum
    }
    if scenario >= 4990 {
        add(&[(13, 12), (14, 12)]); // Cleyra
    }
    if scenario >= 6200 {
        add(&[(14, 6), (21, 10), (22, 14)]); // Black Mage Village
    }
    if (10600..10700).contains(&scenario) {
        add(&[(3, 9), (9, 1)]); // Water Shrine opened
    }
    if event101 & 0x80 != 0 {
        add(&[(16, 1), (13, 4), (14, 5)]); // Mognet Central
    }
    if event101 & 0x40 != 0 {
        add(&[(0, 0), (16, 14), (9, 17)]); // Chocobo's Paradise
    }
    blocks
}

pub struct WorldAssets {
    disc: u32,
    bytes: Vec<u8>,
    pub walk: WorldWalk,
    /// Prefab root `GameObject` per block.
    prefabs: HashMap<(usize, usize), i64>,
    pub encounters: EncounterTable,
    pub special: Vec<[u16; 12]>,
    pub failures: Vec<String>,
}

impl WorldAssets {
    /// `streaming` is the `StreamingAssets` folder.
    pub fn load(streaming: &Path, disc: u32) -> Result<Self, String> {
        let path = streaming.join("p0data3.bin");
        let source = "p0data3.bin";
        let bundle = UnityRawBundle::open(&path).map_err(|error| error.to_string())?;
        let bytes = bundle
            .read_entry(0, MAX_BUNDLE_BYTES)
            .map_err(|error| error.to_string())?;
        let (walk, prefabs, encounters, special, failures) = {
            let file = SerializedFile::parse(&bytes, source).map_err(|e| e.to_string())?;
            let report = analyze_world_map(&file, source).map_err(|e| e.to_string())?;
            let cells: Vec<_> = report.blocks.iter().filter(|b| b.disc == disc).collect();
            let mut wanted = BTreeSet::new();
            for cell in &cells {
                if let Some(prefab) = &cell.prefab {
                    wanted.extend(prefab.slots.iter().filter_map(|s| s.mesh_path.clone()));
                }
            }
            let (extracted, skipped) =
                extract_walk_meshes(&file, source, &|p: &str| wanted.contains(p))
                    .map_err(|e| e.to_string())?;
            let meshes: HashMap<String, WalkMeshData> =
                extracted.into_iter().map(|m| (m.path.clone(), m)).collect();
            let mut loaded = crate::world_walk_cli::load_world(&cells, &meshes);
            loaded.failures.extend(skipped);

            let store = SerializedStore::new(&file, source).map_err(|e| e.to_string())?;
            let container = store.container().map_err(|e| e.to_string())?;
            let roots: HashMap<&str, i64> = container
                .iter()
                .filter(|e| e.class_id == 1)
                .map(|e| (e.path.as_str(), e.path_id))
                .collect();
            let mut prefabs = HashMap::new();
            for cell in &cells {
                if let Some(prefab) = &cell.prefab
                    && let Some(root) = roots.get(prefab.prefab_path.as_str())
                {
                    prefabs.insert((cell.x as usize, cell.y as usize), *root);
                }
            }

            let image_name = format!("disc{disc}/discmr.img.bytes");
            let image = extract_wmap_asset(&file, source, &image_name)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("{image_name} is missing"))?;
            let image_file = DiscImage::parse(&image, "disc.img").map_err(|e| e.to_string())?;
            let frame = image_file
                .entry(&image, FRAME_PACK_SLOT)
                .ok_or("frame pack is missing")?;
            let pack = Pack::parse(frame, "frame pack").map_err(|e| e.to_string())?;
            let (records, _) = parse_encounters(
                pack.segment(ENCOUNTER_PACK).ok_or("encounter pack")?,
                "encounters",
            )
            .map_err(|e| e.to_string())?;
            let (special, _) = parse_special_encounters(
                pack.segment(SPECIAL_ENCOUNTER_PACK).ok_or("special pack")?,
                "special",
            )
            .map_err(|e| e.to_string())?;
            let table = EncounterTable::new(
                records
                    .iter()
                    .map(|r| EncounterRecord {
                        scene: r.scene,
                        pattern: r.pattern,
                        pad: r.pad,
                    })
                    .collect(),
            )
            .map_err(|e| e.to_string())?;
            (loaded.world, prefabs, table, special, loaded.failures)
        };
        Ok(Self {
            disc,
            bytes,
            walk,
            prefabs,
            encounters,
            special,
            failures,
        })
    }

    pub fn disc(&self) -> u32 {
        self.disc
    }

    /// Blocks with render geometry.
    pub fn has_block(&self, x: usize, y: usize) -> bool {
        self.prefabs.contains_key(&(x, y))
    }

    /// Assembles one block's meshes and textures.
    pub fn render_block(&self, x: usize, y: usize) -> Result<BattleBackground, String> {
        let root = *self
            .prefabs
            .get(&(x, y))
            .ok_or_else(|| format!("no block [{x}][{y}]"))?;
        let source = "p0data3.bin";
        let file = SerializedFile::parse(&self.bytes, source).map_err(|e| e.to_string())?;
        let store = SerializedStore::new(&file, source).map_err(|e| e.to_string())?;
        assemble_background(&store, &format!("block[{x}][{y}]"), root).map_err(|e| e.to_string())
    }
}
