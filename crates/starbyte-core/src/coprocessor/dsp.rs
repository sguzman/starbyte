use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::Address;
use crate::cartridge::{CartridgeHeader, Mapper};

const DSP_STATUS_READY: u8 = 0x80;
const DSP_STATUS_DATA_AVAILABLE: u8 = 0x40;
const DSP_STATUS_COMMAND_WAITING: u8 = 0x20;

/// Coarse DSP family revision derived from the cartridge metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DspVariant {
    /// DSP-1 baseline behavior.
    Dsp1,
    /// DSP-1B with the alternate data ROM observed in later boards.
    Dsp1B,
    /// DSP-2 family.
    Dsp2,
    /// DSP-3 family.
    Dsp3,
    /// DSP-4 family.
    Dsp4,
    /// We detected a DSP cartridge but could not refine the revision.
    Unknown,
}

impl DspVariant {
    /// Detect a likely DSP revision from the cartridge title and chipset byte.
    #[must_use]
    pub fn detect(header: &CartridgeHeader) -> Self {
        let title = header.title.to_ascii_uppercase();
        if title.contains("DSP-4") {
            return Self::Dsp4;
        }
        if title.contains("DSP-3") {
            return Self::Dsp3;
        }
        if title.contains("DSP-2") {
            return Self::Dsp2;
        }
        if title.contains("DSP-1B") || title.contains("DSP1B") || title.contains("1B") {
            return Self::Dsp1B;
        }
        if header.rom_type >= 0x03 {
            return Self::Dsp1;
        }
        Self::Unknown
    }
}

impl std::fmt::Display for DspVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dsp1 => f.write_str("DSP-1"),
            Self::Dsp1B => f.write_str("DSP-1B"),
            Self::Dsp2 => f.write_str("DSP-2"),
            Self::Dsp3 => f.write_str("DSP-3"),
            Self::Dsp4 => f.write_str("DSP-4"),
            Self::Unknown => f.write_str("unknown DSP"),
        }
    }
}

/// DSP-family runtime model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspCoprocessor {
    variant: DspVariant,
    map: DspAddressMap,
    shared: DspSharedState,
    status_register: u8,
    write_latch: u16,
    read_latch: u16,
    low_byte_latched: bool,
    read_high_pending: bool,
    active_command: Option<DspCommand>,
    frozen: bool,
    operand_words: Vec<i16>,
    result_words: VecDeque<i16>,
    pending_result_words: VecDeque<i16>,
    busy_cycles: u64,
    pending_freeze: bool,
}

impl DspCoprocessor {
    pub(crate) fn new(header: &CartridgeHeader) -> Self {
        let variant = DspVariant::detect(header);
        let map = DspAddressMap::for_header(header);
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            title = %header.title,
            mapper = ?header.mapper,
            variant = %variant,
            map = ?map,
            rom_type = header.rom_type,
            "initializing DSP coprocessor"
        );
        Self {
            variant,
            map,
            shared: DspSharedState::default(),
            status_register: DSP_STATUS_READY,
            write_latch: 0,
            read_latch: 0,
            low_byte_latched: false,
            read_high_pending: false,
            active_command: None,
            frozen: false,
            operand_words: Vec::new(),
            result_words: VecDeque::new(),
            pending_result_words: VecDeque::new(),
            busy_cycles: 0,
            pending_freeze: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            variant = %self.variant,
            "resetting DSP coprocessor"
        );
        self.status_register = DSP_STATUS_READY;
        self.shared = DspSharedState::default();
        self.write_latch = 0;
        self.read_latch = 0;
        self.low_byte_latched = false;
        self.read_high_pending = false;
        self.active_command = None;
        self.frozen = false;
        self.operand_words.clear();
        self.result_words.clear();
        self.pending_result_words.clear();
        self.busy_cycles = 0;
        self.pending_freeze = false;
    }

    pub(crate) fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let register = self.map.decode(mapper, address)?;
        if self.frozen && !matches!(register, DspRegister::Status) {
            return None;
        }
        let value = match register {
            DspRegister::DataLow => {
                self.refresh_read_latch();
                self.read_latch.to_le_bytes()[0]
            }
            DspRegister::DataHigh => {
                self.refresh_read_latch();
                self.read_latch.to_le_bytes()[1]
            }
            DspRegister::Status => self.status_register,
        };

        match register {
            DspRegister::DataLow => {
                self.read_high_pending = true;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "read DSP data low"
                );
            }
            DspRegister::DataHigh => {
                if self.read_high_pending {
                    let _ = self.result_words.pop_front();
                    self.read_high_pending = false;
                }
                self.refresh_status();
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "read DSP data high"
                );
            }
            DspRegister::Status => {
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    value = value,
                    "read DSP status"
                );
            }
        }

        Some(value)
    }

    pub(crate) fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(register) = self.map.decode(mapper, address) else {
            return false;
        };

        if self.frozen && !matches!(register, DspRegister::Status) {
            return true;
        }

        match register {
            DspRegister::DataLow => {
                self.write_latch = (self.write_latch & 0xFF00) | u16::from(value);
                self.low_byte_latched = true;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "latched DSP data low byte"
                );
            }
            DspRegister::DataHigh => {
                self.write_latch = (self.write_latch & 0x00FF) | (u16::from(value) << 8);
                if self.low_byte_latched {
                    self.accept_word(self.write_latch as i16);
                }
                self.low_byte_latched = false;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    word = self.write_latch,
                    "latched DSP data high byte"
                );
            }
            DspRegister::Status => {
                self.status_register = value;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    value = value,
                    "wrote DSP status"
                );
            }
        }

        true
    }

    pub(crate) fn step_master_cycles(&mut self, clocks: u64) {
        if self.busy_cycles == 0 {
            return;
        }

        self.busy_cycles = self.busy_cycles.saturating_sub(clocks);
        if self.busy_cycles == 0 {
            self.result_words = std::mem::take(&mut self.pending_result_words);
            if self.pending_freeze {
                self.frozen = true;
                self.pending_freeze = false;
            }
            self.refresh_status();
        }
    }

    fn accept_word(&mut self, word: i16) {
        if let Some(command) = self.active_command {
            self.operand_words.push(word);
            trace!(
                target: "starbyte_core::coprocessor::dsp",
                command = ?command,
                operand = word,
                collected = self.operand_words.len(),
                required = command.operand_count(),
                "accepted DSP operand"
            );
            if self.operand_words.len() >= command.operand_count() {
                let operands = std::mem::take(&mut self.operand_words);
                let results = command.execute(&operands, self.variant, &mut self.shared);
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    command = ?command,
                    variant = %self.variant,
                    operands = ?operands,
                    results = ?results,
                    "executed DSP command"
                );
                self.pending_result_words = results.into();
                self.busy_cycles = command.latency_cycles(self.variant);
                self.active_command = None;
                self.read_high_pending = false;
                self.pending_freeze = command.is_freeze();
                if self.busy_cycles == 0 {
                    self.result_words = std::mem::take(&mut self.pending_result_words);
                } else {
                    self.result_words.clear();
                }
                self.refresh_status();
            } else {
                self.refresh_status();
            }
            return;
        }

        let command = DspCommand::from_opcode(word as u16);
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            opcode = word as u16,
            command = ?command,
            variant = %self.variant,
            operand_count = command.operand_count(),
            "accepted DSP command opcode"
        );
        if command.operand_count() == 0 {
            let results = command.execute(&[], self.variant, &mut self.shared);
            trace!(
                target: "starbyte_core::coprocessor::dsp",
                opcode = word as u16,
                command = ?command,
                variant = %self.variant,
                results = ?results,
                "executed DSP command"
            );
            self.pending_result_words = results.into();
            self.busy_cycles = command.latency_cycles(self.variant);
            self.pending_freeze = command.is_freeze();
            if self.busy_cycles == 0 {
                self.result_words = std::mem::take(&mut self.pending_result_words);
                if self.pending_freeze {
                    self.frozen = true;
                    self.pending_freeze = false;
                }
            } else {
                self.result_words.clear();
            }
        } else {
            self.active_command = Some(command);
            self.operand_words.clear();
            self.result_words.clear();
            self.pending_result_words.clear();
        }
        self.read_high_pending = false;
        self.refresh_status();
    }

    fn refresh_read_latch(&mut self) {
        self.read_latch = self.result_words.front().copied().unwrap_or_default() as u16;
    }

    fn refresh_status(&mut self) {
        let previous = self.status_register;
        let mut status = if self.frozen { 0 } else { DSP_STATUS_READY };
        if !self.frozen {
            if !self.result_words.is_empty() {
                status |= DSP_STATUS_DATA_AVAILABLE;
            }
            if self.active_command.is_some() || self.busy_cycles > 0 {
                status |= DSP_STATUS_COMMAND_WAITING;
            }
            if self.busy_cycles > 0 {
                status &= !DSP_STATUS_READY;
            }
        }
        self.status_register = status;
        if previous != status {
            trace!(
                target: "starbyte_core::coprocessor::dsp",
                previous = previous,
                current = status,
                has_result = !self.result_words.is_empty(),
                waiting_for_operand = self.active_command.is_some(),
                "updated DSP status"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DspCommand {
    MemoryTest,
    MemoryDump,
    MemorySize,
    Multiply,
    Multiply2,
    Inverse,
    AttitudeA,
    AttitudeB,
    AttitudeC,
    ObjectiveA,
    ObjectiveB,
    ObjectiveC,
    SubjectiveA,
    SubjectiveB,
    SubjectiveC,
    ScalarA,
    ScalarB,
    ScalarC,
    Gyrate,
    Triangle,
    Radius,
    Range,
    Range2,
    Distance,
    Rotate,
    Polar,
    Freeze,
    Unknown(u16),
}

impl DspCommand {
    fn from_opcode(opcode: u16) -> Self {
        match opcode {
            0x000F => Self::MemoryTest,
            0x001F => Self::MemoryDump,
            0x002F => Self::MemorySize,
            0x0000 => Self::Multiply,
            0x0020 => Self::Multiply2,
            0x0010 | 0x0030 => Self::Inverse,
            0x0001 | 0x0005 | 0x0031 | 0x0035 => Self::AttitudeA,
            0x0011 | 0x0015 => Self::AttitudeB,
            0x0021 | 0x0025 => Self::AttitudeC,
            0x000D | 0x003D => Self::ObjectiveA,
            0x001D => Self::ObjectiveB,
            0x002D => Self::ObjectiveC,
            0x0003 | 0x0033 => Self::SubjectiveA,
            0x0013 => Self::SubjectiveB,
            0x0023 => Self::SubjectiveC,
            0x000B | 0x003B => Self::ScalarA,
            0x001B => Self::ScalarB,
            0x002B => Self::ScalarC,
            0x0014 | 0x0034 => Self::Gyrate,
            0x0004 | 0x0024 => Self::Triangle,
            0x0008 => Self::Radius,
            0x0018 => Self::Range,
            0x0038 => Self::Range2,
            0x0028 => Self::Distance,
            0x000C | 0x002C => Self::Rotate,
            0x001C | 0x003C => Self::Polar,
            0x001A | 0x002A | 0x003A => Self::Freeze,
            value => Self::Unknown(value),
        }
    }

    const fn operand_count(self) -> usize {
        match self {
            Self::Freeze | Self::Unknown(_) => 0,
            Self::MemoryTest => 1,
            Self::MemoryDump => 1,
            Self::MemorySize => 1,
            Self::AttitudeA | Self::AttitudeB | Self::AttitudeC => 4,
            Self::ObjectiveA
            | Self::ObjectiveB
            | Self::ObjectiveC
            | Self::SubjectiveA
            | Self::SubjectiveB
            | Self::SubjectiveC
            | Self::ScalarA
            | Self::ScalarB
            | Self::ScalarC => 3,
            Self::Gyrate => 6,
            Self::Multiply | Self::Multiply2 | Self::Inverse | Self::Triangle => 2,
            Self::Radius | Self::Distance | Self::Rotate => 3,
            Self::Range => 4,
            Self::Polar => 6,
            Self::Range2 => 4,
        }
    }

    const fn is_freeze(self) -> bool {
        matches!(self, Self::Freeze)
    }

    const fn latency_cycles(self, variant: DspVariant) -> u64 {
        match (self, variant) {
            (Self::MemoryDump, _) => 48,
            (Self::AttitudeA | Self::AttitudeB | Self::AttitudeC, _) => 18,
            (
                Self::ObjectiveA
                    | Self::ObjectiveB
                    | Self::ObjectiveC
                    | Self::SubjectiveA
                    | Self::SubjectiveB
                    | Self::SubjectiveC
                    | Self::ScalarA
                    | Self::ScalarB
                    | Self::ScalarC
                    | Self::Gyrate,
                _,
            ) => 18,
            (Self::Polar, _) => 24,
            (Self::Rotate | Self::Distance | Self::Inverse, _) => 18,
            (Self::Radius | Self::Range | Self::Range2 | Self::Triangle, _) => 12,
            (Self::Multiply | Self::Multiply2, _) => 6,
            (Self::Freeze, _) => 2,
            (Self::MemoryTest | Self::MemorySize, _) => 4,
            (Self::Unknown(_), _) => 0,
        }
    }

    fn execute(self, operands: &[i16], variant: DspVariant, shared: &mut DspSharedState) -> Vec<i16> {
        if !self.supported_by(variant) {
            return vec![unsupported_variant_code(variant, self.opcode_hint())];
        }

        match self {
            Self::MemoryTest => vec![0],
            Self::MemoryDump => build_memory_dump(variant, operands.first().copied().unwrap_or_default()),
            Self::MemorySize => vec![0x0100],
            Self::Multiply => vec![q15_mul(operands[0], operands[1])],
            Self::Multiply2 => vec![q15_mul(operands[0], operands[1]).saturating_add(1)],
            Self::Inverse => {
                let (coefficient, exponent) = dsp_inverse(operands[0], operands[1]);
                vec![coefficient, exponent]
            }
            Self::AttitudeA => {
                shared.matrix_a = build_attitude_matrix(operands[0], operands[1], operands[2], operands[3]);
                Vec::new()
            }
            Self::AttitudeB => {
                shared.matrix_b = build_attitude_matrix(operands[0], operands[1], operands[2], operands[3]);
                Vec::new()
            }
            Self::AttitudeC => {
                shared.matrix_c = build_attitude_matrix(operands[0], operands[1], operands[2], operands[3]);
                Vec::new()
            }
            Self::ObjectiveA => transform_objective(&shared.matrix_a, operands),
            Self::ObjectiveB => transform_objective(&shared.matrix_b, operands),
            Self::ObjectiveC => transform_objective(&shared.matrix_c, operands),
            Self::SubjectiveA => transform_subjective(&shared.matrix_a, operands),
            Self::SubjectiveB => transform_subjective(&shared.matrix_b, operands),
            Self::SubjectiveC => transform_subjective(&shared.matrix_c, operands),
            Self::ScalarA => vec![scalar_forward(&shared.matrix_a, operands)],
            Self::ScalarB => vec![scalar_forward(&shared.matrix_b, operands)],
            Self::ScalarC => vec![scalar_forward(&shared.matrix_c, operands)],
            Self::Gyrate => gyrate(operands),
            Self::Triangle => {
                let sin = dsp_sin(operands[0]);
                let cos = dsp_cos(operands[0]);
                vec![q15_mul(sin, operands[1]), q15_mul(cos, operands[1])]
            }
            Self::Radius => {
                let radius = vector_radius(operands[0], operands[1], operands[2]).saturating_mul(2);
                let bytes = radius.to_le_bytes();
                vec![
                    i16::from_le_bytes([bytes[0], bytes[1]]),
                    i16::from_le_bytes([bytes[2], bytes[3]]),
                ]
            }
            Self::Range => vec![vector_range(operands[0], operands[1], operands[2], operands[3], 0)],
            Self::Range2 => vec![vector_range(operands[0], operands[1], operands[2], operands[3], 1)],
            Self::Distance => vec![vector_distance(operands[0], operands[1], operands[2])],
            Self::Rotate => {
                let sin = dsp_sin(operands[0]);
                let cos = dsp_cos(operands[0]);
                let x = q15_mul(operands[2], sin).saturating_add(q15_mul(operands[1], cos));
                let y = q15_mul(operands[2], cos).saturating_sub(q15_mul(operands[1], sin));
                vec![x, y]
            }
            Self::Polar => {
                let (az, ay, ax, mut x, mut y, mut z) = (
                    operands[0], operands[1], operands[2], operands[3], operands[4], operands[5],
                );
                let sin_az = dsp_sin(az);
                let cos_az = dsp_cos(az);
                let rot_x = q15_mul(y, sin_az).saturating_add(q15_mul(x, cos_az));
                let rot_y = q15_mul(y, cos_az).saturating_sub(q15_mul(x, sin_az));
                x = rot_x;
                y = rot_y;

                let sin_ay = dsp_sin(ay);
                let cos_ay = dsp_cos(ay);
                let rot_z = q15_mul(x, sin_ay).saturating_add(q15_mul(z, cos_ay));
                let rot_x = q15_mul(x, cos_ay).saturating_sub(q15_mul(z, sin_ay));
                x = rot_x;
                z = rot_z;

                let sin_ax = dsp_sin(ax);
                let cos_ax = dsp_cos(ax);
                let rot_y = q15_mul(z, sin_ax).saturating_add(q15_mul(y, cos_ax));
                let rot_z = q15_mul(z, cos_ax).saturating_sub(q15_mul(y, sin_ax));
                vec![x, rot_y, rot_z]
            }
            Self::Freeze => Vec::new(),
            Self::Unknown(opcode) => vec![opcode as i16],
        }
    }

    const fn supported_by(self, variant: DspVariant) -> bool {
        match variant {
            DspVariant::Dsp1 | DspVariant::Dsp1B | DspVariant::Unknown => true,
            DspVariant::Dsp2 => matches!(
                self,
                Self::MemoryTest
                    | Self::MemoryDump
                    | Self::MemorySize
                    | Self::Multiply2
                    | Self::AttitudeC
                    | Self::ObjectiveC
                    | Self::SubjectiveC
                    | Self::ScalarC
                    | Self::Triangle
                    | Self::Distance
                    | Self::Rotate
                    | Self::Polar
                    | Self::Freeze
            ),
            DspVariant::Dsp3 => matches!(self, Self::MemoryTest | Self::MemoryDump | Self::Freeze),
            DspVariant::Dsp4 => matches!(
                self,
                Self::MemoryTest | Self::MemoryDump | Self::Freeze | Self::Distance | Self::Rotate
            ),
        }
    }

    const fn opcode_hint(self) -> u16 {
        match self {
            Self::MemoryTest => 0x000F,
            Self::MemoryDump => 0x001F,
            Self::MemorySize => 0x002F,
            Self::Multiply => 0x0000,
            Self::Multiply2 => 0x0020,
            Self::Inverse => 0x0010,
            Self::AttitudeA => 0x0001,
            Self::AttitudeB => 0x0011,
            Self::AttitudeC => 0x0021,
            Self::ObjectiveA => 0x000D,
            Self::ObjectiveB => 0x001D,
            Self::ObjectiveC => 0x002D,
            Self::SubjectiveA => 0x0003,
            Self::SubjectiveB => 0x0013,
            Self::SubjectiveC => 0x0023,
            Self::ScalarA => 0x000B,
            Self::ScalarB => 0x001B,
            Self::ScalarC => 0x002B,
            Self::Gyrate => 0x0014,
            Self::Triangle => 0x0004,
            Self::Radius => 0x0008,
            Self::Range => 0x0018,
            Self::Range2 => 0x0038,
            Self::Distance => 0x0028,
            Self::Rotate => 0x000C,
            Self::Polar => 0x001C,
            Self::Freeze => 0x001A,
            Self::Unknown(value) => value,
        }
    }
}

fn unsupported_variant_code(variant: DspVariant, opcode: u16) -> i16 {
    let tag = match variant {
        DspVariant::Dsp1 => 0x1000_u16,
        DspVariant::Dsp1B => 0x1B00,
        DspVariant::Dsp2 => 0x2D00,
        DspVariant::Dsp3 => 0x3D00,
        DspVariant::Dsp4 => 0x4D00,
        DspVariant::Unknown => 0x7F00,
    };
    tag.wrapping_add(opcode as u8 as u16) as i16
}

fn q15_mul(left: i16, right: i16) -> i16 {
    ((i32::from(left) * i32::from(right)) >> 15) as i16
}

fn vector_radius(x: i16, y: i16, z: i16) -> i32 {
    i32::from(x) * i32::from(x) + i32::from(y) * i32::from(y) + i32::from(z) * i32::from(z)
}

fn vector_range(x: i16, y: i16, z: i16, radius: i16, bias: i16) -> i16 {
    let lhs = vector_radius(x, y, z) - (i32::from(radius) * i32::from(radius));
    ((lhs >> 15) as i16).saturating_add(bias)
}

fn vector_distance(x: i16, y: i16, z: i16) -> i16 {
    let radius = vector_radius(x, y, z) as f64;
    radius.sqrt().round().clamp(f64::from(i16::MIN + 1), f64::from(i16::MAX)) as i16
}

fn dsp_sin(angle: i16) -> i16 {
    let radians = f64::from(angle) * std::f64::consts::PI / 32768.0;
    (radians.sin() * 32767.0).round().clamp(-32767.0, 32767.0) as i16
}

fn dsp_cos(angle: i16) -> i16 {
    let radians = f64::from(angle) * std::f64::consts::PI / 32768.0;
    (radians.cos() * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

fn dsp_inverse(coefficient: i16, exponent: i16) -> (i16, i16) {
    if coefficient == 0 {
        return (0x7fff, 0x002f);
    }

    let mut value = f64::from(coefficient) / 32768.0;
    value *= 2.0_f64.powi(i32::from(exponent));
    if value == 0.0 {
        return (0x7fff, 0x002f);
    }
    let mut inverse = 1.0 / value;
    let mut out_exponent = 0_i16;

    while inverse.abs() >= 1.0 {
        inverse /= 2.0;
        out_exponent = out_exponent.saturating_add(1);
    }
    while inverse.abs() < 0.5 {
        inverse *= 2.0;
        out_exponent = out_exponent.saturating_sub(1);
    }

    let out_coefficient = (inverse * 32768.0)
        .round()
        .clamp(f64::from(i16::MIN + 1), f64::from(i16::MAX)) as i16;
    (out_coefficient, out_exponent)
}

fn build_memory_dump(variant: DspVariant, seed_word: i16) -> Vec<i16> {
    let variant_seed = match variant {
        DspVariant::Dsp1 => 0x1357_u16,
        DspVariant::Dsp1B => 0x1B1B,
        DspVariant::Dsp2 => 0x2D2D,
        DspVariant::Dsp3 => 0x3D3D,
        DspVariant::Dsp4 => 0x4D4D,
        DspVariant::Unknown => 0x7A7A,
    };
    let mut state = variant_seed ^ (seed_word as u16);
    (0..1024)
        .map(|index| {
            state = state.rotate_left(3).wrapping_add(0x41C6).wrapping_add(index as u16);
            state ^= (index as u16).wrapping_mul(0x0101);
            i16::from_le_bytes(state.to_le_bytes())
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DspSharedState {
    matrix_a: [[i16; 3]; 3],
    matrix_b: [[i16; 3]; 3],
    matrix_c: [[i16; 3]; 3],
}

fn build_attitude_matrix(scale: i16, rz: i16, ry: i16, rx: i16) -> [[i16; 3]; 3] {
    let sin_rz = dsp_sin(rz);
    let cos_rz = dsp_cos(rz);
    let sin_ry = dsp_sin(ry);
    let cos_ry = dsp_cos(ry);
    let sin_rx = dsp_sin(rx);
    let cos_rx = dsp_cos(rx);
    let s = scale;

    [
        [
            q15_mul(q15_mul(s, cos_rz), cos_ry),
            q15_mul(q15_mul(s, sin_rz), cos_rx)
                .saturating_add(q15_mul(q15_mul(q15_mul(s, cos_rz), sin_rx), sin_ry)),
            q15_mul(q15_mul(s, sin_rz), sin_rx)
                .saturating_sub(q15_mul(q15_mul(q15_mul(s, cos_rz), cos_rx), sin_ry)),
        ],
        [
            -q15_mul(q15_mul(s, sin_rz), cos_ry),
            q15_mul(q15_mul(s, cos_rz), cos_rx)
                .saturating_sub(q15_mul(q15_mul(q15_mul(s, sin_rz), sin_rx), sin_ry)),
            q15_mul(q15_mul(s, cos_rz), sin_rx)
                .saturating_add(q15_mul(q15_mul(q15_mul(s, sin_rz), cos_rx), sin_ry)),
        ],
        [
            q15_mul(s, sin_ry),
            -q15_mul(q15_mul(s, sin_rx), cos_ry),
            q15_mul(q15_mul(s, cos_rx), cos_ry),
        ],
    ]
}

fn transform_objective(matrix: &[[i16; 3]; 3], operands: &[i16]) -> Vec<i16> {
    let x = operands[0];
    let y = operands[1];
    let z = operands[2];
    vec![
        dot3([matrix[0][0], matrix[1][0], matrix[2][0]], [x, y, z]),
        dot3([matrix[0][1], matrix[1][1], matrix[2][1]], [x, y, z]),
        dot3([matrix[0][2], matrix[1][2], matrix[2][2]], [x, y, z]),
    ]
}

fn transform_subjective(matrix: &[[i16; 3]; 3], operands: &[i16]) -> Vec<i16> {
    let f = operands[0];
    let l = operands[1];
    let u = operands[2];
    vec![
        dot3(matrix[0], [f, l, u]),
        dot3(matrix[1], [f, l, u]),
        dot3(matrix[2], [f, l, u]),
    ]
}

fn scalar_forward(matrix: &[[i16; 3]; 3], operands: &[i16]) -> i16 {
    dot3([matrix[0][0], matrix[1][0], matrix[2][0]], [operands[0], operands[1], operands[2]])
}

fn dot3(left: [i16; 3], right: [i16; 3]) -> i16 {
    let sum = q15_mul(left[0], right[0]) as i32
        + q15_mul(left[1], right[1]) as i32
        + q15_mul(left[2], right[2]) as i32;
    sum.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn gyrate(operands: &[i16]) -> Vec<i16> {
    let az = operands[0];
    let ax = operands[1];
    let ay = operands[2];
    let u = operands[3];
    let f = operands[4];
    let l = operands[5];

    let sin_ay = dsp_sin(ay);
    let cos_ay = dsp_cos(ay);
    let cos_ax = dsp_cos(ax);
    let sin_ax = dsp_sin(ax);
    let (sec_coefficient, sec_exponent) = dsp_inverse(cos_ax, 0);
    let sec_ax = denormalize_fp(sec_coefficient, sec_exponent);

    let rz = az.saturating_add(q15_mul(sec_ax, q15_mul(u, cos_ay).saturating_sub(q15_mul(f, sin_ay))));
    let rx = ax.saturating_add(q15_mul(u, sin_ay).saturating_add(q15_mul(f, cos_ay)));
    let ry = ay
        .saturating_add(l)
        .saturating_sub(q15_mul(q15_mul(sec_ax, sin_ax), q15_mul(u, cos_ay).saturating_add(q15_mul(f, sin_ay))));

    vec![rz, rx, ry]
}

fn denormalize_fp(coefficient: i16, exponent: i16) -> i16 {
    let value = (f64::from(coefficient) / 32768.0) * 2.0_f64.powi(i32::from(exponent));
    (value * 32768.0)
        .round()
        .clamp(f64::from(i16::MIN + 1), f64::from(i16::MAX)) as i16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DspAddressMap {
    LoRom1MiB,
    LoRom2MiB,
    HiRom,
}

impl DspAddressMap {
    fn for_header(header: &CartridgeHeader) -> Self {
        match header.mapper {
            Mapper::HiRom => Self::HiRom,
            Mapper::LoRom
                if header.rom_size_bytes() > (1024 * 1024) || header.ram_size_bytes() > 0 =>
            {
                Self::LoRom2MiB
            }
            Mapper::LoRom => Self::LoRom1MiB,
        }
    }

    fn decode(self, mapper: Mapper, address: Address) -> Option<DspRegister> {
        if mapper == Mapper::HiRom {
            return self.decode_hirom(address);
        }

        match self {
            Self::LoRom1MiB => self.decode_lorom_1mib(address),
            Self::LoRom2MiB => self.decode_lorom_2mib(address),
            Self::HiRom => self.decode_hirom(address),
        }
    }

    fn decode_lorom_1mib(self, address: Address) -> Option<DspRegister> {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;
        let bank = bank & 0x7F;
        if !(0x30..=0x3F).contains(&bank) {
            return None;
        }

        match offset {
            0x8000..=0xBFFF => Some(byte_register(offset)),
            0xC000..=0xFFFF => Some(DspRegister::Status),
            _ => None,
        }
    }

    fn decode_lorom_2mib(self, address: Address) -> Option<DspRegister> {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;
        let bank = bank & 0x7F;
        if !(0x60..=0x6F).contains(&bank) {
            return None;
        }

        match offset {
            0x0000..=0x3FFF => Some(byte_register(offset)),
            0x4000..=0x7FFF => Some(DspRegister::Status),
            _ => None,
        }
    }

    fn decode_hirom(self, address: Address) -> Option<DspRegister> {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;
        let bank = bank & 0x7F;
        if !((0x00..=0x1F).contains(&bank) || (0x20..=0x2F).contains(&bank)) {
            return None;
        }

        match offset {
            0x6000..=0x6FFF => Some(byte_register(offset)),
            0x7000..=0x7FFF => Some(DspRegister::Status),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DspRegister {
    DataLow,
    DataHigh,
    Status,
}

impl std::fmt::Display for DspRegister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DataLow => f.write_str("data-low"),
            Self::DataHigh => f.write_str("data-high"),
            Self::Status => f.write_str("status"),
        }
    }
}

const fn byte_register(offset: u16) -> DspRegister {
    if offset & 0x0001 == 0 {
        DspRegister::DataLow
    } else {
        DspRegister::DataHigh
    }
}

#[cfg(test)]
mod tests {
    use crate::cartridge::{CartridgeHeader, Mapper, Region};

    use super::*;

    fn header(
        title: &str,
        mapper: Mapper,
        rom_type: u8,
        rom_size_code: u8,
        ram_size_code: u8,
    ) -> CartridgeHeader {
        CartridgeHeader {
            title: title.to_owned(),
            mapper,
            map_mode: match mapper {
                Mapper::LoRom => 0x20,
                Mapper::HiRom => 0x21,
            },
            rom_type,
            rom_size_code,
            ram_size_code,
            destination_code: 0x01,
            region: Region::Ntsc,
            complement: 0xFFFF,
            checksum: 0x0000,
        }
    }

    fn write_word(dsp: &mut DspCoprocessor, mapper: Mapper, address: u32, word: u16) {
        assert!(dsp.write(mapper, address, (word & 0x00FF) as u8));
        assert!(dsp.write(mapper, address + 1, (word >> 8) as u8));
    }

    fn read_word(dsp: &mut DspCoprocessor, mapper: Mapper, address: u32) -> u16 {
        let lo = dsp.read(mapper, address).unwrap();
        let hi = dsp.read(mapper, address + 1).unwrap();
        u16::from_le_bytes([lo, hi])
    }

    #[test]
    fn variant_detection_prefers_specific_titles() {
        assert_eq!(
            DspVariant::detect(&header("STARBYTE DSP-1B TEST", Mapper::LoRom, 0x03, 0x08, 0x00)),
            DspVariant::Dsp1B
        );
        assert_eq!(
            DspVariant::detect(&header("STARBYTE DSP-4 TEST", Mapper::LoRom, 0x03, 0x08, 0x00)),
            DspVariant::Dsp4
        );
    }

    #[test]
    fn multiply_command_obeys_q15_scaling_and_latency() {
        let mut dsp = DspCoprocessor::new(&header("STARBYTE DSP-1", Mapper::LoRom, 0x03, 0x08, 0x00));

        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x0000);
        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x4000);
        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x4000);

        assert_eq!(dsp.read(Mapper::LoRom, 0x30C000), Some(DSP_STATUS_COMMAND_WAITING));
        dsp.step_master_cycles(5);
        assert_eq!(dsp.read(Mapper::LoRom, 0x30C000), Some(DSP_STATUS_COMMAND_WAITING));
        dsp.step_master_cycles(1);
        assert_eq!(
            dsp.read(Mapper::LoRom, 0x30C000),
            Some(DSP_STATUS_READY | DSP_STATUS_DATA_AVAILABLE)
        );
        assert_eq!(read_word(&mut dsp, Mapper::LoRom, 0x308000), 0x2000);
    }

    #[test]
    fn triangle_and_rotate_commands_use_geometric_semantics() {
        let mut dsp = DspCoprocessor::new(&header("STARBYTE DSP-1", Mapper::LoRom, 0x03, 0x08, 0x00));

        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x0004);
        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x4000);
        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x4000);
        dsp.step_master_cycles(12);
        let y = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let x = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        assert!(y > 0x3E00_i16);
        assert!(x.unsigned_abs() < 0x0200);

        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x000C);
        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x4000);
        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x4000);
        write_word(&mut dsp, Mapper::LoRom, 0x308000, 0x0000);
        dsp.step_master_cycles(18);
        let x2 = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let y2 = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        assert!(x2.unsigned_abs() < 0x0200);
        assert!(y2 < -0x3E00_i16);
    }

    #[test]
    fn memory_dump_is_large_and_variant_specific() {
        let mut dsp1 = DspCoprocessor::new(&header("STARBYTE DSP-1", Mapper::LoRom, 0x03, 0x08, 0x00));
        let mut dsp1b = DspCoprocessor::new(&header("STARBYTE DSP-1B", Mapper::LoRom, 0x03, 0x08, 0x00));

        write_word(&mut dsp1, Mapper::LoRom, 0x308000, 0x001F);
        write_word(&mut dsp1, Mapper::LoRom, 0x308000, 0x1234);
        write_word(&mut dsp1b, Mapper::LoRom, 0x308000, 0x001F);
        write_word(&mut dsp1b, Mapper::LoRom, 0x308000, 0x1234);
        dsp1.step_master_cycles(48);
        dsp1b.step_master_cycles(48);

        assert_eq!(dsp1.result_words.len(), 1024);
        assert_eq!(dsp1b.result_words.len(), 1024);
        assert_ne!(dsp1.result_words.front(), dsp1b.result_words.front());
    }

    #[test]
    fn unsupported_variant_commands_return_variant_marked_error_word() {
        let mut dsp2 = DspCoprocessor::new(&header("STARBYTE DSP-2", Mapper::LoRom, 0x03, 0x08, 0x00));
        write_word(&mut dsp2, Mapper::LoRom, 0x308000, 0x0010);
        write_word(&mut dsp2, Mapper::LoRom, 0x308000, 0x4000);
        write_word(&mut dsp2, Mapper::LoRom, 0x308000, 0x0000);
        dsp2.step_master_cycles(18);
        assert_eq!(read_word(&mut dsp2, Mapper::LoRom, 0x308000), 0x2D10);
    }

    #[test]
    fn attitude_and_transform_commands_share_matrix_state() {
        let mut dsp = DspCoprocessor::new(&header("STARBYTE DSP-1", Mapper::LoRom, 0x03, 0x08, 0x00));

        for word in [0x0001_u16, 0x7FFF, 0x0000, 0x0000, 0x0000] {
            write_word(&mut dsp, Mapper::LoRom, 0x308000, word);
        }
        dsp.step_master_cycles(18);

        for word in [0x000D_u16, 0x4000, 0x0000, 0x0000] {
            write_word(&mut dsp, Mapper::LoRom, 0x308000, word);
        }
        dsp.step_master_cycles(18);
        let f = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let l = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let u = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        assert!(f > 0x3E00_i16);
        assert!(l.unsigned_abs() < 0x0100);
        assert!(u.unsigned_abs() < 0x0100);

        for word in [0x0003_u16, f as u16, l as u16, u as u16] {
            write_word(&mut dsp, Mapper::LoRom, 0x308000, word);
        }
        dsp.step_master_cycles(18);
        let x = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let y = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let z = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        assert!(x > 0x3E00_i16);
        assert!(y.unsigned_abs() < 0x0100);
        assert!(z.unsigned_abs() < 0x0100);
    }

    #[test]
    fn scalar_and_gyrate_commands_produce_nontrivial_results() {
        let mut dsp = DspCoprocessor::new(&header("STARBYTE DSP-1", Mapper::LoRom, 0x03, 0x08, 0x00));

        for word in [0x0001_u16, 0x7FFF, 0x2000, 0x0000, 0x0000] {
            write_word(&mut dsp, Mapper::LoRom, 0x308000, word);
        }
        dsp.step_master_cycles(18);

        for word in [0x000B_u16, 0x4000, 0x0000, 0x0000] {
            write_word(&mut dsp, Mapper::LoRom, 0x308000, word);
        }
        dsp.step_master_cycles(18);
        let scalar = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        assert!(scalar > 0x2000_i16);

        for word in [0x0014_u16, 0x1000, 0x0800, 0x0400, 0x0100, 0x0080, 0x0040] {
            write_word(&mut dsp, Mapper::LoRom, 0x308000, word);
        }
        dsp.step_master_cycles(18);
        let rz = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let rx = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        let ry = read_word(&mut dsp, Mapper::LoRom, 0x308000) as i16;
        assert_ne!((rz, rx, ry), (0x1000, 0x0800, 0x0400));
    }
}
