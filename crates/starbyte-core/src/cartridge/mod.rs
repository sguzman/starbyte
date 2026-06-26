//! Cartridge loading and mapper metadata.

mod header;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::coprocessor::CoprocessorKind;
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

    /// Detected coprocessor family from the cartridge chipset byte, if any.
    #[must_use]
    pub fn coprocessor_kind(&self) -> Option<CoprocessorKind> {
        CoprocessorKind::detect(&self.header)
    }

    /// Read one ROM byte through the detected CPU address mapping.
    #[must_use]
    pub fn read_byte(&self, address: u32) -> Option<u8> {
        let index = match self.mapper {
            Mapper::LoRom => map_lorom(address),
            Mapper::HiRom => map_hirom(address),
        }?;
        self.rom.get(index % self.rom.len()).copied()
    }

    /// Read the reset vector from the cartridge image if the mapping exposes it.
    #[must_use]
    pub fn reset_vector(&self) -> Option<u16> {
        let lo = self.read_byte(0x00FFFC)?;
        let hi = self.read_byte(0x00FFFD)?;
        Some(u16::from_le_bytes([lo, hi]))
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

fn map_lorom(address: u32) -> Option<usize> {
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;
    let bank_index = usize::from(bank & 0x7F);

    match bank {
        0x00..=0x3F | 0x80..=0xBF if offset >= 0x8000 => {
            Some(bank_index * 0x8000 + usize::from(offset - 0x8000))
        }
        0x40..=0x7D | 0xC0..=0xFF => Some(bank_index * 0x8000 + usize::from(offset & 0x7FFF)),
        _ => None,
    }
}

fn map_hirom(address: u32) -> Option<usize> {
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;
    let bank_index = usize::from(bank & 0x3F);

    match bank {
        0x00..=0x3F | 0x80..=0xBF if offset >= 0x8000 => {
            Some(bank_index * 0x10000 + usize::from(offset))
        }
        0x40..=0x7D | 0xC0..=0xFF => Some(bank_index * 0x10000 + usize::from(offset)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::coprocessor::CoprocessorKind;

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
        assert_eq!(cart.coprocessor_kind(), None);
    }

    #[test]
    fn strips_copier_header() {
        let mut raw = vec![0_u8; 512];
        raw.extend(make_header(Mapper::HiRom));
        let cart = Cartridge::from_bytes(raw, None).unwrap();
        assert_eq!(cart.mapper(), Mapper::HiRom);
        assert_eq!(cart.rom().len(), 0x10000);
    }

    #[test]
    fn maps_lorom_rom_reads() {
        let mut rom = make_header(Mapper::LoRom);
        rom[0] = 0x12;
        rom[0x7FFF] = 0x34;
        let cart = Cartridge::from_bytes(rom, None).unwrap();

        assert_eq!(cart.read_byte(0x008000), Some(0x12));
        assert_eq!(cart.read_byte(0x00FFFF), Some(0x34));
        assert_eq!(cart.read_byte(0x7E0000), None);
    }

    #[test]
    fn maps_hirom_rom_reads() {
        let mut rom = vec![0_u8; 0x20000];
        let header = make_header(Mapper::HiRom);
        rom[..header.len()].copy_from_slice(&header);
        rom[0x008000] = 0x56;
        rom[0x01FFFF] = 0x78;
        let cart = Cartridge::from_bytes(rom, None).unwrap();

        assert_eq!(cart.read_byte(0x408000), Some(0x56));
        assert_eq!(cart.read_byte(0x41FFFF), Some(0x78));
        assert_eq!(cart.read_byte(0x001000), None);
    }

    #[test]
    fn detects_dsp_chipset_from_header() {
        let mut rom = make_header(Mapper::LoRom);
        rom[0x7FC0 + 0x16] = 0x03;
        let cart = Cartridge::from_bytes(rom, None).unwrap();
        assert_eq!(cart.coprocessor_kind(), Some(CoprocessorKind::Dsp));
    }

    #[test]
    fn detects_additional_coprocessor_families_from_header() {
        let mut sa1 = make_header(Mapper::LoRom);
        sa1[0x7FC0..0x7FC0 + 21].copy_from_slice(b"STARBYTE SA-1 TEST   ");
        sa1[0x7FC0 + 0x16] = 0x34;

        let mut cx4 = make_header(Mapper::LoRom);
        cx4[0x7FC0..0x7FC0 + 21].copy_from_slice(b"STARBYTE CX4 TEST    ");
        cx4[0x7FC0 + 0x16] = 0xF3;

        let mut sdd1 = make_header(Mapper::LoRom);
        sdd1[0x7FC0..0x7FC0 + 21].copy_from_slice(b"STARBYTE S-DD1 TEST  ");
        sdd1[0x7FC0 + 0x16] = 0x43;

        let mut obc1 = make_header(Mapper::LoRom);
        obc1[0x7FC0..0x7FC0 + 21].copy_from_slice(b"STARBYTE OBC1 TEST   ");
        obc1[0x7FC0 + 0x16] = 0x23;

        let mut srtc = make_header(Mapper::LoRom);
        srtc[0x7FC0..0x7FC0 + 21].copy_from_slice(b"STARBYTE SRTC TEST   ");
        srtc[0x7FC0 + 0x16] = 0x53;

        assert_eq!(
            Cartridge::from_bytes(sa1, None).unwrap().coprocessor_kind(),
            Some(CoprocessorKind::Sa1)
        );
        assert_eq!(
            Cartridge::from_bytes(cx4, None).unwrap().coprocessor_kind(),
            Some(CoprocessorKind::Cx4)
        );
        assert_eq!(
            Cartridge::from_bytes(sdd1, None)
                .unwrap()
                .coprocessor_kind(),
            Some(CoprocessorKind::Sdd1)
        );
        assert_eq!(
            Cartridge::from_bytes(obc1, None)
                .unwrap()
                .coprocessor_kind(),
            Some(CoprocessorKind::Obc1)
        );
        assert_eq!(
            Cartridge::from_bytes(srtc, None)
                .unwrap()
                .coprocessor_kind(),
            Some(CoprocessorKind::SRtc)
        );
    }
}
