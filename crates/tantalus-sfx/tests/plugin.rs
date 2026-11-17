//! The translated plugin against the player's own DLL, frame by frame:
//! every host callback and every primitive `SFX_GetPrim` hands out.
//! Run: `cargo test -p tantalus-sfx --features dll-oracle --test plugin -- --ignored`.
#![cfg(feature = "dll-oracle")]

mod common;

use common::{dll, effect_bytes, init_param, request};
use std::sync::{Arc, Mutex};
use tantalus_sfx::oracle::Oracle;
use tantalus_sfx::plugin::debug_host::{Call, DebugHost};
use tantalus_sfx::plugin::{Host, Plugin};

struct Shared(Arc<Mutex<DebugHost>>);

impl Host for Shared {
    fn callback(
        &mut self,
        memory: &mut tantalus_sfx::mem::Memory,
        code: i32,
        a0: i32,
        a1: i32,
        a2: i32,
        a3: i32,
        p: u64,
    ) -> i32 {
        self.0
            .lock()
            .unwrap()
            .callback(memory, code, a0, a1, a2, a3, p)
    }
}

/// One run: per frame, the primitives (OT links masked) and the calls.
pub struct Run {
    pub frames: Vec<Vec<(i32, Vec<u32>)>>,
    pub calls: Vec<Call>,
    /// The module base the run used.
    pub base: u64,
}

fn mask(prims: Vec<(i32, Vec<u32>)>) -> Vec<(i32, Vec<u32>)> {
    prims
        .into_iter()
        .map(|(depth, mut words)| {
            words[0] &= 0xFF00_0000;
            (depth, words)
        })
        .collect()
}

pub fn run_dll(id: i32, limit: usize) -> Run {
    let bytes = effect_bytes(id);
    let mut oracle = Oracle::load(&dll()).expect("load");
    oracle.init_battle(&init_param()).expect("init");
    oracle.play(id, &bytes, &request()).expect("play");
    let mut frames = Vec::new();
    while frames.len() < limit {
        match oracle.frame().expect("frame") {
            Some(prims) => frames.push(mask(prims)),
            None => break,
        }
    }
    Run {
        frames,
        calls: oracle.take_calls(),
        base: oracle.base() as u64,
    }
}

pub fn run_port(id: i32, limit: usize, module_base: u64) -> Run {
    let bytes = effect_bytes(id);
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("pe");
    let host = Arc::new(Mutex::new(DebugHost::default()));
    let mut plugin = Plugin::load(&image, Box::new(Shared(host.clone()))).expect("image");
    plugin.module_base = module_base;
    plugin.init_system();
    plugin.init_battle(&init_param());
    plugin.play(id, &bytes, &request());
    let mut frames = Vec::new();
    while frames.len() < limit {
        host.lock().unwrap().frame += 1;
        match plugin.frame() {
            Some(prims) => frames.push(mask(prims)),
            None => break,
        }
    }
    let calls = std::mem::take(&mut host.lock().unwrap().calls);
    Run {
        frames,
        calls,
        base: module_base,
    }
}

/// The first difference, or `None` when the runs agree.
pub fn first_difference(dll: &Run, port: &Run) -> Option<String> {
    let frames = dll.frames.len().max(port.frames.len());
    for frame in 0..=frames {
        let theirs: Vec<_> = dll
            .calls
            .iter()
            .filter(|c| c.frame as usize == frame)
            .collect();
        let ours: Vec<_> = port
            .calls
            .iter()
            .filter(|c| c.frame as usize == frame)
            .collect();
        if theirs != ours {
            let at = theirs
                .iter()
                .zip(&ours)
                .position(|(a, b)| a != b)
                .unwrap_or(theirs.len().min(ours.len()));
            return Some(format!(
                "frame {frame}: calls differ at {at} of {}/{}: dll {:?} port {:?}",
                theirs.len(),
                ours.len(),
                theirs.get(at),
                ours.get(at)
            ));
        }
        if frame == 0 {
            continue;
        }
        let (a, b) = (dll.frames.get(frame - 1), port.frames.get(frame - 1));
        if a != b {
            let (a, b) = (
                a.cloned().unwrap_or_default(),
                b.cloned().unwrap_or_default(),
            );
            let at = a
                .iter()
                .zip(&b)
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(b.len()));
            return Some(format!(
                "frame {frame}: prims differ at {at} of {}/{}: dll {:x?} port {:x?}",
                a.len(),
                b.len(),
                a.get(at),
                b.get(at)
            ));
        }
    }
    None
}

fn compare(id: i32) {
    let limit = 900;
    let theirs = run_dll(id, limit);
    let ours = run_port(id, limit, theirs.base);
    let prims: usize = theirs.frames.iter().map(Vec::len).sum();
    println!(
        "effect {id}: dll {} frames, {prims} prims, {} calls; port {} frames, {} calls",
        theirs.frames.len(),
        theirs.calls.len(),
        ours.frames.len(),
        ours.calls.len()
    );
    if let Some(difference) = first_difference(&theirs, &ours) {
        panic!("effect {id}: {difference}");
    }
}

#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn fire_matches_the_dll() {
    compare(0);
}

#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn cure_matches_the_dll() {
    compare(8);
}

/// `TANTALUS_SFX_EFFECT`: any one effect.
#[test]
#[ignore = "needs the player's FF9SpecialEffectPlugin.dll"]
fn effect_matches_the_dll() {
    let id = std::env::var("TANTALUS_SFX_EFFECT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    compare(id);
}

/// Every effect package (`TANTALUS_SFX_FROM`..`TANTALUS_SFX_TO`), a fresh
/// DLL and a fresh port each; prints one line per effect.
#[test]
#[ignore = "sweep; needs the player's FF9SpecialEffectPlugin.dll"]
fn sweep() {
    let range = |name: &str, default: i32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let mut file = tantalus_assets::pc::serialized_inventory::StandaloneSerialized::open(
        &common::install().join("x64/FF9_Data/resources.assets"),
    )
    .expect("assets");
    let names: std::collections::HashSet<String> = file
        .text_assets()
        .expect("text assets")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let (mut same, mut total) = (0, 0);
    for id in range("TANTALUS_SFX_FROM", 0)..range("TANTALUS_SFX_TO", 512) {
        // The short effects have no package (`SFXData.IsShortSpecialEffect`).
        let short =
            (100..=118).contains(&id) || (313..=376).contains(&id) || (461..=485).contains(&id);
        if !short && !names.contains(&format!("ef{id:03}")) {
            continue;
        }
        total += 1;
        let theirs = run_dll(id, 900);
        let ours = std::panic::catch_unwind(|| run_port(id, 900, theirs.base));
        let line = match ours {
            Err(panic) => format!(
                "PANIC {}",
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default()
            ),
            Ok(ours) => match first_difference(&theirs, &ours) {
                None => {
                    same += 1;
                    format!("same ({} frames)", theirs.frames.len())
                }
                Some(difference) => difference,
            },
        };
        println!("{id}: {line}");
    }
    println!("{same} of {total} effects match");
}

/// Debug probe: runs the port on `TANTALUS_SFX_EFFECT` and prints the RAM
/// window of the mapper when it faults.
#[test]
#[ignore = "probe"]
fn probe_fault() {
    let id: i32 = std::env::var("TANTALUS_SFX_EFFECT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let bytes = effect_bytes(id);
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("pe");
    let host = Arc::new(Mutex::new(DebugHost::default()));
    let mut plugin = Plugin::load(&image, Box::new(Shared(host.clone()))).expect("image");
    plugin.init_system();
    plugin.init_battle(&init_param());
    plugin.play(id, &bytes, &request());
    let mapper = tantalus_sfx::mem::MAPPER;
    let mut frame = 0;
    loop {
        let before = (
            plugin.ld::<u64>(mapper + 8144),
            plugin.ld::<u32>(mapper + 8136),
        );
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plugin.frame()));
        frame += 1;
        match r {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {
                for index in 0..254u64 {
                    let entry = mapper + 32 * index;
                    if plugin.ld::<u8>(entry) != 0 {
                        println!("window {index}: start {:x}", plugin.ld::<u64>(entry + 16));
                    }
                }
                println!(
                    "fault in frame {frame}; ram before {:x?} after {:x?} sp {:x}",
                    before,
                    (
                        plugin.ld::<u64>(mapper + 8144),
                        plugin.ld::<u32>(mapper + 8136)
                    ),
                    plugin.sp
                );
                break;
            }
        }
    }
}

/// The module's `.data` (from RVA 0x4F000) differs where both sides hold
/// the same non-pointer value; aligned 8-byte words where ours holds one of
/// the port's addresses are skipped.
fn data_differences(oracle: &Oracle, plugin: &Plugin, limit: usize) -> Vec<(u32, u64, u64)> {
    const START: u32 = 0x4F000;
    const LEN: usize = 6_108_224;
    let theirs = oracle.module_bytes(START, LEN);
    let ours = plugin
        .m
        .bytes(tantalus_sfx::mem::IMAGE_BASE + u64::from(START), LEN)
        .expect("data")
        .to_vec();
    let mut out = Vec::new();
    let mut skip_to = 0;
    let word = |bytes: &[u8], at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    for at in (0..LEN - 8).step_by(4) {
        if (0x1_8000_0000..0x41_0000_0000).contains(&word(&ours, at)) {
            skip_to = at + 8;
        }
        if at < skip_to {
            continue;
        }
        let (a, b) = (&theirs[at..at + 4], &ours[at..at + 4]);
        if a != b {
            out.push((
                START + at as u32,
                u64::from(u32::from_le_bytes(a.try_into().unwrap())),
                u64::from(u32::from_le_bytes(b.try_into().unwrap())),
            ));
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// Lockstep probe: plays `TANTALUS_SFX_EFFECT` in the DLL and the port and
/// prints the first frame whose module data differ.
#[test]
#[ignore = "probe"]
fn probe_lockstep() {
    let id: i32 = std::env::var("TANTALUS_SFX_EFFECT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let bytes = effect_bytes(id);
    let mut oracle = Oracle::load(&dll()).expect("load");
    oracle.init_battle(&init_param()).expect("init");
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("pe");
    let host = Arc::new(Mutex::new(DebugHost::default()));
    let mut plugin = Plugin::load(&image, Box::new(Shared(host.clone()))).expect("image");
    plugin.module_base = oracle.base() as u64;
    plugin.init_system();
    plugin.init_battle(&init_param());
    let baseline: std::collections::HashSet<u32> = data_differences(&oracle, &plugin, 100_000)
        .into_iter()
        .map(|d| d.0)
        .collect();
    println!("{} words differ before play", baseline.len());
    oracle.play(id, &bytes, &request()).expect("play");
    plugin.play(id, &bytes, &request());
    for frame in 0..900 {
        let differences: Vec<_> = data_differences(&oracle, &plugin, 100_000)
            .into_iter()
            .filter(|d| !baseline.contains(&d.0))
            .collect();
        if !differences.is_empty() {
            println!("frame {frame}: {} words differ", differences.len());
            for (rva, a, b) in differences.iter().take(24) {
                println!("  {rva:#x}: dll {a:016x} port {b:016x}");
            }
            // Where the DLL keeps pointers into its windows.
            let data = oracle.module_bytes(0x4F000, 6_108_224);
            for index in 0..16u32 {
                let entry = 0x576A10 + 32 * index;
                let e = oracle.module_bytes(entry, 32);
                if e[0] == 0 {
                    continue;
                }
                let start = u64::from_le_bytes(e[16..24].try_into().unwrap());
                let mut holders = Vec::new();
                for at in (0..data.len() - 8).step_by(4) {
                    let v = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
                    if v >= start && v < start + 0x30_0000 && holders.len() < 6 {
                        let rva = 0x4F000 + at as u64;
                        holders.push(format!(
                            "{rva:#x}+{:#x} (port {:#x})",
                            v - start,
                            plugin.ld::<u64>(0x1_8000_0000 + rva)
                        ));
                    }
                }
                println!("  window {index} start {start:#x}: {}", holders.join(", "));
            }
            return;
        }
        // The script call context (registers, MIPS stacks).
        let theirs_ctx = u64::from_le_bytes(oracle.module_bytes(0x3678E0, 8).try_into().unwrap());
        let ours_ctx = plugin.ld::<u64>(0x1_803678E0);
        if theirs_ctx != 0 && ours_ctx != 0 {
            // SAFETY: test-only read of the DLL's own context block.
            let a = unsafe { std::slice::from_raw_parts(theirs_ctx as *const u8, 0x2EC0) };
            let b = plugin.m.bytes(ours_ctx, 0x2EC0).expect("context");
            let mut shown = 0;
            for at in (0..0x2EC0).step_by(4) {
                let (x, y) = (&a[at..at + 4], &b[at..at + 4]);
                if x != y && shown < 12 {
                    let ptr = at + 8 <= 0x2EC0
                        && (0x1_8000_0000..0x4_0000_0000)
                            .contains(&u64::from_le_bytes(b[at..at + 8].try_into().unwrap()));
                    let prev_ptr = at >= 4
                        && (0x1_8000_0000..0x4_0000_0000)
                            .contains(&u64::from_le_bytes(b[at - 4..at + 4].try_into().unwrap()));
                    if !ptr && !prev_ptr {
                        println!(
                            "frame {frame}: context +{at:#x}: dll {:08x} port {:08x}",
                            u32::from_le_bytes(x.try_into().unwrap()),
                            u32::from_le_bytes(y.try_into().unwrap())
                        );
                        shown += 1;
                    }
                }
            }
            if shown > 0 {
                return;
            }
        }
        host.lock().unwrap().frame += 1;
        plugin.tracing = std::env::var_os("TANTALUS_SFX_TRACE").is_some();
        let a = oracle.frame().expect("frame");
        let b = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plugin.frame()));
        match (a, b) {
            (_, Err(_)) => {
                let calls: Vec<_> = plugin.calls.iter().rev().take(40).collect();
                if let Ok(name) = std::env::var("TANTALUS_SFX_FIND") {
                    let all: Vec<_> = plugin.calls.iter().collect();
                    for (i, call) in all.iter().enumerate() {
                        if **call == name {
                            println!("before {name}: {:?}", &all[i.saturating_sub(30)..i]);
                        }
                    }
                }
                println!(
                    "port faulted in frame {}; last calls (newest first): {calls:?}",
                    frame + 1
                );
                let f = frame as u32 + 1;
                if let Ok(rva) = std::env::var("TANTALUS_SFX_DUMP") {
                    let rva = u32::from_str_radix(rva.trim_start_matches("0x"), 16).unwrap();
                    println!("dll  {:02x?}", oracle.module_bytes(rva, 48));
                    println!(
                        "port {:02x?}",
                        plugin.m.bytes(0x1_8000_0000 + u64::from(rva), 48).unwrap()
                    );
                }
                println!(
                    "dll calls: {:?}",
                    oracle
                        .take_calls()
                        .into_iter()
                        .filter(|c| c.frame == f)
                        .collect::<Vec<_>>()
                );
                println!(
                    "port calls: {:?}",
                    host.lock()
                        .unwrap()
                        .calls
                        .iter()
                        .filter(|c| c.frame == f)
                        .collect::<Vec<_>>()
                );
                return;
            }
            (None, Ok(None)) => break,
            (Some(_), Ok(Some(_))) => {}
            _ => {
                println!("frame {}: one side ended", frame + 1);
                return;
            }
        }
    }
    println!("no difference");
}

#[test]
#[ignore = "probe"]
fn probe_calls() {
    let id: i32 = std::env::var("TANTALUS_SFX_EFFECT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(188);
    let from: u32 = std::env::var("TANTALUS_SFX_FROM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(55);
    let theirs = run_dll(id, 90);
    let ours = run_port(id, 90, theirs.base);
    for (name, run) in [("dll", &theirs), ("port", &ours)] {
        for call in run
            .calls
            .iter()
            .filter(|c| c.frame >= from && c.frame <= from + 6)
        {
            println!(
                "{name} f{} cmd {:#x} low {:#x} args {:?}",
                call.frame, call.command, call.low, call.args
            );
        }
    }
}

/// The functions the DLL and the port call in one frame
/// (`TANTALUS_SFX_EFFECT`, `TANTALUS_SFX_FRAME`, `TANTALUS_MEMORIA_PLUGIN`).
#[test]
#[ignore = "probe"]
fn probe_functions() {
    let id: i32 = std::env::var("TANTALUS_SFX_EFFECT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(188);
    let target: usize = std::env::var("TANTALUS_SFX_FRAME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(58);
    let folder = std::env::var("TANTALUS_MEMORIA_PLUGIN").expect("TANTALUS_MEMORIA_PLUGIN");
    let text = std::fs::read_to_string(std::path::Path::new(&folder).join("CodeAddresses.g.cs"))
        .expect("CodeAddresses.g.cs");
    let mut names = std::collections::BTreeMap::new();
    for line in text.lines() {
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
    let bytes = effect_bytes(id);
    let image =
        tantalus_sfx::image::PluginImage::parse(std::fs::read(dll()).expect("dll")).expect("pe");
    let text_section = image.section_of(0x1000).expect("text");
    let mut oracle = Oracle::load(&dll()).expect("load");
    oracle.init_battle(&init_param()).expect("init");
    oracle.play(id, &bytes, &request()).expect("play");
    for _ in 0..target {
        oracle.frame().expect("frame");
    }
    let rvas: Vec<u32> = names.keys().copied().collect();
    // SAFETY: test-only; every RVA is a function start Memoria names.
    unsafe { tantalus_sfx::coverage::arm(oracle.base(), text_section, &rvas) };
    oracle.frame().expect("frame");
    let hits = tantalus_sfx::coverage::take_with_args();
    if std::env::var_os("TANTALUS_SFX_ORDER").is_some() {
        for (rva, args) in &hits {
            println!("dll first call {} {args:x?}", names[rva]);
        }
    }
    let dll_set: std::collections::BTreeSet<String> = hits
        .into_iter()
        .map(|(rva, _)| names[&rva].clone())
        .collect();

    let host = Arc::new(Mutex::new(DebugHost::default()));
    let mut plugin = Plugin::load(&image, Box::new(Shared(host.clone()))).expect("image");
    plugin.module_base = oracle.base() as u64;
    plugin.init_system();
    plugin.init_battle(&init_param());
    plugin.play(id, &bytes, &request());
    for _ in 0..target {
        host.lock().unwrap().frame += 1;
        plugin.frame();
    }
    plugin.tracing = true;
    plugin.calls.clear();
    host.lock().unwrap().frame += 1;
    plugin.frame();
    let port_set: std::collections::BTreeSet<String> = plugin
        .calls
        .iter()
        .map(|name| (*name).to_string())
        .filter(|name| names.values().any(|known| known == name))
        .collect();
    println!(
        "dll only: {:?}",
        dll_set.difference(&port_set).collect::<Vec<_>>()
    );
    println!(
        "port only: {:?}",
        port_set.difference(&dll_set).collect::<Vec<_>>()
    );
    println!("dll {} port {}", dll_set.len(), port_set.len());
    if std::env::var_os("TANTALUS_SFX_ORDER").is_some() {
        let mut last = "";
        let mut line = Vec::new();
        for name in plugin.calls.iter() {
            if names.values().any(|known| known == name) && *name != last {
                line.push(*name);
                last = name;
            }
        }
        println!("port order: {line:?}");
    }
}

/// Host callback commands an effect makes (`TANTALUS_SFX_EFFECT`).
#[test]
#[ignore = "probe"]
fn probe_commands() {
    let id: i32 = std::env::var("TANTALUS_SFX_EFFECT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    {
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        let mut file =
            StandaloneSerialized::open(&common::install().join("x64/FF9_Data/resources.assets"))
                .expect("assets");
        let names: Vec<String> = file
            .text_assets()
            .expect("text")
            .into_iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| name.starts_with("ef") && name.len() == 5)
            .filter(|name| {
                name[2..]
                    .parse::<i32>()
                    .is_ok_and(|n| (95..125).contains(&n) || (311..330).contains(&n))
            })
            .collect();
        println!("names {names:?}");
    }
    let run = run_port(id, 400, 0x1_8000_0000);
    let mut seen: std::collections::BTreeMap<u32, (usize, u32, [i32; 4])> = Default::default();
    for call in &run.calls {
        let entry = seen
            .entry(call.command as u32)
            .or_insert((0, call.frame as u32, call.args));
        entry.0 += 1;
    }
    println!("effect {id}: {} frames", run.frames.len());
    for (command, (count, first, args)) in seen {
        println!("cmd {command}: {count} calls, first frame {first}, args {args:?}");
    }
}
