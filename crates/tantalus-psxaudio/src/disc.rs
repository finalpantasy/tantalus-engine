//! Read-only CD-ROM image access: raw `MODE2/2352` BIN files and an ISO 9660 directory walk.
//!
//! A raw sector is 2352 bytes: 12 sync, 4 header, 8 sub-header, then 2048 bytes of user data for
//! a Form 1 sector (the only kind ISO 9660 files use). The data starts at byte 24.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const RAW_SECTOR: u64 = 2352;
pub const DATA_SECTOR: usize = 2048;
const DATA_OFFSET: u64 = 24;

/// An open disc image.
pub struct Disc {
    file: File,
    sectors: u64,
}

/// A file found by [`Disc::find_file`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscFile {
    pub lba: u32,
    pub size: u32,
}

impl Disc {
    /// Opens a `.bin` image (or a `.cue`, whose single track file is used).
    pub fn open(path: &Path) -> Result<Self, String> {
        let path = if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
        {
            cue_track_file(path)?
        } else {
            path.to_owned()
        };
        let file = File::open(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len < 17 * RAW_SECTOR || len % RAW_SECTOR != 0 {
            return Err(format!(
                "{} is not a raw 2352-byte-sector image ({len} bytes)",
                path.display()
            ));
        }
        Ok(Self {
            file,
            sectors: len / RAW_SECTOR,
        })
    }

    pub fn sector_count(&self) -> u64 {
        self.sectors
    }

    /// The 2048 user bytes of one sector.
    pub fn read_sector(&mut self, lba: u32) -> Result<[u8; DATA_SECTOR], String> {
        if u64::from(lba) >= self.sectors {
            return Err(format!("sector {lba} is past the end of the image"));
        }
        self.file
            .seek(SeekFrom::Start(u64::from(lba) * RAW_SECTOR + DATA_OFFSET))
            .map_err(|e| e.to_string())?;
        let mut out = [0u8; DATA_SECTOR];
        self.file.read_exact(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// `len` bytes starting at the first byte of sector `lba`.
    pub fn read(&mut self, lba: u32, len: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(len.next_multiple_of(DATA_SECTOR));
        let mut sector = lba;
        while out.len() < len {
            out.extend_from_slice(&self.read_sector(sector)?);
            sector += 1;
        }
        out.truncate(len);
        Ok(out)
    }

    /// Finds a file by path (`/` or `\` separated, case-insensitive, `;1` suffix optional).
    pub fn find_file(&mut self, path: &str) -> Result<DiscFile, String> {
        let pvd = self.read_sector(16)?;
        if &pvd[1..6] != b"CD001" {
            return Err("no ISO 9660 primary volume descriptor at sector 16".to_owned());
        }
        let root = &pvd[156..190];
        let mut dir = DiscFile {
            lba: u32::from_le_bytes(root[2..6].try_into().expect("4 bytes")),
            size: u32::from_le_bytes(root[10..14].try_into().expect("4 bytes")),
        };
        let parts: Vec<&str> = path
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .collect();
        for (index, part) in parts.iter().enumerate() {
            let entries = self.read_dir(dir)?;
            let wanted = part.to_ascii_uppercase();
            let entry = entries
                .into_iter()
                .find(|(name, _, _)| {
                    let name = name.to_ascii_uppercase();
                    name == wanted || name.strip_suffix(";1") == Some(wanted.as_str())
                })
                .ok_or_else(|| format!("`{path}` not found on the disc (missing `{part}`)"))?;
            let last = index + 1 == parts.len();
            if entry.2 == last {
                return Err(format!(
                    "`{part}` is {} in `{path}`",
                    if last {
                        "a directory"
                    } else {
                        "not a directory"
                    }
                ));
            }
            dir = entry.1;
        }
        Ok(dir)
    }

    /// Directory records: `(name, file, is_dir)`.
    pub fn read_dir(&mut self, dir: DiscFile) -> Result<Vec<(String, DiscFile, bool)>, String> {
        let data = self.read(dir.lba, dir.size as usize)?;
        let mut out = Vec::new();
        let mut at = 0usize;
        while at < data.len() {
            let len = data[at] as usize;
            if len == 0 {
                // Records never straddle a sector; the rest of this one is padding.
                at = (at / DATA_SECTOR + 1) * DATA_SECTOR;
                continue;
            }
            if at + len > data.len() || len < 34 {
                return Err("corrupt ISO 9660 directory record".to_owned());
            }
            let rec = &data[at..at + len];
            let name_len = rec[32] as usize;
            let name = &rec[33..33 + name_len];
            if name != [0] && name != [1] {
                out.push((
                    String::from_utf8_lossy(name).into_owned(),
                    DiscFile {
                        lba: u32::from_le_bytes(rec[2..6].try_into().expect("4 bytes")),
                        size: u32::from_le_bytes(rec[10..14].try_into().expect("4 bytes")),
                    },
                    rec[25] & 2 != 0,
                ));
            }
            at += len;
        }
        Ok(out)
    }
}

/// The first `FILE "..."` of a cue sheet, next to the cue.
fn cue_track_file(cue: &Path) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(cue).map_err(|e| format!("{}: {e}", cue.display()))?;
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("FILE ")
            && let Some(start) = rest.find('"')
            && let Some(end) = rest[start + 1..].find('"')
        {
            return Ok(cue.with_file_name(&rest[start + 1..start + 1 + end]));
        }
    }
    Err(format!("{} names no track file", cue.display()))
}

/// The disc images of a directory, `(disc number 1..=4, path)`, found by their `Disc<N>of4`
/// file names (`.bin` only; a `.cue` next to it adds nothing for a single-track image).
pub fn find_discs(dir: &Path) -> Vec<(u8, PathBuf)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
        else {
            continue;
        };
        if !name.ends_with(".bin") {
            continue;
        }
        for n in 1..=4u8 {
            if name.contains(&format!("disc{n}of4")) {
                found.push((n, path.clone()));
            }
        }
    }
    found.sort();
    found
}
