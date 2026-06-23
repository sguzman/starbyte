use std::path::PathBuf;

use anyhow::{Context, Result};

use starbyte_core::{
    Emulator, EmulatorBuilder,
    cartridge::Cartridge,
    input::ControllerState,
    manifest::AssetConfig,
};

#[derive(Debug)]
pub struct EmulatorSession {
    emulator: Emulator,
    rom_path: Option<PathBuf>,
}

impl EmulatorSession {
    pub fn new(assets: AssetConfig) -> Result<Self> {
        let mut emulator = EmulatorBuilder::new().assets(assets).build();
        let _ = emulator.load_apu_ipl_rom();
        Ok(Self {
            emulator,
            rom_path: None,
        })
    }

    pub fn load_rom(&mut self, rom_path: PathBuf) -> Result<()> {
        let cartridge = Cartridge::load(&rom_path)
            .with_context(|| format!("failed to load ROM at {}", rom_path.display()))?;
        self.emulator.load_rom(cartridge);
        self.rom_path = Some(rom_path);
        Ok(())
    }

    pub fn run_frame(&mut self) -> Result<()> {
        self.emulator.run_until_frame().context("failed to run frame")
    }

    pub fn hold_controller1(&mut self, state: ControllerState) {
        self.emulator.set_controller1(state);
    }

    pub fn emulator(&self) -> &Emulator {
        &self.emulator
    }
    pub fn rom_path(&self) -> Option<&PathBuf> {
        self.rom_path.as_ref()
    }
}
