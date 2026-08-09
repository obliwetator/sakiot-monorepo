//! Small dependency-free radix-2 FFT used by the prototype's convolution and
//! phase-vocoder paths. Keeping it here makes the native and WASM builds run
//! exactly the same transform code.

use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Complex {
    pub(crate) re: f32,
    pub(crate) im: f32,
}

impl Complex {
    pub(crate) fn from_polar(magnitude: f32, phase: f32) -> Self {
        Self {
            re: magnitude * phase.cos(),
            im: magnitude * phase.sin(),
        }
    }

    pub(crate) fn magnitude(self) -> f32 {
        self.re.hypot(self.im)
    }

    pub(crate) fn phase(self) -> f32 {
        self.im.atan2(self.re)
    }

    pub(crate) fn multiply(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }
}

pub(crate) fn transform(values: &mut [Complex], inverse: bool) {
    debug_assert!(values.len().is_power_of_two());
    let length = values.len();
    let mut target = 0;
    for source in 1..length {
        let mut bit = length >> 1;
        while target & bit != 0 {
            target ^= bit;
            bit >>= 1;
        }
        target ^= bit;
        if source < target {
            values.swap(source, target);
        }
    }

    let direction = if inverse { 1.0 } else { -1.0 };
    let mut width = 2;
    while width <= length {
        let angle = direction * TAU / width as f32;
        let root = Complex::from_polar(1.0, angle);
        for start in (0..length).step_by(width) {
            let mut twiddle = Complex { re: 1.0, im: 0.0 };
            for offset in 0..width / 2 {
                let even = values[start + offset];
                let odd = values[start + offset + width / 2].multiply(twiddle);
                values[start + offset] = Complex {
                    re: even.re + odd.re,
                    im: even.im + odd.im,
                };
                values[start + offset + width / 2] = Complex {
                    re: even.re - odd.re,
                    im: even.im - odd.im,
                };
                twiddle = twiddle.multiply(root);
            }
        }
        width *= 2;
    }

    if inverse {
        let scale = 1.0 / length as f32;
        for value in values {
            value.re *= scale;
            value.im *= scale;
        }
    }
}
