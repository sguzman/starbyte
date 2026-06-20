//! Cartridge loading and mapper metadata.

mod header;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::error::{Error, Result};

pub use self::header::{CartridgeHeader, Region};

const SMC_HEADER_LEN: usize = 512;

/// Supported SNES mapper families for the bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mapper {
    /// LoROM / SlowROM style mapping.
    LoRom,
    /// HiROM style mapping.
    HiRom,
}

impl Mapper {
    fn header_base(self) -> usize {
        match self {
            Self::LoRom => 0x7FC0,
            Self::HiRom => 0xFFC0,
        }
    }
}

/// In-memory cartridge representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cartridge {
    rom: Vec<u8>,
    source: Option<PathBuf>,
    header: CartridgeHeader,
    mapper: Mapper,
}

impl Cartridge {
    /// Load a cartridge from the given path.
    #[instrument(skip_all)]
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::io(path, source))?;
        Self::from_bytes(bytes, Some(path.to_path_buf()))
    }

    /// Load a cartridge from owned bytes.
    #[instrument(skip_all)]
    pub fn from_bytes(bytes: Vec<u8>, source: Option<PathBuf>) -> Result<Self> {
        let (rom, had_smc_header) = strip_smc_header(bytes)?;
        let (mapper, header) = detect_header(&rom)?;
        debug!(
            ?mapper,
            had_smc_header,
            title = header.title,
            "loaded cartridge header"
        );

        Ok(Self {
            rom,
            source,
            header,
            mapper,
        })
    }

    /// ROM bytes excluding any copier header.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Parsed cartridge header.
    #[must_use]
    pub const fn header(&self) -> &CartridgeHeader {
        &self.header
    }

    /// Detected mapper family.
    #[must_use]
    pub const fn mapper(&self) -> Mapper {
        self.mapper
    }

    /// Original load path if known.
    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }
}

fn strip_smc_header(bytes: Vec<u8>) -> Result<(Vec<u8>, bool)> {
    if bytes.len() < 0x8000 {
        return Err(Error::InvalidRom("ROM is smaller than 32 KiB".to_owned()));
    }

    if bytes.len() % 1024 == SMC_HEADER_LEN {
        return Ok((bytes[SMC_HEADER_LEN..].to_vec(), true));
    }

    Ok((bytes, false))
}

fn detect_header(rom: &[u8]) -> Result<(Mapper, CartridgeHeader)> {
    let mut best: Option<(Mapper, CartridgeHeader, u32)> = None;

    for mapper in [Mapper::LoRom, Mapper::HiRom] {
        let Some(slice) = rom.get(mapper.header_base()..mapper.header_base() + 0x40) else {
            continue;
        };

        if let Some(header) = CartridgeHeader::parse(slice, mapper) {
            let score = header.score();
            match &best {
                Some((_, _, best_score)) if *best_score >= score => {}
                _ => best = Some((mapper, header, score)),
            }
        }
    }

    best.map(|(mapper, header, _)| (mapper, header))
        .ok_or_else(|| Error::InvalidRom("unable to detect a valid SNES header".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{Cartridge, Mapper};

    fn make_header(mapper: Mapper) -> Vec<u8> {
        let base = match mapper {
            Mapper::LoRom => 0x7FC0,
            Mapper::HiRom => 0xFFC0,
        };
        let mut rom = vec![0_u8; 0x10000];
        let title = b"STARBYTE TEST        ";
        rom[base..base + 21].copy_from_slice(title);
        rom[base + 0x15] = match mapper {
            Mapper::LoRom => 0x20,
            Mapper::HiRom => 0x21,
        };
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x03;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1A] = 0x33;
        rom[base + 0x1B] = 0x00;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        rom
    }

    #[test]
    fn detects_lorom_header() {
        let cart = Cartridge::from_bytes(make_header(Mapper::LoRom), None).unwrap();
        assert_eq!(cart.mapper(), Mapper::LoRom);
        assert_eq!(cart.header().title.trim(), "STARBYTE TEST");
    }

    #[test]
    fn strips_copier_header() {
        let mut raw = vec![0_u8; 512];
        raw.extend(make_header(Mapper::HiRom));
        let cart = Cartridge::from_bytes(raw, None).unwrap();
        assert_eq!(cart.mapper(), Mapper::HiRom);
        assert_eq!(cart.rom().len(), 0x10000);
    }
}
