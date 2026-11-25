//! Mixer de volume do card: o volume de cada app que usa o dispositivo de
//! saída padrão, como o Mixer de Volume do Windows.
//!
//! Sessões de áudio do WASAPI (`IAudioSessionManager2`): as do mesmo app
//! (mesmo .exe, ex.: os vários processos de áudio do navegador) viram uma
//! linha só; os sons do sistema ficam numa linha própria. O volume de cada
//! sessão é o `ISimpleAudioVolume` dela.
//!
//! Uma thread própria só trabalha com o mixer aberto: lê a lista ao abrir e
//! quando o Windows avisa (sessão nova; volume, mudo ou estado de uma sessão
//! mudou), além de uma conferida a cada 3 s (troca do dispositivo padrão).
//! Fechado, ela desliga os avisos e fica parada. Nada roda por quadro.

use crate::appicon;
use crate::audio::log;
use crate::render::Image;
use crate::shared::{MixApp, MixCmd, MixerState, Shared};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::Duration;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AudioSessionDisconnectReason, AudioSessionState, AudioSessionStateExpired, AudioSessionStateActive, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionEvents, IAudioSessionEvents_Impl, IAudioSessionManager2, IAudioSessionNotification, IAudioSessionNotification_Impl, IMMDevice,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ};
use windows::core::{BOOL, GUID, HSTRING, Interface, PCWSTR, Ref, Result, implement};

pub fn spawn(shared: Arc<Shared>) {
    let (tx, rx) = channel::<MixCmd>();
    *shared.mixer_tx.lock().unwrap() = Some(tx.clone());
    let _ = std::thread::Builder::new().name("mixer".into()).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        crate::system::background_thread();
        let mut m = Mixer { tx, open: None, icons: HashMap::new(), names: HashMap::new() };
        while shared.is_running() {
            // Aberto: confere a cada 3 s; fechado, só espera um pedido.
            let wait = if m.open.is_some() { Duration::from_secs(3) } else { Duration::from_secs(1) };
            let cmd = match rx.recv_timeout(wait) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => m.open.is_some().then_some(MixCmd::Refresh),
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let Some(cmd) = cmd else { continue };
            // Junta rajadas (arrastar o slider manda vários volumes; o Windows
            // avisa de cada mudança): vale o último de cada.
            let mut cmds = vec![cmd];
            while let Ok(c) = rx.recv_timeout(Duration::from_millis(15)) {
                cmds.push(c);
                if cmds.len() > 64 {
                    break;
                }
            }
            let mut publish = false;
            let mut last_vol: Vec<(String, f32)> = Vec::new();
            for c in cmds {
                match c {
                    MixCmd::Open => {
                        m.open();
                        publish = true;
                    }
                    MixCmd::Close => m.close(),
                    MixCmd::Refresh => publish = true,
                    MixCmd::Volume(k, v) => match last_vol.iter_mut().find(|(kk, _)| *kk == k) {
                        Some(e) => e.1 = v,
                        None => last_vol.push((k, v)),
                    },
                    MixCmd::Mute(k, on) => {
                        m.each_volume(&k, |sv| unsafe {
                            let _ = sv.SetMute(on, std::ptr::null());
                        });
                        publish = true;
                    }
                }
            }
            for (k, v) in last_vol {
                m.each_volume(&k, |sv| unsafe {
                    let _ = sv.SetMasterVolume(v.clamp(0.0, 1.0), std::ptr::null());
                });
                publish = true;
            }
            if publish && m.open.is_some() {
                let state = m.read();
                *shared.mixer.lock().unwrap() = state;
                shared.mixer_version.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

/// Avisa a thread quando uma sessão nova aparece.
#[implement(IAudioSessionNotification)]
struct NewSession(Sender<MixCmd>);

impl IAudioSessionNotification_Impl for NewSession_Impl {
    fn OnSessionCreated(&self, _new: Ref<IAudioSessionControl>) -> Result<()> {
        let _ = self.0.send(MixCmd::Refresh);
        Ok(())
    }
}

/// Avisa a thread quando o volume, o mudo ou o estado de uma sessão muda.
#[implement(IAudioSessionEvents)]
struct SessionEvents(Sender<MixCmd>);

impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnSimpleVolumeChanged(&self, _: f32, _: BOOL, _: *const GUID) -> Result<()> {
        let _ = self.0.send(MixCmd::Refresh);
        Ok(())
    }
    fn OnChannelVolumeChanged(&self, _: u32, _: *const f32, _: u32, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> Result<()> {
        Ok(())
    }
    fn OnStateChanged(&self, _: AudioSessionState) -> Result<()> {
        let _ = self.0.send(MixCmd::Refresh);
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> Result<()> {
        let _ = self.0.send(MixCmd::Refresh);
        Ok(())
    }
}

/// Uma sessão de áudio aberta e os avisos registrados nela.
struct Session {
    ctl: IAudioSessionControl2,
    vol: ISimpleAudioVolume,
    events: IAudioSessionEvents,
    key: String,
}

/// Mixer aberto: o dispositivo, o gerenciador de sessões e os avisos.
struct Open {
    device_id: String,
    device: String,
    mgr: IAudioSessionManager2,
    notify: IAudioSessionNotification,
    sessions: Vec<Session>,
}

struct Mixer {
    tx: Sender<MixCmd>,
    open: Option<Open>,
    /// Ícones e nomes dos apps já lidos (pelo .exe).
    icons: HashMap<String, Option<Arc<Image>>>,
    names: HashMap<String, String>,
}

impl Mixer {
    fn open(&mut self) {
        self.close();
        match self.setup() {
            Ok(o) => self.open = Some(o),
            Err(e) => log::warn(&format!("mixer: não deu para abrir as sessões de áudio: {e}")),
        }
    }

    fn setup(&self) -> Result<Open> {
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let device_id = device_id(&dev);
            let device = device_name(&dev);
            let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None)?;
            let notify: IAudioSessionNotification = NewSession(self.tx.clone()).into();
            mgr.RegisterSessionNotification(&notify)?;
            Ok(Open { device_id, device, mgr, notify, sessions: Vec::new() })
        }
    }

    fn close(&mut self) {
        if let Some(o) = self.open.take() {
            unsafe {
                let _ = o.mgr.UnregisterSessionNotification(&o.notify);
                for s in &o.sessions {
                    let _ = s.ctl.UnregisterAudioSessionNotification(&s.events);
                }
            }
        }
    }

    /// Chama `f` com o volume de cada sessão do app `key`.
    fn each_volume(&self, key: &str, mut f: impl FnMut(&ISimpleAudioVolume)) {
        if let Some(o) = &self.open {
            for s in o.sessions.iter().filter(|s| s.key == key) {
                f(&s.vol);
            }
        }
    }

    /// Lê as sessões (e registra os avisos nas novas) e monta a lista por app.
    fn read(&mut self) -> MixerState {
        // O dispositivo padrão mudou (fone, HDMI): recomeça nele.
        let changed = self.open.as_ref().is_some_and(|o| unsafe {
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .and_then(|en| en.GetDefaultAudioEndpoint(eRender, eConsole))
                .map(|d| device_id(&d) != o.device_id)
                .unwrap_or(false)
        });
        if changed {
            self.open();
        }
        let Some(o) = self.open.as_mut() else { return MixerState::default() };
        let mut list: Vec<(IAudioSessionControl2, ISimpleAudioVolume, bool, bool, String)> = Vec::new();
        unsafe {
            let Ok(en) = o.mgr.GetSessionEnumerator() else { return MixerState::default() };
            for i in 0..en.GetCount().unwrap_or(0) {
                let Ok(ctl) = en.GetSession(i) else { continue };
                let state = ctl.GetState().unwrap_or(AudioSessionStateExpired);
                if state == AudioSessionStateExpired {
                    continue;
                }
                let (Ok(ctl2), Ok(vol)) = (ctl.cast::<IAudioSessionControl2>(), ctl.cast::<ISimpleAudioVolume>()) else { continue };
                let system = ctl2.IsSystemSoundsSession().0 == 0;
                let key = if system {
                    "system".to_string()
                } else {
                    let pid = ctl2.GetProcessId().unwrap_or(0);
                    appicon::exe_path(pid).unwrap_or_else(|| format!("pid:{pid}"))
                };
                list.push((ctl2, vol, state == AudioSessionStateActive, system, key));
            }
        }
        // Avisos: registra nas sessões novas; esquece as que sumiram.
        let mut kept: Vec<Session> = Vec::new();
        for (ctl, vol, _, _, key) in &list {
            match o.sessions.iter().position(|s| s.ctl.as_raw() == ctl.as_raw()) {
                Some(i) => kept.push(o.sessions.swap_remove(i)),
                None => {
                    let events: IAudioSessionEvents = SessionEvents(self.tx.clone()).into();
                    unsafe {
                        let _ = ctl.RegisterAudioSessionNotification(&events);
                    }
                    kept.push(Session { ctl: ctl.clone(), vol: vol.clone(), events, key: key.clone() });
                }
            }
        }
        for s in o.sessions.drain(..) {
            unsafe {
                let _ = s.ctl.UnregisterAudioSessionNotification(&s.events);
            }
        }
        o.sessions = kept;
        let device = o.device.clone();

        // Junta por app: volume = o maior; mudo só se todas estiverem mudas.
        let mut apps: Vec<MixApp> = Vec::new();
        for (_, vol, active, system, key) in &list {
            let (v, mute) = unsafe { (vol.GetMasterVolume().unwrap_or(1.0), vol.GetMute().map(|b| b.as_bool()).unwrap_or(false)) };
            match apps.iter_mut().find(|a| a.key == *key) {
                Some(a) => {
                    a.volume = a.volume.max(v);
                    a.muted &= mute;
                    a.active |= *active;
                }
                None => {
                    let (name, icon) = if *system { (String::new(), None) } else { (self.name_of(key), self.icon_of(key)) };
                    apps.push(MixApp { key: key.clone(), name, icon, volume: v, muted: mute, active: *active, system: *system });
                }
            }
        }
        // Tocando primeiro, depois os outros; os sons do sistema no fim.
        apps.sort_by_key(|a| (a.system, !a.active, a.name.to_lowercase()));
        MixerState { device, apps }
    }

    fn icon_of(&mut self, path: &str) -> Option<Arc<Image>> {
        self.icons.entry(path.to_string()).or_insert_with(|| appicon::exe_icon(path).map(Arc::new)).clone()
    }

    fn name_of(&mut self, path: &str) -> String {
        self.names.entry(path.to_string()).or_insert_with(|| app_name(path)).clone()
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        self.close();
    }
}

fn device_id(dev: &IMMDevice) -> String {
    unsafe {
        dev.GetId()
            .map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as _));
                s
            })
            .unwrap_or_default()
    }
}

/// Nome do dispositivo de saída ("Alto-falantes (Realtek…)").
fn device_name(dev: &IMMDevice) -> String {
    unsafe { dev.OpenPropertyStore(STGM_READ).and_then(|ps| ps.GetValue(&PKEY_Device_FriendlyName)).map(|v| v.to_string()).unwrap_or_default() }
}

/// Nome do app: a descrição do .exe ("Brave Browser", "Discord"); sem ela,
/// o nome do arquivo.
fn app_name(path: &str) -> String {
    let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let fallback = || {
        let mut c = stem.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    file_description(path).filter(|d| !d.trim().is_empty()).unwrap_or_else(fallback)
}

fn file_description(path: &str) -> Option<String> {
    unsafe {
        let name = HSTRING::from(path);
        let size = GetFileVersionInfoSizeW(&name, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(&name, None, size, buf.as_mut_ptr() as _).ok()?;
        // Primeiro idioma/código de página da tabela de traduções.
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(buf.as_ptr() as _, &HSTRING::from("\\VarFileInfo\\Translation"), &mut p, &mut len).as_bool() || len < 4 {
            return None;
        }
        let tr = std::slice::from_raw_parts(p as *const u16, 2);
        let key = format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", tr[0], tr[1]);
        if !VerQueryValueW(buf.as_ptr() as _, &HSTRING::from(key), &mut p, &mut len).as_bool() || len == 0 {
            return None;
        }
        let s = std::slice::from_raw_parts(p as *const u16, len as usize);
        Some(String::from_utf16_lossy(s).trim_end_matches('\0').trim().to_string())
    }
}

/// O mixer desta máquina agora (capturas).
#[cfg(test)]
pub fn snapshot() -> MixerState {
    let (tx, _rx) = channel();
    let mut m = Mixer { tx, open: None, icons: HashMap::new(), names: HashMap::new() };
    m.open();
    m.read()
}

#[cfg(test)]
mod tests {
    /// Lista os apps do mixer desta máquina (sem mexer em nada).
    /// `cargo test --release mixer_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn mixer_here() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut m = super::Mixer { tx, open: None, icons: Default::default(), names: Default::default() };
        let t = std::time::Instant::now();
        m.open();
        let st = m.read();
        println!("dispositivo: {} ({:?})", st.device, t.elapsed());
        for a in &st.apps {
            println!("  {:<28} vol={:.2} mudo={} tocando={} sistema={} ícone={} [{}]", a.name, a.volume, a.muted, a.active, a.system, a.icon.is_some(), a.key);
        }
        let t = std::time::Instant::now();
        let _ = m.read();
        println!("releitura: {:?}", t.elapsed());
    }
}
