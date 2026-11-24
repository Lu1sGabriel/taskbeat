//! Captura WASAPI em modo *loopback* do dispositivo de saída padrão.
//!
//! O loopback devolve exatamente o mix que está indo para os alto-falantes,
//! então funciona com qualquer player: Spotify, YouTube Music (navegador),
//! Apple Music, Deezer, Tidal, VLC, Windows Media Player etc.
//! Se o dispositivo padrão mudar (ex.: conectou um fone), a captura reinicia.

use crate::shared::Shared;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::core::Result;

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

#[derive(Clone, Copy, Debug)]
enum SampleKind {
    F32,
    I16,
    I24,
    I32,
}

pub fn spawn(shared: Arc<Shared>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("audio-loopback".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            crate::system::background_thread();
            while shared.is_running() {
                // Visualizador desligado (padrão): nada de capturar áudio.
                if !shared.config.lock().unwrap().show_visualizer {
                    std::thread::sleep(Duration::from_millis(400));
                    continue;
                }
                match capture(&shared) {
                    Ok(()) => {} // dispositivo padrão mudou: reinicia imediatamente
                    Err(e) => {
                        log::warn(&format!("captura de áudio falhou: {e}; tentando de novo"));
                        std::thread::sleep(Duration::from_millis(1000));
                    }
                }
            }
            unsafe { CoUninitialize() };
        })
        .expect("thread de áudio")
}

fn device_id(device: &IMMDevice) -> Result<String> {
    unsafe {
        let id = device.GetId()?;
        let s = id.to_string().unwrap_or_default();
        CoTaskMemFree(Some(id.0 as _));
        Ok(s)
    }
}

/// Roda até o dispositivo padrão mudar (Ok) ou ocorrer um erro (Err).
fn capture(shared: &Shared) -> Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let id = device_id(&device)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

        let fmt_ptr = client.GetMixFormat()?;
        let fmt = *fmt_ptr;
        let channels = fmt.nChannels.max(1) as usize;
        let rate = fmt.nSamplesPerSec;
        let bits = fmt.wBitsPerSample;
        let mut tag = fmt.wFormatTag;
        if tag == WAVE_FORMAT_EXTENSIBLE && fmt.cbSize >= 22 {
            // WAVEFORMATEXTENSIBLE: SubFormat começa no byte 24; seu Data1 é o "format tag".
            let data1 = std::ptr::read_unaligned((fmt_ptr as *const u8).add(24) as *const u32);
            tag = data1 as u16;
        }
        let kind = match (tag, bits) {
            (WAVE_FORMAT_IEEE_FLOAT, 32) => SampleKind::F32,
            (WAVE_FORMAT_PCM, 16) => SampleKind::I16,
            (WAVE_FORMAT_PCM, 24) => SampleKind::I24,
            (WAVE_FORMAT_PCM, 32) => SampleKind::I32,
            _ => {
                CoTaskMemFree(Some(fmt_ptr as _));
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_NOTIMPL,
                    format!("formato de mix não suportado: tag={tag} bits={bits}"),
                ));
            }
        };
        let block_align = fmt.nBlockAlign as usize;

        // 100 ms de buffer, modo compartilhado + loopback.
        let init = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            1_000_000,
            0,
            fmt_ptr,
            None,
        );
        CoTaskMemFree(Some(fmt_ptr as _));
        init?;

        let capture: IAudioCaptureClient = client.GetService()?;
        shared.sample_rate.store(rate, Ordering::Relaxed);
        client.Start()?;
        log::info(&format!("loopback iniciado: {rate} Hz, {channels} canais, {kind:?}"));

        let mut last_device_check = Instant::now();
        let result = loop {
            if !shared.is_running() {
                break Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));

            if last_device_check.elapsed() > Duration::from_millis(1500) {
                last_device_check = Instant::now();
                if !shared.config.lock().unwrap().show_visualizer {
                    break Ok(()); // desligaram o visualizador: solta o dispositivo
                }
                match enumerator.GetDefaultAudioEndpoint(eRender, eConsole).and_then(|d| device_id(&d)) {
                    Ok(now) if now != id => {
                        log::info("dispositivo de saída padrão mudou");
                        break Ok(());
                    }
                    Err(e) => break Err(e),
                    _ => {}
                }
            }

            if let Err(e) = drain(&capture, shared, channels, block_align, kind) {
                break Err(e);
            }
        };
        let _ = client.Stop();
        result
    }
}

unsafe fn drain(
    capture: &IAudioCaptureClient,
    shared: &Shared,
    channels: usize,
    block_align: usize,
    kind: SampleKind,
) -> Result<()> {
    unsafe {
        loop {
            let packet = capture.GetNextPacketSize()?;
            if packet == 0 {
                return Ok(());
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            let frames_n = frames as usize;
            let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null();
            {
                let mut ring = shared.audio.lock().unwrap();
                if silent {
                    for _ in 0..frames_n {
                        ring.push(0.0);
                    }
                } else {
                    let bytes = std::slice::from_raw_parts(data, frames_n * block_align);
                    for f in 0..frames_n {
                        let frame = &bytes[f * block_align..(f + 1) * block_align];
                        let mut acc = 0.0f32;
                        for c in 0..channels {
                            acc += read_sample(frame, c, kind);
                        }
                        ring.push(acc / channels as f32);
                    }
                }
            }
            capture.ReleaseBuffer(frames)?;
        }
    }
}

#[inline]
fn read_sample(frame: &[u8], ch: usize, kind: SampleKind) -> f32 {
    match kind {
        SampleKind::F32 => {
            let o = ch * 4;
            f32::from_le_bytes([frame[o], frame[o + 1], frame[o + 2], frame[o + 3]])
        }
        SampleKind::I16 => {
            let o = ch * 2;
            i16::from_le_bytes([frame[o], frame[o + 1]]) as f32 / 32768.0
        }
        SampleKind::I24 => {
            let o = ch * 3;
            let v = i32::from_le_bytes([0, frame[o], frame[o + 1], frame[o + 2]]) >> 8;
            v as f32 / 8_388_608.0
        }
        SampleKind::I32 => {
            let o = ch * 4;
            i32::from_le_bytes([frame[o], frame[o + 1], frame[o + 2], frame[o + 3]]) as f32 / 2_147_483_648.0
        }
    }
}

/// Registro de diagnóstico: no console em builds de depuração e, em qualquer
/// build, no arquivo indicado por `TASKBEAT_LOG` (para investigar um problema
/// no app instalado: `$env:TASKBEAT_LOG="$env:TEMP\taskbeat.log"` antes de abrir).
pub mod log {
    use std::io::Write;
    use std::sync::OnceLock;

    fn file() -> Option<&'static std::path::PathBuf> {
        static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
        PATH.get_or_init(|| std::env::var_os("TASKBEAT_LOG").map(Into::into)).as_ref()
    }

    /// Registro detalhado (cada leitura do player) ligado?
    pub fn verbose() -> bool {
        cfg!(debug_assertions) || file().is_some()
    }

    fn write(level: &str, msg: &str) {
        #[cfg(debug_assertions)]
        eprintln!("[{level}] {msg}");
        if let Some(p) = file() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                let _ = writeln!(f, "{t:.3} [{level}] {msg}");
            }
        }
    }

    pub fn info(msg: &str) {
        write("info", msg);
    }

    pub fn warn(msg: &str) {
        write("warn", msg);
    }
}
