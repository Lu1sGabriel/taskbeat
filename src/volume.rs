//! Volume principal do dispositivo de saída padrão (IAudioEndpointVolume),
//! o mesmo que o botão de volume da bandeja controla.

use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

pub struct Volume {
    ep: Option<IAudioEndpointVolume>,
    last_endpoint: Instant,
    last_read: Instant,
    pub level: f32,
    pub muted: bool,
}

impl Volume {
    pub fn new() -> Self {
        let long_ago = Instant::now() - Duration::from_secs(60);
        let mut v = Self { ep: None, last_endpoint: long_ago, last_read: long_ago, level: 0.0, muted: false };
        v.poll();
        v
    }

    fn endpoint(&mut self) -> Option<IAudioEndpointVolume> {
        // Reabre a cada 3 s: se o dispositivo padrão mudar (fone, HDMI), segue junto.
        if self.ep.is_none() || self.last_endpoint.elapsed() > Duration::from_secs(3) {
            self.last_endpoint = Instant::now();
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
        if self.last_read.elapsed() < Duration::from_millis(150) {
            return;
        }
        self.last_read = Instant::now();
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
            self.last_read = Instant::now();
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
            self.last_read = Instant::now();
        }
    }
}
