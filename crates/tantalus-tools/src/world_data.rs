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
