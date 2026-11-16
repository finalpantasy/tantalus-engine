//! Cutscene movies: which file a script's `FMV` request means.
//!
//! `MBG.Seek(disc, fmv)` (`Global/MBG.cs:65`) looks the name up in `MBG.MBGDiscTable`; the PC
//! build keeps a movie as an Ogg Theora file `StreamingAssets/ma/<name>.bytes`. Table rows that
//! name a PlayStation `.STR` path or an in-engine `mbg###` scene have no such file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `(disc, fmv)` to the table's name, from the `MBGDiscTable` initializer in `MBG.cs`.
pub fn parse_disc_table(source: &str) -> BTreeMap<(i32, i32), String> {
    let mut table = BTreeMap::new();
    let Some(start) = source.find("MBGDiscTable = new MBG_DEF[][]") else {
        return table;
    };
    let mut disc = -1;
    let mut number = 0;
    for line in source[start..].lines().skip(1) {
        let line = line.trim();
        if line.starts_with("};") {
            break;
        }
        if line.starts_with("new MBG_DEF[]") {
            disc += 1;
            number = 0;
        } else if let Some(rest) = line.strip_prefix("new MBG_DEF(\"")
            && let Some(end) = rest.find('"')
        {
            table.insert((disc, number), rest[..end].to_owned());
            number += 1;
        }
    }
    table
}

/// `StreamingAssets/ma/<name>.bytes` when it exists.
pub fn movie_file(streaming_assets: &Path, name: &str) -> Option<PathBuf> {
    let path = streaming_assets.join("ma").join(format!("{name}.bytes"));
    path.is_file().then_some(path)
}

/// A movie a script started and the play loop should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieRequest {
    pub disc: i32,
    pub number: i32,
    pub name: String,
    pub path: PathBuf,
}

impl MovieRequest {
    /// The sound resource the movie plays (`SoundLib.GetMovieSoundIndex`; the first
    /// localization where it depends on one), when the sound files have it.
    pub fn sound_resource(&self) -> Option<String> {
        tantalus_assets::pc::audio_decode::movie_sound_choices(&self.name)
            .into_iter()
            .next()
            .map(|choice| choice.resource_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_table_rows_are_numbered_per_disc() {
        let source = r#"
    public static readonly MBG_DEF[][] MBGDiscTable = new MBG_DEF[][]
    {
        new MBG_DEF[]
        {
            new MBG_DEF("\SPARE\FMV090.STR;1", 1, 0),
            new MBG_DEF("FMV091", 1, 0)
        },
        new MBG_DEF[]
        {
            new MBG_DEF("mbg101", 0, 1),
            new MBG_DEF("FMV001", 1, 0)
        }
    };
    other new MBG_DEF("ignored", 1, 0)
"#;
        let table = parse_disc_table(source);
        assert_eq!(table.len(), 4);
        assert_eq!(table[&(0, 1)], "FMV091");
        assert_eq!(table[&(1, 1)], "FMV001");
    }
}
