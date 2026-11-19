//! Local-only checks against the owner's real discs. Ignored by default; run with
//! `cargo test -p tantalus-psxaudio -- --ignored` (set `TANTALUS_PSX_DISCS` to the folder
//! that holds the four `.bin` images; the default is the owner's PC path).

use std::path::PathBuf;
use tantalus_psxaudio::akao::{Song, find_akao};
use tantalus_psxaudio::disc::{Disc, find_discs};
use tantalus_psxaudio::img::{GroupKind, Img};

fn disc_dir() -> PathBuf {
    std::env::var_os("TANTALUS_PSX_DISCS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("E:/PSX - Final Fantasy IX"))
}

#[test]
#[ignore = "needs the owner's discs"]
fn lists_music_on_every_disc() {
    let discs = find_discs(&disc_dir());
    assert!(!discs.is_empty(), "no discs found");
    for (n, path) in discs {
        let mut disc = Disc::open(&path).unwrap();
        let img = Img::open(&mut disc).unwrap();
        let mut count = 0;
        let group = img.groups_of(&mut disc, GroupKind::Music).unwrap();
        println!("disc {n} music groups {group:?}");
        for file in img.files.iter().filter(|f| group.contains(&f.group)) {
            let data = img.read(&mut disc, file).unwrap();
            assert!(!find_akao(&data).is_empty());
            let song = match Song::parse(n, file, &data) {
                Ok(song) => song,
                Err(e) => {
                    println!(
                        "disc {n} file {:#x}: {e} (akao at {:?})",
                        file.id,
                        find_akao(&data)
                    );
                    continue;
                }
            };
            println!(
                "disc {n} file {:#x} seq {:#x} bank {:x?} reverb {} channels {} arts {:?} adpcm {:?}",
                file.id,
                song.sequence.id,
                song.bank.as_ref().map(|b| b.id),
                song.sequence.reverb_type,
                song.sequence.channels.len(),
                song.bank.as_ref().map(|b| b.arts.len()),
                song.bank.as_ref().map(|b| b.adpcm.len())
            );
            count += 1;
        }
        println!("disc {n}: {count} songs");
    }
}

#[test]
#[ignore = "needs the owner's discs"]
fn renders_a_song() {
    let discs = find_discs(&disc_dir());
    let (n, path) = &discs[0];
    let mut disc = Disc::open(path).unwrap();
    let img = Img::open(&mut disc).unwrap();
    let group = img.groups_of(&mut disc, GroupKind::Music).unwrap();
    let want: Vec<u32> = std::env::var("SONGS")
        .unwrap_or_else(|_| "0".into())
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    for file in img
        .files
        .iter()
        .filter(|f| group.contains(&f.group) && want.contains(&f.id))
    {
        let data = img.read(&mut disc, file).unwrap();
        let song = Song::parse(*n, file, &data).unwrap();
        let t = std::time::Instant::now();
        let r = tantalus_psxaudio::render::render_song(&song.sequence, song.bank.as_ref().unwrap());
        println!(
            "file {} seq {:#x}: {:.1} s, loop {:?}, peak {:.3} gain {:.3}, stats {:?}, took {:?}",
            file.id,
            song.sequence.id,
            r.seconds(),
            r.loop_frames,
            r.peak,
            r.gain,
            r.stats,
            t.elapsed()
        );
    }
}

#[test]
#[ignore = "needs the owner's discs"]
fn reads_the_resident_ui_art() {
    use tantalus_psxaudio::ui_art::{UiArt, glyph_code, read_ui_file};
    let file = read_ui_file(&disc_dir()).unwrap();
    let art = UiArt::parse(&file).unwrap();
    assert_eq!((art.font.width, art.font.height), (256, 120));
    assert_eq!((art.icons.width, art.window.width), (256, 256));
    assert!(art.glyphs.len() > 100);
    // `A` is 9 pixels wide in the pen, `1` sits 2 pixels right inside its 8-pixel cell.
    let a = art.glyphs[glyph_code('A').unwrap()];
    assert_eq!((a.x, a.y, a.advance), (118, 0, 9));
    let one = art.glyphs[glyph_code('1').unwrap()];
    assert_eq!((one.offset, one.width, one.advance), (2, 4, 8));
}
