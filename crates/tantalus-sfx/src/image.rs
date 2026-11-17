//! The data sections of the player's `FF9SpecialEffectPlugin.dll`, read by
//! RVA. The effect runtime starts from tables and templates the original
//! keeps in its `.data` and `.rdata` (Memoria's `ModuleData.g.cs` lists them by
//! address); Tantalus reads them from the player's own install and never
//! ships them. Only bytes are read: no code in the DLL is run.

/// A PE image mapped by section.
#[derive(Debug, Clone)]
pub struct PluginImage {
    bytes: Vec<u8>,
    /// (RVA, virtual size, file offset, raw size) of each section.
    sections: Vec<(u32, u32, u32, u32)>,
    image_base: u64,
}

impl PluginImage {
    /// Parses the PE headers of `bytes`; `None` when they are not a PE32+
    /// image.
    pub fn parse(bytes: Vec<u8>) -> Option<Self> {
        let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
        let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
        let pe = u32_at(0x3C)? as usize;
        if bytes.get(pe..pe + 4)? != b"PE\x00\x00" {
            return None;
        }
        let count = usize::from(u16_at(pe + 6)?);
        let optional = usize::from(u16_at(pe + 20)?);
        let image_base = if optional >= 32 && u16_at(pe + 24)? == 0x20B {
            u64::from_le_bytes(bytes.get(pe + 48..pe + 56)?.try_into().ok()?)
        } else {
            0
        };
        let table = pe + 24 + optional;
        let mut sections = Vec::with_capacity(count);
        for index in 0..count {
            let at = table + index * 40;
            sections.push((
                u32_at(at + 12)?,
                u32_at(at + 8)?,
                u32_at(at + 20)?,
                u32_at(at + 16)?,
            ));
        }
        Some(Self {
            bytes,
            sections,
            image_base,
        })
    }

    /// `len` bytes at `rva`; bytes past a section's raw data (its BSS) read
    /// as zero. `None` when the range leaves every section.
    pub fn read(&self, rva: u32, len: usize) -> Option<Vec<u8>> {
        let (start, size, raw, raw_size) =
            *self.sections.iter().find(|(start, size, _, raw_size)| {
                rva >= *start && rva < start + (*size).max(*raw_size)
            })?;
        let end = u64::from(rva) + len as u64;
        if end > u64::from(start) + u64::from(size.max(raw_size)) {
            return None;
        }
        let mut out = vec![0u8; len];
        let offset = rva - start;
        if offset < raw_size {
            let available = ((raw_size - offset) as usize).min(len);
            let from = (raw + offset) as usize;
            out[..available].copy_from_slice(self.bytes.get(from..from + available)?);
        }
        Some(out)
    }

    /// (RVA, mapped size) of every section.
    pub fn section_ranges(&self) -> Vec<(u32, u32)> {
        self.sections
            .iter()
            .map(|&(start, size, _, raw)| (start, size.max(raw)))
            .collect()
    }

    /// One past the last mapped byte.
    pub fn image_end(&self) -> Option<u32> {
        self.section_ranges()
            .iter()
            .map(|(start, size)| start + size)
            .max()
    }

    /// (RVA, virtual size) of the section holding `rva`.
    pub fn section_of(&self, rva: u32) -> Option<(u32, u32)> {
        self.sections
            .iter()
            .find(|(start, size, _, raw)| rva >= *start && rva < start + (*size).max(*raw))
            .map(|&(start, size, _, raw)| (start, size.max(raw)))
    }

    /// The RVA of virtual address `va` (`0x1800xxxxx` in Memoria's names).
    pub fn rva(&self, va: u64) -> Option<u32> {
        u32::try_from(va.checked_sub(self.image_base)?).ok()
    }
}
