//! Volume da saída padrão (o mesmo do Controle Central): volume "virtual
//! principal", que vale mesmo em dispositivos sem controle de volume geral.

use super::audio::{default_output, get};
use objc2_core_audio::{AudioObjectID, AudioObjectPropertyAddress, AudioObjectSetPropertyData, kAudioDevicePropertyMute, kAudioObjectPropertyElementMain};
use std::ptr::NonNull;

/// kAudioHardwareServiceDeviceProperty_VirtualMainVolume ('vmvc').
const VIRTUAL_MAIN_VOLUME: u32 = 0x766d_7663;
/// kAudioDevicePropertyScopeOutput ('outp').
const SCOPE_OUTPUT: u32 = 0x6f75_7470;

fn addr(sel: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: sel, mScope: SCOPE_OUTPUT, mElement: kAudioObjectPropertyElementMain }
}

fn set<T: Copy>(dev: AudioObjectID, sel: u32, v: T) -> bool {
    let a = addr(sel);
    unsafe { AudioObjectSetPropertyData(dev, NonNull::from(&a), 0, std::ptr::null(), std::mem::size_of::<T>() as u32, NonNull::from(&v).cast()) == 0 }
}

#[derive(Default)]
pub struct Volume {
    pub level: f32,
    pub muted: bool,
}

impl Volume {
    /// Lê o volume e o mudo atuais (chamado com o card aberto).
    pub fn poll(&mut self) {
        let Some(dev) = default_output() else { return };
        if let Some(v) = get(dev, addr(VIRTUAL_MAIN_VOLUME), 0f32) {
            self.level = v.clamp(0.0, 1.0);
        }
        if let Some(m) = get(dev, addr(kAudioDevicePropertyMute), 0u32) {
            self.muted = m != 0;
        }
    }

    pub fn set_level(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if let Some(dev) = default_output() {
            if set(dev, VIRTUAL_MAIN_VOLUME, v) {
                self.level = v;
            }
            // Mexer no volume tira o mudo, como no Controle Central.
            if self.muted && v > 0.0 && set(dev, kAudioDevicePropertyMute, 0u32) {
                self.muted = false;
            }
        }
    }

    pub fn toggle_mute(&mut self) {
        if let Some(dev) = default_output() {
            if set(dev, kAudioDevicePropertyMute, (!self.muted) as u32) {
                self.muted = !self.muted;
            }
        }
    }
}
