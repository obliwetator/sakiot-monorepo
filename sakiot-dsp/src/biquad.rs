use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Coefficients {
    pub(crate) b0: f32,
    pub(crate) b1: f32,
    pub(crate) b2: f32,
    pub(crate) a1: f32,
    pub(crate) a2: f32,
}

impl Coefficients {
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FilterState {
    z1: f32,
    z2: f32,
}

#[derive(Debug)]
pub(crate) struct Biquad {
    coefficients: Coefficients,
    states: Vec<FilterState>,
}

impl Biquad {
    pub(crate) fn new(channels: usize) -> Self {
        Self {
            coefficients: Coefficients::IDENTITY,
            states: vec![FilterState::default(); channels],
        }
    }

    pub(crate) fn set_coefficients(&mut self, coefficients: Coefficients) {
        self.coefficients = coefficients;
    }

    pub(crate) fn process(&mut self, channel: usize, input: f32) -> f32 {
        let state = &mut self.states[channel];
        let coefficients = self.coefficients;
        // Transposed direct form II: two state values per channel and no
        // dependence on the caller's block size.
        let output = coefficients.b0 * input + state.z1;
        state.z1 = coefficients.b1 * input - coefficients.a1 * output + state.z2;
        state.z2 = coefficients.b2 * input - coefficients.a2 * output;
        output
    }

    pub(crate) fn reset(&mut self) {
        self.states.fill(FilterState::default());
    }
}

pub(crate) fn peaking(sample_rate: f64, frequency: f64, q: f64, gain_db: f64) -> Coefficients {
    if gain_db == 0.0 {
        return Coefficients::IDENTITY;
    }
    let a = libm::pow(10.0, gain_db / 40.0);
    let omega = 2.0 * PI * frequency / sample_rate;
    let alpha = libm::sin(omega) / (2.0 * q);
    Coefficients::normalized(
        1.0 + alpha * a,
        -2.0 * libm::cos(omega),
        1.0 - alpha * a,
        1.0 + alpha / a,
        -2.0 * libm::cos(omega),
        1.0 - alpha / a,
    )
}

pub(crate) fn low_shelf(sample_rate: f64, frequency: f64, gain_db: f64) -> Coefficients {
    if gain_db == 0.0 {
        return Coefficients::IDENTITY;
    }
    let a = libm::pow(10.0, gain_db / 40.0);
    let omega = 2.0 * PI * frequency / sample_rate;
    let cosine = libm::cos(omega);
    let alpha = libm::sin(omega) * libm::sqrt(2.0) / 2.0;
    let beta = 2.0 * libm::sqrt(a) * alpha;
    Coefficients::normalized(
        a * ((a + 1.0) - (a - 1.0) * cosine + beta),
        2.0 * a * ((a - 1.0) - (a + 1.0) * cosine),
        a * ((a + 1.0) - (a - 1.0) * cosine - beta),
        (a + 1.0) + (a - 1.0) * cosine + beta,
        -2.0 * ((a - 1.0) + (a + 1.0) * cosine),
        (a + 1.0) + (a - 1.0) * cosine - beta,
    )
}

pub(crate) fn high_shelf(sample_rate: f64, frequency: f64, gain_db: f64) -> Coefficients {
    if gain_db == 0.0 {
        return Coefficients::IDENTITY;
    }
    let a = libm::pow(10.0, gain_db / 40.0);
    let omega = 2.0 * PI * frequency / sample_rate;
    let cosine = libm::cos(omega);
    let alpha = libm::sin(omega) * libm::sqrt(2.0) / 2.0;
    let beta = 2.0 * libm::sqrt(a) * alpha;
    Coefficients::normalized(
        a * ((a + 1.0) + (a - 1.0) * cosine + beta),
        -2.0 * a * ((a - 1.0) + (a + 1.0) * cosine),
        a * ((a + 1.0) + (a - 1.0) * cosine - beta),
        (a + 1.0) - (a - 1.0) * cosine + beta,
        2.0 * ((a - 1.0) - (a + 1.0) * cosine),
        (a + 1.0) - (a - 1.0) * cosine - beta,
    )
}
