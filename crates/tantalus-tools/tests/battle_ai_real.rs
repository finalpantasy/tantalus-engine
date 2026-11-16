//! Real-data battle AI check, gated on the owned install. A regression pin of
//! what the host and VM produce, not a value compared against the original
//! runtime.

use std::path::PathBuf;
use tantalus_assets::pc::battle_scene::BattleBundleData;
use tantalus_tools::battle_ai_demo::{demo_report, first_random_attack_scene, load_battle_scripts};

fn streaming_assets() -> Option<PathBuf> {
    let dir = std::env::var_os("TANTALUS_FFIX_STREAMING_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets")
        });
    dir.join("p0data7.bin").is_file().then_some(dir)
}

#[test]
fn first_random_scene_picks_are_stable_and_in_range() {
    let Some(dir) = streaming_assets() else {
        eprintln!("install not found; skipping");
        return;
    };
    let scripts = load_battle_scripts(&dir.join("p0data7.bin")).unwrap();
    let name = first_random_attack_scene(&scripts).unwrap();
    assert_eq!(name, "EVT_BATTLE_AC_E028F");
    let data = BattleBundleData::load(&dir.join("p0data2.bin")).unwrap();
    let (_, scene) = data.scene(&name).unwrap();
    let report = demo_report(&name, &scripts[&name], scene.as_ref().unwrap()).unwrap();
    let enemy = &report["enemies"][0];
    assert_eq!(enemy["btl_id"], 16);
    let picks: Vec<i64> = enemy["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|round| round["attack_index"].as_i64().unwrap())
        .collect();
    assert_eq!(picks, [2, 0, 3, 1, 0, 2]);
    for round in enemy["rounds"].as_array().unwrap() {
        let mask = round["target_mask"].as_u64().unwrap();
        assert!(
            mask != 0 && mask & !0x0F == 0,
            "target outside party: {mask}"
        );
    }
}

#[test]
fn every_battle_script_drives_without_a_host_error() {
    let Some(dir) = streaming_assets() else {
        eprintln!("install not found; skipping");
        return;
    };
    let scripts = load_battle_scripts(&dir.join("p0data7.bin")).unwrap();
    let data = BattleBundleData::load(&dir.join("p0data2.bin")).unwrap();
    let mut driven = 0;
    let mut failures = Vec::new();
    for (name, bytes) in &scripts {
        let Some((_, Ok(scene))) = data.scene(name) else {
            continue;
        };
        match demo_report(name, bytes, scene) {
            Ok(_) => driven += 1,
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    assert!(driven > 100, "only {driven} scripts driven");
    // Two scripts stop in the VM at byte 1061 of entry 1 tag 5 ("missing
    // logical operand"). Unresolved; pinned so a change is noticed.
    let names: Vec<&str> = failures
        .iter()
        .map(|f| f.split(':').next().unwrap())
        .collect();
    assert_eq!(
        names,
        ["EVT_BATTLE_CH_E093", "EVT_BATTLE_CH_E093B"],
        "{failures:?}"
    );
}
