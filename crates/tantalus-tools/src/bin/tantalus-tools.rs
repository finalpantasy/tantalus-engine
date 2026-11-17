//! `tantalus-tools`: small utilities that are not part of the game or `tantalusctl`.
//!
//! `psx-audio` renders the PS1 music and sound effects from the owner's own discs.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("psx-audio") => tantalus_tools::psx_audio::run(&args[1..]),
        _ => Err("usage: tantalus-tools psx-audio [render|compare|coverage] --disc-dir <dir> [--out <dir>] [--steam <install>] [--ids 0,3]".to_owned()),
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(2);
    }
}
