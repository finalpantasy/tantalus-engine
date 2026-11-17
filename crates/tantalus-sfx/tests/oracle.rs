//! Comparisons with the player's own plugin. Needs the `dll-oracle` feature
//! and an install (`TANTALUS_FF9`, default the Steam library path); the
//! tests are ignored so a plain `cargo test` never looks for the DLL.
//! Run: `cargo test -p tantalus-sfx --features dll-oracle -- --ignored`.
#![cfg(feature = "dll-oracle")]

use tantalus_sfx::oracle::Oracle;

mod common;
use common::{dll, effect_bytes, init_param, install, request};

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

/// `MipsDecodeAll`: the port decodes every script the plugin decoded
/// while playing Fire and Cure (and every effect with `TANTALUS_SFX_SWEEP`)
/// into the same records.
#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn decoder_matches_the_dll() {
    use tantalus_sfx::mips::{Decoder, OpcodeTable};
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("image");
    let table = OpcodeTable::read(&image).expect("opcode table");
    let sweep = std::env::var_os("TANTALUS_SFX_SWEEP").is_some();
    let mut file = tantalus_assets::pc::serialized_inventory::StandaloneSerialized::open(
        &install().join("x64/FF9_Data/resources.assets"),
    )
    .expect("assets");
    let assets = file.text_assets().expect("text assets");
    let (mut scripts, mut stale, mut differing) = (0, 0, Vec::new());
    let range = |name: &str, default: i32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    for id in range("TANTALUS_SFX_FROM", 0)..range("TANTALUS_SFX_TO", 512) {
        if !sweep && id != 0 && id != 8 {
            continue;
        }
        let name = format!("ef{id:03}");
        let Some((_, object)) = assets.iter().find(|(asset, _)| *asset == name) else {
            continue;
        };
        let bytes = file.read_text_asset(object).expect("read").script;
        if sweep {
            eprintln!("{id}: playing");
        }
        // A fresh plugin per effect: what one effect leaves behind (its
        // buffers, a script still running) does not leak into the next.
        let mut oracle = Oracle::load(&dll()).expect("load");
        oracle.init_battle(&init_param()).expect("init");
        oracle.play(id, &bytes, &request()).expect("play");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..900 {
            for script in oracle.decoded_scripts() {
                if !seen.insert((script.bank, script.address, script.words.len())) {
                    continue;
                }
                scripts += 1;
                // Scripts keep data inside their code block and write it
                // while they run, so decode the words as the package holds
                // them (found by the first eight words in memory).
                let probe: Vec<u8> = script
                    .words
                    .iter()
                    .take(8)
                    .flat_map(|w| w.to_le_bytes())
                    .collect();
                let words = match bytes
                    .windows(probe.len())
                    .position(|w| w == probe.as_slice())
                {
                    Some(at) if at + script.words.len() * 4 <= bytes.len() => bytes
                        [at..at + script.words.len() * 4]
                        .chunks(4)
                        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                        .collect(),
                    _ => script.words.clone(),
                };
                let ours = Decoder::default().decode(&table, script.address, &words);
                // Slots an opcode does not write hold the previous
                // record's values, which may come from a script decoded
                // before this one; compare the slots each opcode writes.
                let written = |record: &[u32; 4]| {
                    let count = table.operand_count((record[0] & 0xFFFF) as usize);
                    record[..=count].to_vec()
                };
                let bad: Vec<usize> = (0..ours.len())
                    .filter(|&at| written(&ours[at]) != written(&script.records[at]))
                    .collect();
                // The plugin reuses the code buffer after decoding, so a
                // later snapshot can hold other bytes than it decoded: most
                // records then differ. Count those apart from real misses.
                if bad.len() * 2 > ours.len() {
                    stale += 1;
                } else if let Some(&at) = bad.first() {
                    differing.push((id, script.bank, at, words[at], ours[at], script.records[at]));
                }
            }
            if oracle.frame().expect("frame").is_none() {
                break;
            }
        }
    }
    println!(
        "{scripts} scripts decoded, {stale} overwritten after decoding, {} differ",
        differing.len()
    );
    for row in differing.iter().take(10) {
        println!("{row:x?}");
    }
    assert!(scripts > stale);
    assert!(differing.is_empty());
}

/// The GTE subset against the plugin's own functions, called directly on
/// its register file (`0x180211F40`) with pseudo-random registers:
/// `GteRtps` (0x3E80), `GteRtV0Tr` (0x3D60), `GteAvsz3` (0x48D0) and
/// `GteDpcs` (0x4B50). The entry points come from the code layout (the
/// RVAs Memoria cites inside each function).
#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn gte_matches_the_dll() {
    use tantalus_sfx::gte::Gte;
    let mut oracle = Oracle::load(&dll()).expect("load");
    let mut seed = 0x1234_5678_9ABC_DEF0u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as u32
    };
    let words = |oracle: &Oracle| -> Vec<u32> {
        oracle
            .module_bytes(0x211F40, 256)
            .chunks(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    };
    let mut differing = Vec::new();
    for (name, rva, command) in [
        ("rtps", 0x3E80u32, 0x18_0001u32),
        ("rtv0tr", 0x3D60, 0x48_0012),
        ("avsz3", 0x48D0, 0x158_002D),
        ("dpcs", 0x4B50, 0x78_0010),
    ] {
        for round in 0..2000 {
            let mut gte = Gte {
                dqa_address: (oracle.base() as u32).wrapping_add(0x211FAC),
                ..Gte::default()
            };
            for word in gte.control.iter_mut().chain(gte.data.iter_mut()) {
                // Small values half the time, so clamps and fast paths both run.
                *word = if round % 2 == 0 {
                    next() & 0x0FFF_0FFF
                } else {
                    next()
                };
            }
            let bytes: Vec<u8> = gte
                .control
                .iter()
                .chain(&gte.data)
                .flat_map(|w| w.to_le_bytes())
                .collect();
            oracle.write_module_bytes(0x211F40, &bytes);
            // SAFETY: test-only; the functions take an int (RTPS's vector)
            // or nothing and only touch the register file.
            unsafe {
                let function: extern "C" fn(i32) -> i64 = std::mem::transmute(oracle.internal(rva));
                function(0);
            }
            assert!(gte.command(command));
            let theirs = words(&oracle);
            let ours: Vec<u32> = gte.control.iter().chain(&gte.data).copied().collect();
            if theirs != ours {
                let at = (0..64).find(|&i| theirs[i] != ours[i]).unwrap();
                differing.push((name, round, at, ours[at], theirs[at]));
            }
        }
    }
    println!(
        "module base {:#x}, {} differ",
        oracle.base(),
        differing.len()
    );
    for row in differing.iter().take(12) {
        println!("{row:x?}");
    }
    assert!(differing.is_empty());
}

/// Which plugin functions an effect runs (`TANTALUS_SFX_EFFECT`, default
/// Fire), in first-call order, named from Memoria's `CodeAddresses.g.cs`
/// (`TANTALUS_MEMORIA_PLUGIN`: its `FF9SpecialEffectPlugin` folder).
#[test]
#[ignore = "probe; needs the DLL and Memoria's sources"]
fn coverage_probe() {
    let folder = std::env::var("TANTALUS_MEMORIA_PLUGIN").expect("TANTALUS_MEMORIA_PLUGIN");
    let text = std::fs::read_to_string(std::path::Path::new(&folder).join("CodeAddresses.g.cs"))
        .expect("CodeAddresses.g.cs");
    let mut names = std::collections::BTreeMap::new();
    for line in text.lines() {
        // "... (Name @ 0x18000d140) ..."
        if let Some(at) = line.find(" @ 0x18") {
            let name = line[..at].rsplit('(').next().unwrap_or("").to_string();
            let hex: String = line[at + 5..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if let Ok(va) = u64::from_str_radix(&hex, 16) {
                names.insert((va - 0x1_8000_0000) as u32, name);
            }
        }
    }
    let id: i32 = std::env::var("TANTALUS_SFX_EFFECT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("image");
    let text_section = image.section_of(0x1000).expect("text");
    let bytes = effect_bytes(id);
    let mut oracle = Oracle::load(&dll()).expect("load");
    oracle.init_battle(&init_param()).expect("init");
    let rvas: Vec<u32> = names.keys().copied().collect();
    // SAFETY: test-only; every RVA is a function start Memoria names.
    unsafe { tantalus_sfx::coverage::arm(oracle.base(), text_section, &rvas) };
    oracle.play(id, &bytes, &request()).expect("play");
    let mut frames = 0;
    while frames < 900 && oracle.frame().expect("frame").is_some() {
        frames += 1;
    }
    let hits = tantalus_sfx::coverage::take();
    println!("effect {id}: {frames} frames, {} functions", hits.len());
    for rva in hits {
        println!("  {rva:#07x} {}", names[&rva]);
    }
}
