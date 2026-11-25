//! Saídas de áudio do Mac, para o "mixer" do card.
//!
//! O macOS não tem volume por app: o único jeito seria capturar cada app com
//! um *process tap* mudo (14.2+) e tocar o som de volta com outro volume, um
//! roteador de áudio completo (atraso, ponto roxo de gravação sempre aceso,
//! permissão, e sem um Mac para testar). Então, no Mac, a setinha do card
//! abre a lista das saídas de áudio (alto-falantes, fones, AirPlay…) para
//! escolher a padrão, como no Controle Central; o volume geral continua no
//! slider logo acima. Funções do Core Audio que existem desde o macOS 10,
//! então vale para o macOS 12+.

use super::audio::{default_output, get};
use objc2_core_audio::*;
use objc2_core_foundation::{CFRetained, CFString};
use std::ptr::NonNull;

/// Uma saída de áudio: o dispositivo e o nome dele.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub id: AudioObjectID,
    pub name: String,
}

fn addr(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}

/// Tamanho (bytes) de uma propriedade.
fn size_of_prop(obj: AudioObjectID, a: AudioObjectPropertyAddress) -> u32 {
    let mut size = 0u32;
    let st = unsafe { AudioObjectGetPropertyDataSize(obj, NonNull::from(&a), 0, std::ptr::null(), NonNull::from(&mut size)) };
    if st == 0 { size } else { 0 }
}

fn name(dev: AudioObjectID) -> Option<String> {
    let p: *const CFString = get(dev, addr(kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal), std::ptr::null())?;
    let s = NonNull::new(p as *mut CFString).map(|p| unsafe { CFRetained::from_raw(p) })?;
    Some(s.to_string())
}

/// Saídas de áudio disponíveis (dispositivos com canais de saída).
pub fn list() -> Vec<Output> {
    let sys = kAudioObjectSystemObject as u32;
    let a = addr(kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal);
    let n = size_of_prop(sys, a) as usize / std::mem::size_of::<AudioObjectID>();
    if n == 0 {
        return Vec::new();
    }
    let mut ids = vec![0 as AudioObjectID; n];
    let mut size = (n * std::mem::size_of::<AudioObjectID>()) as u32;
    let st = unsafe { AudioObjectGetPropertyData(sys, NonNull::from(&a), 0, std::ptr::null(), NonNull::from(&mut size), NonNull::new(ids.as_mut_ptr()).unwrap().cast()) };
    if st != 0 {
        return Vec::new();
    }
    ids.truncate(size as usize / std::mem::size_of::<AudioObjectID>());
    ids.into_iter()
        .filter(|&d| size_of_prop(d, addr(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeOutput)) > 0)
        .filter_map(|id| name(id).filter(|n| !n.trim().is_empty()).map(|name| Output { id, name }))
        .collect()
}

/// Saída padrão atual.
pub fn current() -> Option<AudioObjectID> {
    default_output()
}

/// Escolhe a saída padrão (como no Controle Central).
pub fn set_default(id: AudioObjectID) -> bool {
    let a = addr(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal);
    let v = id;
    unsafe { AudioObjectSetPropertyData(kAudioObjectSystemObject as u32, NonNull::from(&a), 0, std::ptr::null(), std::mem::size_of::<AudioObjectID>() as u32, NonNull::from(&v).cast()) == 0 }
}
