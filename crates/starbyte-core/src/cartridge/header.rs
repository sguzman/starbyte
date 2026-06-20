//! Cartridge header parsing helpers.

use serde::{Deserialize, Serialize};

use super::Mapper;

/// SNES territory identifier normalized to a coarse region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Region {
    /// NTSC-like region.
    Ntsc,
    /// PAL-like region.
    Pal,
    /// Region is unknown or vendor-specific.
    Unknown,
}

/// Parsed SNES cartridge header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartridgeHeader {
    /// Human-readable title.
    pub title: String,
    /// Mapper detected from the candidate slot.
    pub mapper: Mapper,
    /// Raw mapping mode byte.
    pub map_mode: u8,
    /// ROM type byte.
    pub rom_type: u8,
    /// Encoded ROM size exponent.
    pub rom_size_code: u8,
    /// Encoded RAM size exponent.
    pub ram_size_code: u8,
    /// Destination/region byte.
    pub destination_code: u8,
    /// Derived coarse region.
    pub region: Region,
    /// Header checksum complement.
    pub complement: u16,
    /// Header checksum.
    pub checksum: u16,
}

impl CartridgeHeader {
    /// Parse a header from a 0x40-byte candidate slot.
    #[must_use]
    pub fn parse(bytes: &[u8], mapper: Mapper) -> Option<Self> {
        if bytes.len() < 0x20 {
            return None;
        }

        let title = std::str::from_utf8(&bytes[..21])
            .ok()?
            .trim_end_matches('\0')
            .trim_end();
        if title.is_empty()
            || !title
                .bytes()
                .all(|byte| byte == b' ' || byte.is_ascii_graphic())
        {
            return None;
        }

        let complement = u16::from_le_bytes([bytes[0x1C], bytes[0x1D]]);
        let checksum = u16::from_le_bytes([bytes[0x1E], bytes[0x1F]]);

        Some(Self {
            title: title.to_owned(),
            mapper,
            map_mode: bytes[0x15],
            rom_type: bytes[0x16],
            rom_size_code: bytes[0x17],
            ram_size_code: bytes[0x18],
            destination_code: bytes[0x19],
            region: region_from_code(bytes[0x19]),
            complement,
            checksum,
        })
    }

    /// Score the candidate header for mapper selection.
    #[must_use]
    pub fn score(&self) -> u32 {
        let mut score = 0;

        if self.checksum ^ self.complement == 0xFFFF {
            score += 8;
        }

        let mapper_match = matches!(
            (self.mapper, self.map_mode),
            (Mapper::LoRom, 0x20..=0x3F) | (Mapper::HiRom, 0x21..=0x3F)
        );
        if mapper_match {
            score += 4;
        }

        if self.title.len() >= 4 {
            score += 2;
        }

        score
    }

    /// Approximate ROM size in bytes from the size code.
    #[must_use]
    pub fn rom_size_bytes(&self) -> usize {
        0x400usize << usize::from(self.rom_size_code.min(0x1F))
    }

    /// Approximate save RAM size in bytes from the size code.
    #[must_use]
    pub fn ram_size_bytes(&self) -> usize {
        if self.ram_size_code == 0 {
            0
        } else {
            0x400usize << usize::from(self.ram_size_code.min(0x1F))
        }
    }
}

fn region_from_code(code: u8) -> Region {
    match code {
        0x00 | 0x01 | 0x0D => Region::Ntsc,
        0x02..=0x0C | 0x11 => Region::Pal,
        _ => Region::Unknown,
    }
}
