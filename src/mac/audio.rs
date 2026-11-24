//! Som que está tocando no Mac, para as barras de áudio: um *process tap* do
//! Core Audio (macOS 14.2+) com a mistura mono de todos os apps, ligado a um
//! dispositivo agregado privado (não aparece em Ajustes > Som).
//!
//! Na primeira vez o macOS pede a permissão "Gravação de áudio do sistema"
//! (texto em `NSAudioCaptureUsageDescription`). Sem ela o tap entrega silêncio:
//! as barras ficam em repouso e o resto do app funciona igual.
//! Só captura com as barras ligadas (como no Windows).

use super::system::{self, log};
use crate::shared::Shared;
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2_core_audio::*;
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_foundation::{NSArray, NSNumber, NSString, NSUUID};
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Duration;

fn addr(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}

/// Lê uma propriedade de tamanho fixo.
pub(super) fn get<T: Copy>(obj: AudioObjectID, a: AudioObjectPropertyAddress, init: T) -> Option<T> {
    let mut v = init;
    let mut size = std::mem::size_of::<T>() as u32;
    let st = unsafe { AudioObjectGetPropertyData(obj, NonNull::from(&a), 0, std::ptr::null(), NonNull::from(&mut size), NonNull::from(&mut v).cast()) };
    (st == 0).then_some(v)
}

pub(super) fn default_output() -> Option<AudioObjectID> {
    get(kAudioObjectSystemObject as u32, addr(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal), 0u32).filter(|&d| d != 0)
}

/// Identificador (UID) de um dispositivo.
fn device_uid(dev: AudioObjectID) -> Option<CFRetained<CFString>> {
    let p: *const CFString = get(dev, addr(kAudioDevicePropertyDeviceUID, kAudioObjectPropertyScopeGlobal), std::ptr::null())?;
    NonNull::new(p as *mut CFString).map(|p| unsafe { CFRetained::from_raw(p) })
}

fn key(k: &std::ffi::CStr) -> CFRetained<CFString> {
    CFString::from_str(k.to_str().unwrap_or_default())
}

/// Tap + dispositivo agregado + IOProc rodando.
struct Capture {
    tap: AudioObjectID,
    agg: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    output_uid: String,
}

impl Capture {
    fn start(shared: &Arc<Shared>) -> Option<Self> {
        // A classe só existe a partir do macOS 14.2.
        objc2::runtime::AnyClass::get(c"CATapDescription")?;
        let out = default_output()?;
        let out_uid = device_uid(out)?;

        // Mistura mono de todos os processos (sem excluir nenhum).
        let desc = unsafe { CATapDescription::initMonoGlobalTapButExcludeProcesses(CATapDescription::alloc(), &NSArray::<NSNumber>::new()) };
        let uuid = NSUUID::new();
        unsafe {
            desc.setUUID(&uuid);
            desc.setName(&NSString::from_str("Taskbeat"));
            desc.setPrivate(true);
            desc.setMuteBehavior(CATapMuteBehavior::Unmuted);
        }
        let mut tap: AudioObjectID = 0;
        let st = unsafe { AudioHardwareCreateProcessTap(Some(&desc), &mut tap) };
        if st != 0 || tap == 0 {
            log::warn(&format!("áudio: tap não criado ({st})"));
            return None;
        }
        let tap_uid = CFString::from_str(&uuid.UUIDString().to_string());

        // Dispositivo agregado privado: saída padrão + o tap.
        let sub_dev = CFDictionary::<CFString, CFType>::from_slices(&[&key(kAudioSubDeviceUIDKey)], &[out_uid.as_ref()]);
        let sub_tap = CFDictionary::<CFString, CFType>::from_slices(&[&key(kAudioSubTapUIDKey), &key(kAudioSubTapDriftCompensationKey)], &[tap_uid.as_ref(), CFBoolean::new(true).as_ref()]);
        let devs = CFArray::from_objects(&[&*sub_dev]);
        let taps = CFArray::from_objects(&[&*sub_tap]);
        let agg_uid = CFString::from_str(&NSUUID::new().UUIDString().to_string());
        let name = CFString::from_str("Taskbeat Tap");
        let keys = [
            key(kAudioAggregateDeviceNameKey),
            key(kAudioAggregateDeviceUIDKey),
            key(kAudioAggregateDeviceMainSubDeviceKey),
            key(kAudioAggregateDeviceIsPrivateKey),
            key(kAudioAggregateDeviceIsStackedKey),
            key(kAudioAggregateDeviceTapAutoStartKey),
            key(kAudioAggregateDeviceSubDeviceListKey),
            key(kAudioAggregateDeviceTapListKey),
        ];
        let t = CFBoolean::new(true);
        let f = CFBoolean::new(false);
        let values: [&CFType; 8] = [name.as_ref(), agg_uid.as_ref(), out_uid.as_ref(), t.as_ref(), f.as_ref(), t.as_ref(), devs.as_ref(), taps.as_ref()];
        let key_refs: Vec<&CFString> = keys.iter().map(|k| &**k).collect();
        let dict = CFDictionary::<CFString, CFType>::from_slices(&key_refs, &values);
        let mut agg: AudioObjectID = 0;
        let st = unsafe { AudioHardwareCreateAggregateDevice(dict.as_ref(), NonNull::from(&mut agg)) };
        if st != 0 || agg == 0 {
            log::warn(&format!("áudio: dispositivo agregado não criado ({st})"));
            unsafe { AudioHardwareDestroyProcessTap(tap) };
            return None;
        }

        // Formato do tap (Float32; mono pedido, mas lê o que vier).
        let zero = AudioStreamBasicDescription { mSampleRate: 0.0, mFormatID: 0, mFormatFlags: 0, mBytesPerPacket: 0, mFramesPerPacket: 0, mBytesPerFrame: 0, mChannelsPerFrame: 0, mBitsPerChannel: 0, mReserved: 0 };
        let fmt = get(tap, addr(kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal), zero).unwrap_or(zero);
        if fmt.mSampleRate > 0.0 {
            shared.sample_rate.store(fmt.mSampleRate as u32, std::sync::atomic::Ordering::Relaxed);
        }
        let channels = fmt.mChannelsPerFrame.max(1) as usize;

        let sh = shared.clone();
        let block = RcBlock::new(move |_now: NonNull<AudioTimeStamp>, input: NonNull<AudioBufferList>, _t: NonNull<AudioTimeStamp>, _out: NonNull<AudioBufferList>, _ot: NonNull<AudioTimeStamp>| {
            let list = unsafe { input.as_ref() };
            let n = list.mNumberBuffers as usize;
            if n == 0 {
                return;
            }
            let bufs = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), n) };
            let mut ring = sh.audio.lock().unwrap();
            if n == 1 {
                // Intercalado (ou mono): média dos canais.
                let b = &bufs[0];
                let ch = (b.mNumberChannels as usize).max(1).max(if channels > 1 { channels } else { 1 });
                let len = b.mDataByteSize as usize / 4;
                let data = unsafe { std::slice::from_raw_parts(b.mData as *const f32, len) };
                for frame in data.chunks(ch) {
                    ring.push(frame.iter().sum::<f32>() / frame.len() as f32);
                }
            } else {
                // Um buffer por canal.
                let len = bufs.iter().map(|b| b.mDataByteSize as usize / 4).min().unwrap_or(0);
                for i in 0..len {
                    let mut acc = 0.0;
                    for b in bufs {
                        acc += unsafe { *(b.mData as *const f32).add(i) };
                    }
                    ring.push(acc / n as f32);
                }
            }
        });
        let mut proc_id: AudioDeviceIOProcID = None;
        let st = unsafe { AudioDeviceCreateIOProcIDWithBlock(NonNull::from(&mut proc_id), agg, None, RcBlock::as_ptr(&block)) };
        // O Core Audio copia o bloco; o nosso pode ser solto.
        drop(block);
        if st != 0 || proc_id.is_none() {
            log::warn(&format!("áudio: IOProc não criado ({st})"));
            unsafe {
                AudioHardwareDestroyAggregateDevice(agg);
                AudioHardwareDestroyProcessTap(tap);
            }
            return None;
        }
        let st = unsafe { AudioDeviceStart(agg, proc_id) };
        if st != 0 {
            log::warn(&format!("áudio: não iniciou ({st})"));
        }
        log::info(&format!("áudio: capturando ({} Hz, {channels} canal/is)", fmt.mSampleRate));
        Some(Self { tap, agg, proc_id, output_uid: out_uid.to_string() })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            AudioDeviceStop(self.agg, self.proc_id);
            AudioDeviceDestroyIOProcID(self.agg, self.proc_id);
            AudioHardwareDestroyAggregateDevice(self.agg);
            AudioHardwareDestroyProcessTap(self.tap);
        }
    }
}

pub fn spawn(shared: Arc<Shared>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("audio-tap".into())
        .spawn(move || {
            system::background_thread();
            let mut cap: Option<Capture> = None;
            let mut failures = 0u32;
            let mut wait = 0u32;
            let mut last_playing: Option<std::time::Instant> = None;
            while shared.is_running() {
                std::thread::sleep(Duration::from_millis(500));
                // Só capta enquanto um player toca (e um pouco depois da pausa):
                // o macOS mostra o ponto roxo de gravação enquanto o tap existe.
                if shared.media.lock().unwrap().playing {
                    last_playing = Some(std::time::Instant::now());
                }
                let playing = last_playing.is_some_and(|t| t.elapsed() < Duration::from_secs(5));
                let want = shared.config.lock().unwrap().show_visualizer && playing;
                if !want {
                    cap = None;
                    continue;
                }
                // Trocou a saída (fone, AirPods…): refaz o tap na nova.
                if let Some(c) = &cap {
                    let now = default_output().and_then(device_uid).map(|u| u.to_string());
                    if now.is_some_and(|u| u != c.output_uid) {
                        log::info("áudio: saída padrão mudou, recriando o tap");
                        cap = None;
                    }
                }
                if cap.is_none() {
                    if wait > 0 {
                        wait -= 1;
                        continue;
                    }
                    cap = Capture::start(&shared);
                    if cap.is_none() {
                        failures += 1;
                        // Espera cada vez mais entre tentativas (até ~30 s).
                        wait = (failures * 4).min(60);
                    } else {
                        failures = 0;
                    }
                }
            }
        })
        .expect("thread de áudio")
}
