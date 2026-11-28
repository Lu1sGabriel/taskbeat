//! Volume principal do dispositivo de saída padrão (IAudioEndpointVolume),
//! o mesmo que o botão de volume da bandeja controla.

use std::sync::atomic::{AtomicU64, Ordering};
use crate::clock::elapsed_or_max;
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

/// Quantas vezes a saída padrão mudou (aviso do Windows, na thread do mixer).
static OUTPUT_VERSION: AtomicU64 = AtomicU64::new(0);

/// A saída padrão mudou: o volume do card passa a valer para a nova.
pub fn output_changed() {
    OUTPUT_VERSION.fetch_add(1, Ordering::Relaxed);
}

pub struct Volume {
    ep: Option<IAudioEndpointVolume>,
    /// `OUTPUT_VERSION` de quando `ep` foi aberto.
    version: u64,
    /// `None` = nunca (abre e lê já no primeiro uso).
    last_endpoint: Option<Instant>,
    last_read: Option<Instant>,
    pub level: f32,
    pub muted: bool,
}

impl Volume {
    pub fn new() -> Self {
        let mut v = Self { ep: None, version: 0, last_endpoint: None, last_read: None, level: 0.0, muted: false };
        v.poll();
        v
    }

    fn endpoint(&mut self) -> Option<IAudioEndpointVolume> {
        // Reabre quando o Windows avisa que a saída padrão mudou (fone, HDMI,
        // a escolha no mixer do card) e, por garantia, a cada 30 s.
        let version = OUTPUT_VERSION.load(Ordering::Relaxed);
        if self.ep.is_none() || self.version != version || elapsed_or_max(self.last_endpoint) > Duration::from_secs(30) {
            self.last_endpoint = Some(Instant::now());
            self.version = version;
            self.ep = unsafe {
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .and_then(|e| e.GetDefaultAudioEndpoint(eRender, eConsole))
                    .and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None))
                    .ok()
            };
        }
        self.ep.clone()
    }

    /// Lê nível e mudo (no máximo ~7x por segundo).
    pub fn poll(&mut self) {
        if elapsed_or_max(self.last_read) < Duration::from_millis(150) {
            return;
        }
        self.last_read = Some(Instant::now());
        if let Some(ep) = self.endpoint() {
            unsafe {
                if let Ok(l) = ep.GetMasterVolumeLevelScalar() {
                    self.level = l;
                }
                if let Ok(m) = ep.GetMute() {
                    self.muted = m.as_bool();
                }
            }
        }
    }

    pub fn set(&mut self, level: f32) {
        let level = level.clamp(0.0, 1.0);
        if let Some(ep) = self.endpoint() {
            unsafe {
                let _ = ep.SetMasterVolumeLevelScalar(level, std::ptr::null());
                if self.muted && level > 0.0 {
                    let _ = ep.SetMute(false, std::ptr::null());
                    self.muted = false;
                }
            }
            self.level = level;
            self.last_read = Some(Instant::now());
        }
    }

    pub fn step(&mut self, delta: f32) {
        let target = ((self.level + delta) * 50.0).round() / 50.0; // passos de 2%
        self.set(target);
    }

    pub fn toggle_mute(&mut self) {
        if let Some(ep) = self.endpoint() {
            unsafe {
                let _ = ep.SetMute(!self.muted, std::ptr::null());
            }
            self.muted = !self.muted;
            self.last_read = Some(Instant::now());
        }
    }
}
