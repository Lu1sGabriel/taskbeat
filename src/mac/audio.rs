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

/// Os *process taps* existem neste macOS (14.2+)?
///
/// Confere a versão do sistema, a função do Core Audio e os métodos da classe
/// antes de tocar em qualquer um deles: no macOS 13 a classe pode até existir
/// (interna), sem os métodos, e chamá-los fecha o app na hora.
fn taps_supported() -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};
    let v = NSOperatingSystemVersion { majorVersion: 14, minorVersion: 2, patchVersion: 0 };
    if !NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(v) || super::compat::create_process_tap().is_none() {
        return false;
    }
    let Some(cls) = objc2::runtime::AnyClass::get(c"CATapDescription") else { return false };
    let inst = [
        objc2::sel!(initMonoGlobalTapButExcludeProcesses:),
        objc2::sel!(setUUID:),
        objc2::sel!(setName:),
        objc2::sel!(setPrivate:),
        objc2::sel!(setMuteBehavior:),
    ];
    inst.iter().all(|&s| cls.instance_method(s).is_some())
}

/// A saída tem entradas (fone com microfone, monitor com câmera…)? Num
/// dispositivo agregado elas viram gravação do microfone, e sem
/// `NSMicrophoneUsageDescription` o macOS fecha o app.
fn has_input_streams(dev: AudioObjectID) -> bool {
    let a = addr(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput);
    let mut size = 0u32;
    let st = unsafe { AudioObjectGetPropertyDataSize(dev, NonNull::from(&a), 0, std::ptr::null(), NonNull::from(&mut size)) };
    // Na dúvida (erro ao ler), trata como se tivesse.
    st != 0 || size > 0
}

impl Capture {
    fn start(shared: &Arc<Shared>) -> Option<Self> {
        if !taps_supported() {
            return None;
        }
        let out = default_output()?;
        let out_uid = device_uid(out)?;
        let with_output = !has_input_streams(out);
        if !with_output {
            log::info("áudio: a saída tem entradas; o tap fica sozinho no dispositivo agregado");
        }

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
        // Procurada na hora: no macOS 12/13 a função não existe.
        let create = super::compat::create_process_tap()?;
        let st = unsafe { create(objc2::rc::Retained::as_ptr(&desc).cast(), &mut tap) };
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
        let t = CFBoolean::new(true);
        let f = CFBoolean::new(false);
        let mut keys = vec![
            key(kAudioAggregateDeviceNameKey),
            key(kAudioAggregateDeviceUIDKey),
            key(kAudioAggregateDeviceIsPrivateKey),
            key(kAudioAggregateDeviceIsStackedKey),
            key(kAudioAggregateDeviceTapAutoStartKey),
            key(kAudioAggregateDeviceTapListKey),
        ];
        let mut values: Vec<&CFType> = vec![name.as_ref(), agg_uid.as_ref(), t.as_ref(), f.as_ref(), t.as_ref(), taps.as_ref()];
        // Com a saída de relógio (o normal); sem ela se a saída também grava.
        if with_output {
            keys.push(key(kAudioAggregateDeviceMainSubDeviceKey));
            values.push(out_uid.as_ref());
            keys.push(key(kAudioAggregateDeviceSubDeviceListKey));
            values.push(devs.as_ref());
        }
        let key_refs: Vec<&CFString> = keys.iter().map(|k| &**k).collect();
        let dict = CFDictionary::<CFString, CFType>::from_slices(&key_refs, &values);
        let mut agg: AudioObjectID = 0;
        let st = unsafe { AudioHardwareCreateAggregateDevice(dict.as_ref(), NonNull::from(&mut agg)) };
        if st != 0 || agg == 0 {
            log::warn(&format!("áudio: dispositivo agregado não criado ({st})"));
            super::compat::destroy_process_tap(tap);
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
            // Thread de tempo real do Core Audio: nada aqui pode travar nem
            // entrar em pânico (um pânico fora do Rust fecha o app).
            let list = unsafe { input.as_ref() };
            let n = list.mNumberBuffers as usize;
            if n == 0 {
                return;
            }
            let bufs = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), n) };
            // Buffer sem dados (o tap ainda subindo, sem permissão…): pula.
            if bufs.iter().any(|b| b.mData.is_null() || b.mDataByteSize < 4) {
                return;
            }
            // A thread principal está lendo: perde estes quadros em vez de esperar.
            let Ok(mut ring) = sh.audio.try_lock() else { return };
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
            }
            super::compat::destroy_process_tap(tap);
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
            super::compat::destroy_process_tap(self.tap);
        }
    }
}

// ---------------------------------------------------------------- trava de segurança
//
// Se o app fecha sozinho logo depois de ligar o tap (algo no Core Audio ou na
// privacidade do macOS que não dá para pegar daqui), a próxima abertura acha a
// marca `tap-starting` e desliga o tap nesta versão (`tap-off`): as barras viram
// o indicador "tocando" e o resto funciona. Uma versão nova tenta de novo.

/// Tempo com o tap ligado para considerá-lo seguro.
const TAP_SAFE: Duration = Duration::from_secs(15);

fn guard_file(name: &str) -> Option<std::path::PathBuf> {
    Some(crate::config::Config::path()?.parent()?.join(name))
}

fn mark_starting(on: bool) {
    let Some(p) = guard_file("tap-starting") else { return };
    if on {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, env!("CARGO_PKG_VERSION"));
    } else {
        let _ = std::fs::remove_file(&p);
    }
}

/// O app vai sair normalmente: o tap não foi o que o fechou.
pub fn clean_exit() {
    mark_starting(false);
}

/// O tap está desligado nesta versão (o app fechou ao ligá-lo antes)?
fn tap_disabled() -> bool {
    let (Some(starting), Some(off)) = (guard_file("tap-starting"), guard_file("tap-off")) else { return false };
    if starting.exists() {
        let _ = std::fs::remove_file(&starting);
        let _ = std::fs::write(&off, env!("CARGO_PKG_VERSION"));
        log::crash("o app fechou logo depois de ligar a captura de som: barras de áudio sem som nesta versão");
        return true;
    }
    std::fs::read_to_string(&off).is_ok_and(|v| v.trim() == env!("CARGO_PKG_VERSION"))
}

pub fn spawn(shared: Arc<Shared>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("audio-tap".into())
        .spawn(move || {
            system::background_thread();
            if tap_disabled() {
                log::warn("áudio: captura desligada (o app fechou ao ligá-la); barras mostram o indicador");
                return;
            }
            if !taps_supported() {
                log::info("áudio: este macOS não tem captura do som do sistema (só no 14.2+); barras mostram o indicador");
                return;
            }
            let mut cap: Option<Capture> = None;
            let mut cap_since: Option<std::time::Instant> = None;
            let mut failures = 0u32;
            let mut wait = 0u32;
            let mut last_playing: Option<std::time::Instant> = None;
            while shared.is_running() {
                // O tap aguentou um tempo ligado: tira a marca.
                if cap_since.is_some_and(|t| t.elapsed() > TAP_SAFE) {
                    cap_since = None;
                    mark_starting(false);
                }
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
                    mark_starting(true);
                    cap = Capture::start(&shared);
                    cap_since = cap.as_ref().map(|_| std::time::Instant::now());
                    if cap.is_none() {
                        mark_starting(false);
                        failures += 1;
                        // Espera cada vez mais entre tentativas (até ~30 s).
                        wait = (failures * 4).min(60);
                    } else {
                        failures = 0;
                    }
                }
            }
            mark_starting(false);
        })
        .expect("thread de áudio")
}
