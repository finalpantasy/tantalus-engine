//! `FF9.IMG`, the one big archive on every FF9 disc.
//!
//! Layout, worked out from the discs (2048-byte blocks):
//! - block 0: `"FF9 "`, a version word (6), the group count, then one 16-byte record per group:
//!   `(kind, file_count, group_id, first_block)`.
//! - from block 1 the groups' file tables follow back to back, each padded to a whole block:
//!   `file_count + 1` pairs `(file_id, start_block)`; the last pair (`0xFFFFFFFF`) marks the end.
//!
//! File `i` of a group spans `start_block[i] .. start_block[i+1]`.

use crate::disc::{DATA_SECTOR, Disc};

/// One archive file's place in the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImgFile {
    pub group: u32,
    pub index: usize,
    pub id: u32,
    pub start_block: u32,
    pub blocks: u32,
}

/// What a group of files holds, as far as audio goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// One song per file: a sample collection at byte 128 and the sequence after it.
    Music,
    /// Small sample collections for sound effects (`AKAO` at byte 64).
    SfxBanks,
    Other,
}

pub struct Img {
    pub base_lba: u32,
    pub files: Vec<ImgFile>,
}

fn word(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().expect("4 bytes"))
}

impl Img {
    /// Opens `FF9.IMG` on the disc and reads its directory.
    pub fn open(disc: &mut Disc) -> Result<Self, String> {
        let file = disc.find_file("FF9.IMG")?;
        let head = disc.read(file.lba, DATA_SECTOR)?;
        if &head[0..4] != b"FF9 " {
            return Err("FF9.IMG does not start with `FF9 `".to_owned());
        }
        let groups = word(&head, 8) as usize;
        if groups == 0 || groups > 60 {
            return Err(format!("implausible FF9.IMG group count {groups}"));
        }
        let records: Vec<(usize, u32)> = (0..groups)
            .map(|g| {
                let at = 0x10 + g * 16;
                (word(&head, at + 4) as usize, word(&head, at + 8))
            })
            .collect();
        let table_blocks: usize = records
            .iter()
            .map(|(n, _)| ((n + 1) * 8).div_ceil(DATA_SECTOR))
            .sum();
        let tables = disc.read(file.lba + 1, table_blocks * DATA_SECTOR)?;
        let mut files = Vec::new();
        let mut pos = 0usize;
        for &(count, group) in &records {
            let size = ((count + 1) * 8).div_ceil(DATA_SECTOR) * DATA_SECTOR;
            let table = &tables[pos..pos + size];
            pos += size;
            let pair = |i: usize| (word(table, i * 8), word(table, i * 8 + 4));
            for index in 0..count {
                let (id, start) = pair(index);
                let (_, next) = pair(index + 1);
                files.push(ImgFile {
                    group,
                    index,
                    id,
                    start_block: start,
                    blocks: next.saturating_sub(start),
                });
            }
        }
        Ok(Self {
            base_lba: file.lba,
            files,
        })
    }

    /// Group ids are not stable between discs (disc 2 and 3 carry an extra group, which shifts
    /// the later ids by one), so the audio groups are found by what their files start with.
    pub fn classify(&self, disc: &mut Disc, group: u32) -> Result<GroupKind, String> {
        let Some(first) = self.group(group).find(|f| f.blocks > 0) else {
            return Ok(GroupKind::Other);
        };
        // Groups of the odd kinds (overlays) have no valid block numbers: not audio.
        let Ok(head) = disc.read(self.base_lba + first.start_block, 256) else {
            return Ok(GroupKind::Other);
        };
        let at = |o: usize| head.get(o..o + 4) == Some(b"AKAO");
        Ok(if at(128) {
            GroupKind::Music
        } else if at(64) {
            GroupKind::SfxBanks
        } else {
            GroupKind::Other
        })
    }

    /// The group ids of a kind.
    pub fn groups_of(&self, disc: &mut Disc, kind: GroupKind) -> Result<Vec<u32>, String> {
        let mut ids: Vec<u32> = self.files.iter().map(|f| f.group).collect();
        ids.dedup();
        let mut out = Vec::new();
        for id in ids {
            if self.classify(disc, id)? == kind {
                out.push(id);
            }
        }
        Ok(out)
    }

    pub fn group(&self, group: u32) -> impl Iterator<Item = &ImgFile> {
        self.files.iter().filter(move |f| f.group == group)
    }

    pub fn read(&self, disc: &mut Disc, file: &ImgFile) -> Result<Vec<u8>, String> {
        disc.read(
            self.base_lba + file.start_block,
            file.blocks as usize * DATA_SECTOR,
        )
    }
}
