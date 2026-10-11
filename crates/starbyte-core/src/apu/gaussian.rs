//! Four-tap S-DSP Gaussian interpolation using a generated 512-entry table.
//!
//! The coefficient formula is reconstructed from the documented S-DSP table,
//! using the permissively licensed ares/RustySNES mathematical derivation.
//! Generating the table once avoids embedding code or data from SNES9x.
//! The surrounding voice pipeline is still sample-stepped, not cycle-exact.

use std::sync::OnceLock;

/// The S-DSP's lookup coefficients, generated once from its Gaussian curve.
fn coefficients() -> &'static [i16; 512] {
    static COEFFICIENTS: OnceLock<[i16; 512]> = OnceLock::new();
    COEFFICIENTS.get_or_init(|| {
        let mut curve = [0.0_f64; 512];
        let pi = std::f64::consts::PI;
        for (index, slot) in curve.iter_mut().enumerate() {
            let k = index as f64 + 0.5;
            let sin = (pi * k * 1.280 / 1024.0).sin();
            let first = ((pi * k * 2.0 / 1023.0).cos() - 1.0) * 0.5;
            let second = ((pi * k * 4.0 / 1023.0).cos() - 1.0) * 0.08;
            *slot = sin * (1.0 + first + second) / k;
        }
        let mut result = [0_i16; 512];
        for phase in 0..128 {
            let indices = [phase, phase + 256, 511 - phase, 255 - phase];
            let sum: f64 = indices.iter().map(|&i| curve[511 - i]).sum();
            let factor = 2048.0 / sum;
            for i in indices {
                result[i] = (curve[511 - i] * factor + 0.5) as i16;
            }
        }
        result
    })
}

/// Interpolate four consecutive decoded BRR samples at a 12-bit pitch phase.
///
/// The first three independently shifted terms wrap as a signed 16-bit
/// intermediate; the fourth is added before a final clamp and even-bit mask.
/// The raw sample window is supplied by the voice's decoded history.
pub(super) fn interpolate(samples: [i16; 4], fraction: u32) -> i16 {
    let table = coefficients();
    let offset = (fraction >> 4 & 0xff) as usize;
    let weights = [
        table[255 - offset],
        table[511 - offset],
        table[256 + offset],
        table[offset],
    ];
    let mut value = 0_i32;
    for i in 0..3 {
        value += (i32::from(samples[i]) * i32::from(weights[i])) >> 11;
    }
    value = i32::from(value as i16);
    value += (i32::from(samples[3]) * i32::from(weights[3])) >> 11;
    (value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16) & !1
}

#[cfg(test)]
mod tests {
    use super::{coefficients, interpolate};

    #[test]
    fn generated_coefficients_match_known_spc_gaussian_landmarks() {
        let table = coefficients();
        assert_eq!(
            (table[0], table[255], table[256], table[511]),
            (0, 370, 374, 1305)
        );
        assert!(table.windows(2).all(|pair| pair[0] <= pair[1]));
        for phase in 0..256 {
            let sum = i32::from(table[255 - phase])
                + i32::from(table[511 - phase])
                + i32::from(table[256 + phase])
                + i32::from(table[phase]);
            assert!((2045..=2051).contains(&sum));
        }
    }

    #[test]
    fn fractional_phase_uses_all_four_taps_and_is_repeatable() {
        assert_eq!(interpolate([0; 4], 0), 0);
        let low = interpolate([0, 0, 12_000, 0], 0);
        let high = interpolate([0, 0, 12_000, 0], 0xff0);
        assert!(low > 1000);
        assert!(high > low * 2);
        assert_eq!(interpolate([0, 0, 12_000, 0], 0xff0), high);
        assert_eq!(interpolate([0, 0, 0, 12_000], 0), 0);
        assert!(interpolate([0, 0, 0, 12_000], 0xff0) > 1000);
    }

    #[test]
    fn interpolation_preserves_signed_wrap_and_clamp_order() {
        // The first three weighted terms are wrapped to signed 16 bits
        // before tap four is added. Full-scale negative BRR can therefore
        // become positive at the exact Gaussian overshoot boundary.
        assert_eq!(interpolate([i16::MIN; 4], 0), 32_752);
        assert!(interpolate([-24_576; 4], 0) < 0);
        let loud = interpolate([30_000; 4], 0);
        assert_eq!(loud & 1, 0);
        assert!(loud <= i16::MAX);
        assert!(interpolate([-30_000; 4], 0) < 0);
    }
}
