//! Análise espectral: janela de Hann + FFT real, bandas em escala logarítmica,
//! ganho automático (AGC) e suavização com "gravidade" + picos.

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub const FFT_SIZE: usize = 4096;
const F_MIN: f32 = 40.0;
const F_MAX: f32 = 16_000.0;
/// Faixa dinâmica (dB) mapeada para 0..1.
const RANGE_DB: f32 = 48.0;

struct Band {
    lo: f32,
    hi: f32,
    center_hz: f32,
}

pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    window_gain: f32,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    mags: Vec<f32>,
    bands: Vec<Band>,
    targets: Vec<f32>,
    dbs: Vec<f32>,
    pub values: Vec<f32>,
    pub peaks: Vec<f32>,
    peak_hold: Vec<f32>,
    /// Nível de referência do AGC (dBFS do topo da escala).
    ref_db: f32,
    sample_rate: u32,
}

impl Analyzer {
    pub fn new(bands: usize, sample_rate: u32) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
            .collect();
        let window_gain = window.iter().sum::<f32>();
        let mut a = Self {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            mags: vec![0.0; FFT_SIZE / 2 + 1],
            fft,
            window,
            window_gain,
            bands: Vec::new(),
            targets: Vec::new(),
            dbs: Vec::new(),
            values: Vec::new(),
            peaks: Vec::new(),
            peak_hold: Vec::new(),
            ref_db: -20.0,
            sample_rate: 0,
        };
        a.configure(bands, sample_rate);
        a
    }

    pub fn configure(&mut self, count: usize, sample_rate: u32) {
        if count == self.values.len() && sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        let sr = sample_rate.max(8000) as f32;
        let fmax = F_MAX.min(sr * 0.45);
        let hz_to_bin = FFT_SIZE as f32 / sr;
        let ratio = fmax / F_MIN;
        self.bands = (0..count)
            .map(|i| {
                let f0 = F_MIN * ratio.powf(i as f32 / count as f32);
                let f1 = F_MIN * ratio.powf((i + 1) as f32 / count as f32);
                Band { lo: f0 * hz_to_bin, hi: f1 * hz_to_bin, center_hz: (f0 * f1).sqrt() }
            })
            .collect();
        self.targets = vec![0.0; count];
        self.dbs = vec![0.0; count];
        self.values = vec![0.0; count];
        self.peaks = vec![0.0; count];
        self.peak_hold = vec![0.0; count];
    }

    /// `samples` precisa ter FFT_SIZE amostras mono. `dt` em segundos.
    pub fn update(&mut self, samples: &[f32], dt: f32, sensitivity_db: f32) {
        debug_assert_eq!(samples.len(), FFT_SIZE);
        for ((dst, &s), &w) in self.input.iter_mut().zip(samples).zip(&self.window) {
            *dst = s * w;
        }
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            return;
        }
        let norm = 2.0 / self.window_gain;
        for (m, c) in self.mags.iter_mut().zip(&self.spectrum) {
            *m = c.norm() * norm;
        }

        // 1) Valor em dB por banda (+3 dB/oitava para compensar o espectro "rosa" da música).
        let mut frame_max_db = -200.0f32;
        let n = self.bands.len();
        let mut dbs = std::mem::take(&mut self.dbs);
        dbs.resize(n, 0.0);
        for (i, b) in self.bands.iter().enumerate() {
            let mag = if b.hi - b.lo < 1.0 {
                // Banda mais estreita que um bin: interpola no centro.
                let c = (b.lo + b.hi) * 0.5;
                let i0 = c.floor() as usize;
                let t = c - i0 as f32;
                let a = self.mags.get(i0).copied().unwrap_or(0.0);
                let bb = self.mags.get(i0 + 1).copied().unwrap_or(0.0);
                a + (bb - a) * t
            } else {
                let i0 = b.lo.floor() as usize;
                let i1 = (b.hi.ceil() as usize).min(self.mags.len() - 1);
                self.mags[i0..=i1].iter().copied().fold(0.0, f32::max)
            };
            let tilt = 3.0 * (b.center_hz / 1000.0).log2();
            let db = 20.0 * mag.max(1e-9).log10() + tilt;
            dbs[i] = db;
            frame_max_db = frame_max_db.max(db);
        }

        // 2) AGC: a referência sobe rápido e desce devagar; limitada para não
        //    amplificar ruído quando está tudo quieto.
        if frame_max_db > self.ref_db {
            self.ref_db += (frame_max_db - self.ref_db) * (dt * 12.0).min(1.0);
        } else {
            self.ref_db -= 4.0 * dt;
        }
        self.ref_db = self.ref_db.clamp(-38.0, 6.0);
        let top = self.ref_db + 2.0 - sensitivity_db;
        let floor = top - RANGE_DB;

        for (t, &db) in self.targets.iter_mut().zip(&dbs) {
            *t = ((db - floor) / RANGE_DB).clamp(0.0, 1.0).powf(1.35);
        }
        self.dbs = dbs;

        // 3) Suavização espacial leve + temporal (ataque rápido, queda com gravidade).
        let attack = (dt * 35.0).min(1.0);
        let fall = 1.9 * dt;
        for i in 0..n {
            let l = self.targets[i.saturating_sub(1)];
            let r = self.targets[(i + 1).min(n - 1)];
            let target = self.targets[i] * 0.6 + (l + r) * 0.2;
            let v = &mut self.values[i];
            if target > *v {
                *v += (target - *v) * attack;
            } else {
                *v = (*v - fall).max(target);
            }

            if *v >= self.peaks[i] {
                self.peaks[i] = *v;
                self.peak_hold[i] = 0.45;
            } else if self.peak_hold[i] > 0.0 {
                self.peak_hold[i] -= dt;
            } else {
                self.peaks[i] = (self.peaks[i] - dt * 0.8).max(*v);
            }
        }
    }

    pub fn energy(&self) -> f32 {
        self.values.iter().copied().fold(0.0, f32::max)
    }
}
