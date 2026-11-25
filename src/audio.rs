//! Captura WASAPI em modo *loopback* do dispositivo de saída padrão.
//!
//! O loopback devolve exatamente o mix que está indo para os alto-falantes,
//! então funciona com qualquer player: Spotify, YouTube Music (navegador),
//! Apple Music, Deezer, Tidal, VLC, Windows Media Player etc.
//! Se o dispositivo padrão mudar (ex.: conectou um fone), a captura reinicia.

use crate::shared::Shared;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{Interface, Result};

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
            let _ = shared.audio_thread.set(std::thread::current());
            while shared.is_running() {
                // Visualizador desligado e card fechado (ou sem as barras):
                // nada de capturar áudio. Parada, acorda a cada 400 ms ou na
                // hora em que o card abre com as barras (o overlay a acorda).
                if !shared.wants_audio() {
                    std::thread::park_timeout(Duration::from_millis(400));
                    continue;
                }
                match capture_any(&shared) {
                    Ok(()) => {} // dispositivo padrão ou player mudou: reinicia imediatamente
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

/// App de mídia atual (AUMID da sessão do Windows); vazio = nada tocando.
fn media_app(shared: &Shared) -> String {
    let m = shared.media.lock().unwrap();
    if m.present() { m.aumid.clone() } else { String::new() }
}

/// O Windows não tem captura por processo (anterior ao Windows 10 2004): só a do sistema.
static NO_PROCESS_LOOPBACK: AtomicBool = AtomicBool::new(false);

/// Escolhe o que ouvir. Com "Só o som do player" (padrão), só o processo do
/// app que está tocando e os filhos dele (o áudio dos navegadores sai de um
/// processo filho): chamadas, jogos e avisos do sistema não mexem nas barras.
/// Sem player tocando, não capta nada. Sem como achar o processo do player
/// (ou num Windows sem captura por processo), volta ao som do sistema inteiro.
fn capture_any(shared: &Shared) -> Result<()> {
    if !shared.config.lock().unwrap().viz_player_only {
        return capture(shared, None);
    }
    let app = media_app(shared);
    if app.is_empty() {
        std::thread::sleep(Duration::from_millis(400));
        return Ok(());
    }
    if !NO_PROCESS_LOOPBACK.load(Ordering::Relaxed) {
        match process::root_of(&app) {
            Some(pid) => match start_process_capture(pid) {
                Ok(started) => return process_capture(shared, &app, pid, started),
                // Não deu para começar (ex.: player rodando como administrador):
                // som do sistema até o player mudar.
                Err(e) => {
                    log::warn(&format!("captura só do player falhou ({app}, pid {pid}): {e}; usando o som do sistema"));
                    if e.code() == windows::Win32::Foundation::E_NOTIMPL || e.code() == windows::Win32::Foundation::E_NOINTERFACE {
                        NO_PROCESS_LOOPBACK.store(true, Ordering::Relaxed);
                    }
                }
            },
            None => process::log_unknown(&app),
        }
    }
    capture(shared, Some(&app))
}

#[windows_core::implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(Arc<(Mutex<bool>, Condvar)>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(&self, _op: windows_core::Ref<IActivateAudioInterfaceAsyncOperation>) -> Result<()> {
        let (done, cv) = &*self.0;
        *done.lock().unwrap() = true;
        cv.notify_all();
        Ok(())
    }
}

/// Cliente de áudio que capta só o processo `pid` e os filhos dele.
fn process_client(pid: u32) -> Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS { TargetProcessId: pid, ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE },
        },
    };
    // O PROPVARIANT do crate libera o conteúdo ao sair de escopo: aqui ele só
    // aponta para `params` (na pilha), então não pode ser liberado.
    let mut pv = std::mem::ManuallyDrop::new(PROPVARIANT::default());
    unsafe {
        let inner = &mut *pv.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB { cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &mut params as *mut _ as *mut u8 };
    }
    let signal = Arc::new((Mutex::new(false), Condvar::new()));
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(signal.clone()).into();
    let op = unsafe { ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*pv), &handler)? };
    {
        let (done, cv) = &*signal;
        let (guard, _) = cv.wait_timeout_while(done.lock().unwrap(), Duration::from_secs(5), |d| !*d).unwrap();
        if !*guard {
            return Err(windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "captura por processo não respondeu"));
        }
    }
    let mut hr = windows::core::HRESULT(0);
    let mut unk: Option<windows::core::IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut unk)? };
    hr.ok()?;
    unk.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_POINTER))?.cast()
}

const PROCESS_RATE: u32 = 48_000;

/// Captura do processo pronta para começar.
struct Started {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: windows::Win32::Foundation::HANDLE,
}

/// Prepara a captura só do processo `pid` (e filhos).
fn start_process_capture(pid: u32) -> Result<Started> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::CreateEventW;
    unsafe {
        let client = process_client(pid)?;
        // A captura por processo não informa o formato do mix: pedimos float
        // estéreo e o Windows converte.
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
            nChannels: 2,
            nSamplesPerSec: PROCESS_RATE,
            nAvgBytesPerSec: PROCESS_RATE * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            1_000_000,
            0,
            &fmt,
            None,
        )?;
        let event = CreateEventW(None, false, false, None)?;
        let ready = client.SetEventHandle(event).and_then(|_| client.GetService::<IAudioCaptureClient>());
        match ready {
            Ok(capture) => Ok(Started { client, capture, event }),
            Err(e) => {
                let _ = CloseHandle(event);
                Err(e)
            }
        }
    }
}

/// Capta só o app que está tocando. Roda até o player mudar, o processo
/// fechar ou a opção mudar (Ok), ou até um erro (Err).
fn process_capture(shared: &Shared, app: &str, pid: u32, st: Started) -> Result<()> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    unsafe {
        let proc = OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok();
        let result = (|| {
            shared.sample_rate.store(PROCESS_RATE, Ordering::Relaxed);
            st.client.Start()?;
            log::info(&format!("captura só do player: {app} (pid {pid}), {PROCESS_RATE} Hz"));
            let mut last_check = Instant::now();
            loop {
                if !shared.is_running() {
                    return Ok(());
                }
                // Sem som o Windows não manda pacotes: o tempo limite mantém as checagens.
                let _ = WaitForSingleObject(st.event, 100);
                if last_check.elapsed() > Duration::from_millis(1000) {
                    last_check = Instant::now();
                    let cfg_on = shared.wants_audio() && shared.player_only();
                    if !cfg_on || media_app(shared) != app {
                        return Ok(());
                    }
                    if proc.is_some_and(|h| WaitForSingleObject(h, 0) == WAIT_OBJECT_0) {
                        log::info(&format!("o processo do player fechou ({app})"));
                        return Ok(());
                    }
                }
                drain(&st.capture, shared, 2, 8, SampleKind::F32)?;
            }
        })();
        let _ = st.client.Stop();
        if let Some(h) = proc {
            let _ = CloseHandle(h);
        }
        let _ = CloseHandle(st.event);
        result
    }
}

/// Roda até o dispositivo padrão mudar ou, com `player` (som do sistema por não
/// achar o processo do player), até o player mudar (Ok), ou ocorrer um erro (Err).
fn capture(shared: &Shared, player: Option<&str>) -> Result<()> {
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
                let (viz, only) = (shared.wants_audio(), shared.player_only());
                if !viz {
                    break Ok(()); // desligaram o visualizador (ou fecharam o card): solta o dispositivo
                }
                // Ligaram "Só o som do player", ou o player mudou: escolhe de novo.
                if only && player.is_none_or(|p| media_app(shared) != p) {
                    break Ok(());
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

/// Processo do player a partir do AUMID da sessão de mídia do Windows.
pub(crate) mod process {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
    use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::core::PWSTR;

    pub(crate) struct Proc {
        pub pid: u32,
        pub parent: u32,
        /// Nome do executável sem ".exe", em minúsculas.
        pub stem: String,
    }

    pub(crate) fn processes() -> Vec<Proc> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
                let stem = name.strip_suffix(".exe").unwrap_or(&name).to_string();
                out.push(Proc { pid: e.th32ProcessID, parent: e.th32ParentProcessID, stem });
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        out
    }

    /// AUMID de um processo de app da Store (vazio nos programas comuns).
    pub(crate) fn package_aumid(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            let r = GetApplicationUserModelId(h, &mut len, Some(PWSTR(buf.as_mut_ptr())));
            let _ = CloseHandle(h);
            (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]))
        }
    }

    /// Nomes de executável que o AUMID sugere: "Spotify.exe" → spotify;
    /// "Brave._crx_…" (app instalado do navegador) → brave; "MSEdge" → msedge;
    /// "com.squirrel.Discord.Discord" → discord.
    pub(crate) fn stems(aumid: &str) -> Vec<String> {
        let last = aumid.rsplit(['\\', '/']).next().unwrap_or(aumid).to_lowercase();
        let base = last.strip_suffix(".exe").unwrap_or(&last).to_string();
        let mut out = vec![base.clone()];
        for part in [base.split('.').next(), base.rsplit('.').next()].into_iter().flatten() {
            if !part.is_empty() && !out.iter().any(|s| s == part) {
                out.push(part.to_string());
            }
        }
        out
    }

    /// O processo "raiz" do player (o de cima entre os do mesmo app): a
    /// captura inclui os filhos, onde fica o áudio dos navegadores.
    pub fn root_of(aumid: &str) -> Option<u32> {
        let procs = processes();
        let hits: Vec<&Proc> = if let Some((family, _)) = aumid.split_once('!') {
            // App da Store: o processo tem o mesmo AUMID (mesmo pacote).
            let family = family.to_lowercase();
            procs.iter().filter(|p| package_aumid(p.pid).is_some_and(|a| a.to_lowercase().starts_with(&family))).collect()
        } else {
            let want = stems(aumid);
            procs.iter().filter(|p| want.iter().any(|w| *w == p.stem)).collect()
        };
        let ids: HashSet<u32> = hits.iter().map(|p| p.pid).collect();
        hits.iter().find(|p| !ids.contains(&p.parent)).or(hits.first()).map(|p| p.pid)
    }

    /// Avisa uma vez por app que não deu para achar o processo dele.
    pub fn log_unknown(aumid: &str) {
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        if !seen.iter().any(|s| s == aumid) {
            seen.push(aumid.to_string());
            super::log::info(&format!("processo do player não encontrado ({aumid}): barras com o som do sistema"));
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn stems_from_aumids() {
            assert_eq!(super::stems("Spotify.exe"), ["spotify"]);
            assert!(super::stems("Brave._crx_cinhimbnkkghhklpknlkffjgod").contains(&"brave".to_string()));
            assert!(super::stems("MSEdge").contains(&"msedge".to_string()));
            assert!(super::stems("com.squirrel.Discord.Discord").contains(&"discord".to_string()));
            assert!(super::stems(r"C:\Program Files\foobar2000\foobar2000.exe").contains(&"foobar2000".to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Capta 2 s só do player que o Windows diz que está tocando agora e
    /// mostra o pico do som recebido.
    /// `cargo test --release player_capture -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn player_capture() {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let m = Manager::RequestAsync().and_then(|op| op.join()).unwrap();
        let app = m.GetCurrentSession().unwrap().SourceAppUserModelId().unwrap().to_string();
        let t = Instant::now();
        let pid = process::root_of(&app);
        println!("player: {app} → pid {pid:?} ({:?})", t.elapsed());
        let st = start_process_capture(pid.unwrap()).unwrap();
        unsafe {
            st.client.Start().unwrap();
            let (mut frames, mut peak) = (0usize, 0f32);
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(20));
                while st.capture.GetNextPacketSize().unwrap() > 0 {
                    let (mut data, mut n, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                    st.capture.GetBuffer(&mut data, &mut n, &mut flags, None, None).unwrap();
                    let s = std::slice::from_raw_parts(data as *const f32, n as usize * 2);
                    if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 {
                        peak = s.iter().fold(peak, |p, v| p.max(v.abs()));
                    }
                    frames += n as usize;
                    st.capture.ReleaseBuffer(n).unwrap();
                }
            }
            let _ = st.client.Stop();
            println!("quadros: {frames} (~{:.1} s), pico: {peak:.3}", frames as f32 / PROCESS_RATE as f32);
        }
    }
}
