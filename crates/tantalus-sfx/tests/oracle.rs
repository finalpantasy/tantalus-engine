//! Comparisons with the player's own plugin. Needs the `dll-oracle` feature
//! and an install (`TANTALUS_FF9`, default the Steam library path); the
//! tests are ignored so a plain `cargo test` never looks for the DLL.
//! Run: `cargo test -p tantalus-sfx --features dll-oracle -- --ignored`.
#![cfg(feature = "dll-oracle")]

use tantalus_sfx::oracle::{INIT_PARAM_SIZE, Oracle, REQUEST_SIZE};

pub fn install() -> std::path::PathBuf {
    std::env::var_os("TANTALUS_FF9").map_or_else(
        || std::path::PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"),
        std::path::PathBuf::from,
    )
}

/// `SpecialEffects/ef<id>` from `resources.assets`.
pub fn effect_bytes(id: i32) -> Vec<u8> {
    use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
    let mut file = StandaloneSerialized::open(&install().join("x64/FF9_Data/resources.assets"))
        .expect("assets");
    let name = format!("ef{id:03}");
    let (_, object) = file
        .text_assets()
        .expect("text assets")
        .into_iter()
        .find(|(asset, _)| *asset == name)
        .expect("effect");
    file.read_text_asset(&object).expect("read").script
}

/// Zidane (btl_id 1, serial 0) against one enemy (btl_id 0x10), as
/// `SFX.InitBattleParty` fills it.
pub fn init_param() -> [u8; INIT_PARAM_SIZE] {
    let mut param = [0u8; INIT_PARAM_SIZE];
    for (slot, (player, btl_id)) in [(0usize, (1u8, 1u16)), (4, (0, 0x10))] {
        let at = 8 + slot * 36;
        param[at] = player;
        param[at + 1] = slot as u8 & 3;
        param[at + 8..at + 12].copy_from_slice(&200i32.to_le_bytes());
        param[at + 12..at + 16].copy_from_slice(&200i32.to_le_bytes());
        param[at + 16..at + 20].copy_from_slice(&400i32.to_le_bytes());
        param[at + 20..at + 22].copy_from_slice(&btl_id.to_le_bytes());
    }
    // Identity camera.
    let cam = 8 + 8 * 36;
    for i in [0usize, 4, 8] {
        param[cam + 2 * i..cam + 2 * i + 2].copy_from_slice(&4096i16.to_le_bytes());
    }
    param
}

/// Caster 1 on target 0x10.
pub fn request() -> [u8; REQUEST_SIZE] {
    let mut request = [0u8; REQUEST_SIZE];
    request[0..2].copy_from_slice(&1u16.to_le_bytes());
    request[4..6].copy_from_slice(&1u16.to_le_bytes());
    request[8..10].copy_from_slice(&0x10u16.to_le_bytes());
    request[56] = 1;
    request
}

pub fn dll() -> std::path::PathBuf {
    install().join("x64/FF9_Data/Plugins/FF9SpecialEffectPlugin.dll")
}

/// The uploads the plugin made (RECT and pixel hash), in order.
fn uploads(calls: &[tantalus_sfx::oracle::Call]) -> Vec<([i32; 4], u64)> {
    calls
        .iter()
        .filter(|call| call.command == 100)
        .map(|call| (call.args, call.pixels.unwrap_or(0)))
        .collect()
}

fn ours(list: &[tantalus_sfx::loader::Upload]) -> Vec<([i32; 4], u64)> {
    list.iter()
        .map(|upload| {
            let r = upload.rect;
            (
                [
                    i32::from(r.x),
                    i32::from(r.y),
                    i32::from(r.w),
                    i32::from(r.h),
                ],
                tantalus_sfx::oracle::fnv(&upload.pixels),
            )
        })
        .collect()
}

/// Plays `id` in the plugin until it ends (at most `frames`); returns the
/// effect's uploads and the VRAM it leaves.
fn play_in_dll(
    oracle: &mut Oracle,
    id: i32,
    bytes: &[u8],
    frames: u32,
) -> (Vec<([i32; 4], u64)>, tantalus_sfx::vram::Vram) {
    oracle.take_calls();
    oracle.play(id, bytes, &request()).expect("play");
    for _ in 0..frames {
        if oracle.frame().expect("frame").is_none() {
            break;
        }
    }
    (uploads(&oracle.take_calls()), oracle.vram())
}

/// `SFX_InitSystem` uploads the same templates, and Fire (0), Cure (8) and
/// Potion's effect upload the same pages, as the port reads them.
#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn loader_uploads_match_the_dll() {
    use tantalus_assets::pc::special_effect::EffectFile;
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("pe");
    let init = tantalus_sfx::loader::init_uploads(&image).expect("templates");
    let mut oracle = Oracle::load(&dll()).expect("load");
    assert_eq!(uploads(&oracle.take_calls()), ours(&init), "SFX_InitSystem");
    oracle.init_battle(&init_param()).expect("init battle");
    for id in [0, 8] {
        let bytes = effect_bytes(id);
        let effect = EffectFile::parse(&bytes, "ef").expect("parse");
        let mine = tantalus_sfx::loader::effect_uploads(&effect, &bytes, &image);
        let (theirs, vram) = play_in_dll(&mut oracle, id, &bytes, 600);
        assert_eq!(theirs, ours(&mine), "effect {id}");
        let mut port = tantalus_sfx::vram::Vram::default();
        tantalus_sfx::loader::apply(&mut port, &init);
        tantalus_sfx::loader::apply(&mut port, &mine);
        // Effects write VRAM only through these uploads before their
        // first frame of drawing; the rest (StoreImage, MoveImage) is the
        // interpreter's.
        let _ = vram;
    }
}

#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn fire_uploads() {
    let mut oracle = Oracle::load(&dll()).expect("load");
    let init = oracle.take_calls();
    println!("init: {} calls", init.len());
    for call in &init {
        println!("  {:?}", call);
    }
    oracle.init_battle(&init_param()).expect("init battle");
    let bytes = effect_bytes(0);
    oracle.play(0, &bytes, &request()).expect("play");
    let mut frames = 0;
    while frames < 400 {
        let Some(prims) = oracle.frame().expect("frame") else {
            break;
        };
        frames += 1;
        if frames % 50 == 1 {
            println!("frame {frames}: {} prims", prims.len());
        }
    }
    let calls = oracle.take_calls();
    println!("{frames} frames, {} calls", calls.len());
    let mut counts = std::collections::BTreeMap::new();
    for call in &calls {
        *counts.entry(call.command).or_insert(0) += 1;
        if call.command == 100 {
            println!(
                "  frame {} LoadImage {:?} {:x?}",
                call.frame, call.args, call.pixels
            );
        }
    }
    println!("{counts:?}");
}

/// Every effect package: the port's uploads against the plugin's, one line
/// per effect (`TANTALUS_SFX_FROM`/`TANTALUS_SFX_TO` pick the range).
#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn loader_sweep() {
    use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
    use tantalus_assets::pc::special_effect::EffectFile;
    let range = |name: &str, default: i32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (from, to) = (range("TANTALUS_SFX_FROM", 0), range("TANTALUS_SFX_TO", 511));
    let mut file = StandaloneSerialized::open(&install().join("x64/FF9_Data/resources.assets"))
        .expect("assets");
    let assets = file.text_assets().expect("text assets");
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("pe");
    let mut oracle = Oracle::load(&dll()).expect("load");
    oracle.init_battle(&init_param()).expect("init battle");
    let (mut same, mut differ) = (0, Vec::new());
    for id in from..to {
        let name = format!("ef{id:03}");
        let Some((_, object)) = assets.iter().find(|(asset, _)| *asset == name) else {
            continue;
        };
        let bytes = file.read_text_asset(object).expect("read").script;
        let Ok(effect) = EffectFile::parse(&bytes, &name) else {
            eprintln!("{id}: parse failed");
            continue;
        };
        eprintln!("{id}: playing");
        let mine = ours(&tantalus_sfx::loader::effect_uploads(
            &effect, &bytes, &image,
        ));
        let (theirs, _) = play_in_dll(&mut oracle, id, &bytes, 900);
        if mine == theirs {
            same += 1;
        } else {
            eprintln!("{id}: differ ours {} theirs {}", mine.len(), theirs.len());
            if std::env::var_os("TANTALUS_SFX_SHOW").is_some() {
                for (i, pair) in theirs.iter().enumerate() {
                    eprintln!(
                        "  theirs {i} {:?}{}",
                        pair,
                        if mine.contains(pair) {
                            ""
                        } else {
                            " (not ours)"
                        }
                    );
                }
                for pair in mine.iter().filter(|pair| !theirs.contains(pair)) {
                    eprintln!("  ours only {pair:?}");
                }
            }
            differ.push(id);
        }
    }
    eprintln!("same {same}, differ {differ:?}");
}

/// Writes, under `TANTALUS_SFX_OUT` (default `local/sfx`), the texture page
/// the first textured primitive of `id` samples, decoded from the DLL's
/// VRAM and from the port's, side by side, and asserts they are equal.
fn page_capture(id: i32) {
    use tantalus_assets::pc::special_effect::EffectFile;
    use tantalus_assets::pc::ui_atlas::encode_png_rgba;
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("pe");
    let mut oracle = Oracle::load(&dll()).expect("load");
    oracle.init_battle(&init_param()).expect("init battle");
    let bytes = effect_bytes(id);
    oracle.play(id, &bytes, &request()).expect("play");
    // The first textured polygon: GP0 0x24..0x3F with bit 2 set; word 2
    // holds uv0 and the CLUT, word 4 uv1 and the texture page.
    let mut found = None;
    let mut dll_vram = None;
    for _ in 0..900 {
        let Some(prims) = oracle.frame().expect("frame") else {
            break;
        };
        for (_, words) in &prims {
            let Some(&first) = words.get(1) else { continue };
            let code = (first >> 24) as u8;
            if (0x20..0x40).contains(&code) && code & 4 != 0 && words.len() > 5 {
                let clut = (words[3] >> 16) as u16;
                let tpage = (words[5] >> 16) as u16;
                found = Some((code, clut, tpage));
                break;
            }
        }
        if found.is_some() {
            dll_vram = Some(oracle.vram());
            break;
        }
    }
    let (code, clut, tpage) = found.expect("a textured primitive");
    let dll_vram = dll_vram.expect("vram");
    let effect = EffectFile::parse(&bytes, "ef").expect("parse");
    let mut port = tantalus_sfx::vram::Vram::default();
    tantalus_sfx::loader::apply(
        &mut port,
        &tantalus_sfx::loader::init_uploads(&image).expect("init"),
    );
    let mut uploads = tantalus_sfx::loader::effect_uploads(&effect, &bytes, &image);
    // The restore comes after the sequence ends.
    if uploads.len() >= 4
        && uploads[uploads.len() - 4..]
            == tantalus_sfx::loader::plugin_pages(&image).expect("pages")[..]
    {
        uploads.truncate(uploads.len() - 4);
    }
    tantalus_sfx::loader::apply(&mut port, &uploads);
    let tx = i32::from(tpage & 0xF) * 64;
    let ty = i32::from((tpage >> 4) & 1) * 256;
    let depth = ((tpage >> 7) & 3) as u8;
    let clut_at = (i32::from(clut & 0x3F) * 16, i32::from(clut >> 6));
    let (width, theirs) = dll_vram.page_rgba(tx, ty, depth, clut_at);
    let (_, mine) = port.page_rgba(tx, ty, depth, clut_at);
    let mut both = Vec::with_capacity(theirs.len() * 2 + 32 * 256 * 4);
    for row in 0..256 {
        both.extend_from_slice(&theirs[row * width * 4..(row + 1) * width * 4]);
        both.extend(std::iter::repeat_n([40u8, 40, 40, 255], 16).flatten());
        both.extend_from_slice(&mine[row * width * 4..(row + 1) * width * 4]);
    }
    let out = std::env::var_os("TANTALUS_SFX_OUT")
        .map_or_else(|| std::path::PathBuf::from("local/sfx"), Into::into);
    std::fs::create_dir_all(&out).expect("out dir");
    let name = format!(
        "ef{id:03}-page-{tx}-{ty}-d{depth}-clut-{}-{}.png",
        clut_at.0, clut_at.1
    );
    std::fs::write(
        out.join(&name),
        encode_png_rgba((2 * width + 16) as u32, 256, &both),
    )
    .expect("png");
    eprintln!(
        "ef{id:03}: prim 0x{code:02X}, page ({tx}, {ty}) depth {depth}, CLUT {clut_at:?} -> {name}"
    );
    assert_eq!(theirs, mine, "page of ef{id:03}");
}

#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn fire_and_cure_pages_match_the_dll() {
    page_capture(0);
    page_capture(8);
}
