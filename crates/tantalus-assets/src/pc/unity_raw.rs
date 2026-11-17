use crate::install::ImportError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnityRawEntry {
    pub path: String,
    pub relative_offset: u32,
    pub size: u32,
    pub absolute_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnityRawReport {
    pub format_version: u32,
    pub player_version: String,
    pub engine_version: String,
    pub header_size: u32,
    pub streamed_content_size: u32,
    pub complete_file_size: u32,
    pub data_offset: u64,
    pub entries: Vec<UnityRawEntry>,
}

pub struct UnityRawBundle {
    path: PathBuf,
    report: UnityRawReport,
}

impl UnityRawBundle {
    pub fn open(path: &Path) -> Result<Self, ImportError> {
        let mut file =
            File::open(path).map_err(|source| ImportError::io("open UnityRaw", path, source))?;
        let file_length = file
            .metadata()
            .map_err(|source| ImportError::io("inspect UnityRaw metadata", path, source))?
            .len();
        let signature = read_c_string(&mut file, path, 32)?;
        if signature != "UnityRaw" {
            return Err(ImportError::invalid(
                path.display(),
                format!("expected UnityRaw signature, found `{signature}`"),
            ));
        }
        let format_version = read_u32(&mut file, path)?;
        if format_version != 3 {
            return Err(ImportError::unsupported(
                path.display(),
                format!("UnityRaw format {format_version}; only version 3 is implemented"),
            ));
        }
        let player_version = read_c_string(&mut file, path, 256)?;
        let engine_version = read_c_string(&mut file, path, 256)?;
        let _minimum_streamed_bytes = read_u32(&mut file, path)?;
        let header_size = read_u32(&mut file, path)?;
        let _levels_before_streaming = read_u32(&mut file, path)?;
        let level_count = read_u32(&mut file, path)?;
        if level_count == 0 {
            return Err(ImportError::invalid(
                path.display(),
                "level count must be nonzero",
            ));
        }
        let skip = u64::from(level_count - 1)
            .checked_mul(8)
            .ok_or_else(|| ImportError::invalid(path.display(), "level table size overflow"))?;
        file.seek(SeekFrom::Current(skip as i64))
            .map_err(|source| ImportError::io("skip UnityRaw level table", path, source))?;
        let streamed_content_size = read_u32(&mut file, path)?;
        let _uncompressed_streamed_content_size = read_u32(&mut file, path)?;
        let complete_file_size = read_u32(&mut file, path)?;
        let _file_info_header_size = read_u32(&mut file, path)?;
        if u64::from(complete_file_size) != file_length {
            return Err(ImportError::invalid(
                path.display(),
                format!("header declares {complete_file_size} bytes, file contains {file_length}"),
            ));
        }
        if u64::from(header_size) >= file_length {
            return Err(ImportError::invalid(
                path.display(),
                "header lies outside the file",
            ));
        }
        file.seek(SeekFrom::Start(u64::from(header_size)))
            .map_err(|source| ImportError::io("seek UnityRaw directory", path, source))?;
        let directory_end = u64::from(header_size) + u64::from(streamed_content_size);
        if directory_end > file_length {
            return Err(ImportError::invalid(
                path.display(),
                "directory is truncated",
            ));
        }
        let entry_count = read_i32(&mut file, path)?;
        let entry_count = usize::try_from(entry_count)
            .map_err(|_| ImportError::invalid(path.display(), "negative directory entry count"))?;
        if entry_count > 1_024 {
            return Err(ImportError::invalid(
                path.display(),
                "directory exceeds 1024 entries",
            ));
        }
        let data_offset = u64::from(header_size);
        let mut entries = Vec::with_capacity(entry_count);
        let mut names = BTreeSet::new();
        for _ in 0..entry_count {
            let entry_path = read_c_string(&mut file, path, 4_096)?;
            let relative_offset = read_u32(&mut file, path)?;
            let size = read_u32(&mut file, path)?;
            if !names.insert(entry_path.clone()) {
                return Err(ImportError::invalid(
                    path.display(),
                    format!("duplicate bundle path `{entry_path}`"),
                ));
            }
            let absolute_offset = data_offset + u64::from(relative_offset);
            let end = absolute_offset
                .checked_add(u64::from(size))
                .ok_or_else(|| {
                    ImportError::invalid(path.display(), "bundle entry range overflow")
                })?;
            if end > file_length {
                return Err(ImportError::invalid(
                    path.display(),
                    format!("bundle entry `{entry_path}` lies outside the file"),
                ));
            }
            entries.push(UnityRawEntry {
                path: entry_path,
                relative_offset,
                size,
                absolute_offset,
            });
        }
        if file
            .stream_position()
            .map_err(|source| ImportError::io("finish UnityRaw directory", path, source))?
            > directory_end
        {
            return Err(ImportError::invalid(
                path.display(),
                "directory exceeds its declared size",
            ));
        }
        Ok(Self {
            path: path.to_path_buf(),
            report: UnityRawReport {
                format_version,
                player_version,
                engine_version,
                header_size,
                streamed_content_size,
                complete_file_size,
                data_offset,
                entries,
            },
        })
    }

    pub fn report(&self) -> &UnityRawReport {
        &self.report
    }

    /// Where entry `index` sits in the file: (absolute offset, size), for
    /// callers that map the file instead of copying the entry out.
    pub fn entry_span(&self, index: usize) -> Option<(u64, u64)> {
        let entry = self.report.entries.get(index)?;
        Some((entry.absolute_offset, u64::from(entry.size)))
    }

    /// Entry `index` mapped from the file rather than copied out: the
    /// bundles are hundreds of megabytes stored uncompressed, so this skips
    /// the copy.
    pub fn map_entry(
        &self,
        index: usize,
        maximum_bytes: usize,
    ) -> Result<MappedEntry, ImportError> {
        let entry = self.report.entries.get(index).ok_or_else(|| {
            ImportError::invalid(self.path.display(), format!("unknown bundle entry {index}"))
        })?;
        if entry.size as usize > maximum_bytes {
            return Err(ImportError::invalid(
                self.path.display(),
                format!("entry `{}` exceeds the read budget", entry.path),
            ));
        }
        let file = File::open(&self.path)
            .map_err(|source| ImportError::io("open UnityRaw entry", &self.path, source))?;
        // SAFETY: the game files are not modified while a tool or the client runs.
        let map = unsafe { memmap2::Mmap::map(&file) }
            .map_err(|source| ImportError::io("map UnityRaw entry", &self.path, source))?;
        let start = entry.absolute_offset as usize;
        let end = start + entry.size as usize;
        if end > map.len() {
            return Err(ImportError::invalid(
                self.path.display(),
                format!("entry `{}` lies outside the file", entry.path),
            ));
        }
        Ok(MappedEntry { map, start, end })
    }

    pub fn read_entry(&self, index: usize, maximum_bytes: usize) -> Result<Vec<u8>, ImportError> {
        let entry = self.report.entries.get(index).ok_or_else(|| {
            ImportError::invalid(self.path.display(), format!("unknown bundle entry {index}"))
        })?;
        if entry.size as usize > maximum_bytes {
            return Err(ImportError::invalid(
                self.path.display(),
                format!("entry `{}` exceeds the read budget", entry.path),
            ));
        }
        let mut file = File::open(&self.path)
            .map_err(|source| ImportError::io("open UnityRaw entry", &self.path, source))?;
        file.seek(SeekFrom::Start(entry.absolute_offset))
            .map_err(|source| ImportError::io("seek UnityRaw entry", &self.path, source))?;
        let mut bytes = vec![0; entry.size as usize];
        file.read_exact(&mut bytes)
            .map_err(|source| ImportError::io("read UnityRaw entry", &self.path, source))?;
        Ok(bytes)
    }
}

fn read_u32(file: &mut File, path: &Path) -> Result<u32, ImportError> {
    Ok(u32::from_be_bytes(read_array(file, path)?))
}

fn read_i32(file: &mut File, path: &Path) -> Result<i32, ImportError> {
    Ok(i32::from_be_bytes(read_array(file, path)?))
}

fn read_array<const N: usize>(file: &mut File, path: &Path) -> Result<[u8; N], ImportError> {
    let mut bytes = [0; N];
    file.read_exact(&mut bytes)
        .map_err(|source| ImportError::io("read UnityRaw integer", path, source))?;
    Ok(bytes)
}

fn read_c_string(file: &mut File, path: &Path, maximum: usize) -> Result<String, ImportError> {
    let mut bytes = Vec::new();
    for _ in 0..maximum {
        let byte = read_array::<1>(file, path)?[0];
        if byte == 0 {
            return String::from_utf8(bytes).map_err(|error| {
                ImportError::invalid(
                    path.display(),
                    format!("non-UTF-8 UnityRaw string: {error}"),
                )
            });
        }
        bytes.push(byte);
    }
    Err(ImportError::invalid(
        path.display(),
        "unterminated UnityRaw string",
    ))
}

/// A bundle entry mapped from its file (see [`UnityRawBundle::map_entry`]).
pub struct MappedEntry {
    map: memmap2::Mmap,
    start: usize,
    end: usize,
}

impl std::ops::Deref for MappedEntry {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.map[self.start..self.end]
    }
}
