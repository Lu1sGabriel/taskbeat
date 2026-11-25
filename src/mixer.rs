//! Mixer de volume do card: o volume de cada app com sessão de áudio, como o
//! Mixer de Volume do Windows, e a escolha da saída padrão.
//!
//! Sessões do WASAPI (`IAudioSessionManager2`) de TODAS as saídas ativas (um
//! app pode tocar noutra saída, como o NVIDIA Broadcast): as do mesmo app
//! viram uma linha só. O app é o pacote dele quando tem um (apps da Store e o
//! WebView2 que eles usam: o som do WhatsApp sai de um `msedgewebview2.exe`
//! do pacote do WhatsApp) e o .exe nos outros; os sons do sistema ficam numa
//! linha própria. O volume de cada sessão é o `ISimpleAudioVolume` dela.
//!
//! Uma thread própria só lê sessões com o mixer aberto: ao abrir e quando o
//! Windows avisa (sessão nova; volume, mudo ou estado de uma sessão mudou),
//! com uma conferida a cada 10 s. A troca da saída padrão e as saídas que
//! aparecem e somem chegam pelos avisos do `IMMNotificationClient` (sempre
//! ligados: parados, não custam nada); a saída padrão é trocada pelo
//! `IPolicyConfig`, a mesma interface que o EarTrumpet e o SoundSwitch usam.

use crate::appicon;
use crate::audio::log;
use crate::render::Image;
use crate::shared::{MixApp, MixCmd, MixerState, OutDevice, Shared};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    AudioSessionDisconnectReason, AudioSessionState, AudioSessionStateActive, AudioSessionStateExpired, DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole,
    IAudioSessionControl, IAudioSessionControl2, IAudioSessionEvents, IAudioSessionEvents_Impl, IAudioSessionManager2, IAudioSessionNotification,
    IAudioSessionNotification_Impl, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, ISimpleAudioVolume,
    MMDeviceEnumerator, eCommunications, eConsole, eMultimedia, eRender,
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
        // Avisos das saídas (padrão trocada, saída nova ou removida).
        let watch = DeviceWatch::register(tx.clone());
        let mut m = Mixer { tx, open: None, icons: HashMap::new(), names: HashMap::new() };
        while shared.is_running() {
            // Aberto: confere a cada 10 s (os avisos cobrem o resto); fechado,
            // só espera um pedido.
            let wait = if m.open.is_some() { Duration::from_secs(10) } else { Duration::from_secs(1) };
            let cmd = match rx.recv_timeout(wait) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => m.open.is_some().then_some(MixCmd::Refresh),
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let Some(cmd) = cmd else { continue };
            // Junta rajadas (arrastar o slider manda vários volumes; o Windows
            // avisa de cada mudança): vale o último de cada. Janela fixa de
            // 30 ms a partir do primeiro: esperar "até parar de chegar" fazia,
            // em telas de 120 Hz ou mais (um volume a cada ~8 ms), o volume só
            // ser aplicado a cada ~0,4 s durante o arrasto.
            let mut cmds = vec![cmd];
            let until = Instant::now() + Duration::from_millis(30);
            while let Some(left) = until.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(c) => cmds.push(c),
                    Err(_) => break,
                }
            }
            let mut publish = false;
            let mut reopen = false;
            let mut last_vol: Vec<(String, f32)> = Vec::new();
            for c in cmds {
                match c {
                    MixCmd::Open => {
                        m.open();
                        publish = true;
                    }
                    MixCmd::Close => m.close(),
                    MixCmd::Refresh => publish = true,
                    // Saída padrão trocada ou saídas mudaram: abre de novo nelas.
                    MixCmd::DefaultChanged | MixCmd::DevicesChanged => reopen |= m.open.is_some(),
                    MixCmd::SetDevice(id) => match set_default_output(&id) {
                        // O aviso de "padrão mudou" chega em seguida e reabre.
                        Ok(()) => log::info(&format!("mixer: saída padrão trocada para {id}")),
                        Err(e) => {
                            log::warn(&format!("mixer: não deu para trocar a saída padrão: {e}"));
                            shared.output_failed.fetch_add(1, Ordering::Relaxed);
                        }
                    },
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
            if reopen {
                m.open();
                publish = true;
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
        drop(watch);
    });
}

// ---------------------------------------------------------------- saídas

/// Avisos das saídas de áudio: a padrão trocou (pelo app ou por fora dele),
/// uma saída apareceu, sumiu ou mudou de estado.
#[implement(IMMNotificationClient)]
struct DeviceEvents(Sender<MixCmd>);

impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        let _ = self.0.send(MixCmd::DevicesChanged);
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        let _ = self.0.send(MixCmd::DevicesChanged);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        let _ = self.0.send(MixCmd::DevicesChanged);
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            // O volume do card passa a valer para a saída nova.
            crate::volume::output_changed();
            let _ = self.0.send(MixCmd::DefaultChanged);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// Os avisos das saídas registrados (desregistra ao sair).
struct DeviceWatch {
    en: Option<IMMDeviceEnumerator>,
    client: IMMNotificationClient,
}

impl DeviceWatch {
    fn register(tx: Sender<MixCmd>) -> DeviceWatch {
        let client: IMMNotificationClient = DeviceEvents(tx).into();
        let en = unsafe { CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
        let ok = en.as_ref().is_some_and(|e| unsafe { e.RegisterEndpointNotificationCallback(&client) }.is_ok());
        if !ok {
            log::warn("mixer: sem os avisos de troca da saída de áudio");
        }
        DeviceWatch { en: en.filter(|_| ok), client }
    }
}

impl Drop for DeviceWatch {
    fn drop(&mut self) {
        if let Some(en) = &self.en {
            unsafe {
                let _ = en.UnregisterEndpointNotificationCallback(&self.client);
            }
        }
    }
}

/// `IPolicyConfig` (Windows 7+): não documentada, mas estável desde o Vista;
/// é o que o EarTrumpet e o SoundSwitch usam para trocar a saída padrão. Só o
/// `SetDefaultEndpoint` é usado; os outros métodos estão aqui pela ordem da
/// tabela da interface (os nomes seguem os do Windows).
#[allow(non_snake_case, clippy::too_many_arguments)]
mod policy {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::ERole;
    use windows::core::{HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, id: PCWSTR, format: *mut *mut core::ffi::c_void) -> HRESULT;
        fn GetDeviceFormat(&self, id: PCWSTR, default: i32, format: *mut *mut core::ffi::c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, id: PCWSTR, endpoint: *const core::ffi::c_void, mix: *const core::ffi::c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, id: PCWSTR, default: i32, period: *mut i64, min: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, id: PCWSTR, period: *const i64) -> HRESULT;
        fn GetShareMode(&self, id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        fn SetShareMode(&self, id: PCWSTR, mode: *const core::ffi::c_void) -> HRESULT;
        fn GetPropertyValue(&self, id: PCWSTR, store: i32, key: *const PROPERTYKEY, value: *mut core::ffi::c_void) -> HRESULT;
        fn SetPropertyValue(&self, id: PCWSTR, store: i32, key: *const PROPERTYKEY, value: *const core::ffi::c_void) -> HRESULT;
        fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
    }

    /// Torna a saída `id` a padrão no papel `role`.
    pub fn set_default(pc: &IPolicyConfig, id: PCWSTR, role: ERole) -> HRESULT {
        unsafe { pc.SetDefaultEndpoint(id, role) }
    }
}
use policy::IPolicyConfig;

/// CPolicyConfigClient.
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// Torna a saída `id` a padrão nos três papéis (o que o Windows faz ao
/// escolher a saída no menu do volume).
fn set_default_output(id: &str) -> Result<()> {
    unsafe {
        let pc: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
        let id = HSTRING::from(id);
        for role in [eConsole, eMultimedia, eCommunications] {
            policy::set_default(&pc, PCWSTR(id.as_ptr()), role).ok()?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- sessões

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

/// Uma saída ativa aberta: o gerenciador de sessões dela e o aviso de sessão nova.
struct Dev {
    id: String,
    name: String,
    mgr: IAudioSessionManager2,
    notify: IAudioSessionNotification,
}

/// Mixer aberto: as saídas ativas (qual é a padrão) e as sessões delas.
struct Open {
    default_id: String,
    devs: Vec<Dev>,
    sessions: Vec<Session>,
}

struct Mixer {
    tx: Sender<MixCmd>,
    open: Option<Open>,
    /// Ícones e nomes dos apps já lidos (pela chave do app).
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
            let default_id = en.GetDefaultAudioEndpoint(eRender, eConsole).map(|d| device_id(&d)).unwrap_or_default();
            let col = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let mut devs = Vec::new();
            for i in 0..col.GetCount()? {
                let Ok(dev) = col.Item(i) else { continue };
                let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
                let notify: IAudioSessionNotification = NewSession(self.tx.clone()).into();
                let _ = mgr.RegisterSessionNotification(&notify);
                devs.push(Dev { id: device_id(&dev), name: device_name(&dev), mgr, notify });
            }
            Ok(Open { default_id, devs, sessions: Vec::new() })
        }
    }

    fn close(&mut self) {
        if let Some(o) = self.open.take() {
            unsafe {
                for d in &o.devs {
                    let _ = d.mgr.UnregisterSessionNotification(&d.notify);
                }
                for s in &o.sessions {
                    let _ = s.ctl.UnregisterAudioSessionNotification(&s.events);
                }
            }
        }
    }

    /// Chama `f` com o volume de cada sessão do app `key` (em qualquer saída).
    fn each_volume(&self, key: &str, mut f: impl FnMut(&ISimpleAudioVolume)) {
        if let Some(o) = &self.open {
            for s in o.sessions.iter().filter(|s| s.key == key) {
                f(&s.vol);
            }
        }
    }

    /// Lê as sessões de todas as saídas (e registra os avisos nas novas) e
    /// monta a lista por app.
    fn read(&mut self) -> MixerState {
        let Some(o) = self.open.as_mut() else { return MixerState::default() };
        // (sessão, volume, tocando, sistema, chave, nome dado pela sessão)
        let mut list: Vec<(IAudioSessionControl2, ISimpleAudioVolume, bool, bool, String, String)> = Vec::new();
        unsafe {
            for d in &o.devs {
                let Ok(en) = d.mgr.GetSessionEnumerator() else { continue };
                for i in 0..en.GetCount().unwrap_or(0) {
                    let Ok(ctl) = en.GetSession(i) else { continue };
                    let state = ctl.GetState().unwrap_or(AudioSessionStateExpired);
                    if state == AudioSessionStateExpired {
                        continue;
                    }
                    let (Ok(ctl2), Ok(vol)) = (ctl.cast::<IAudioSessionControl2>(), ctl.cast::<ISimpleAudioVolume>()) else { continue };
                    let system = ctl2.IsSystemSoundsSession().0 == 0;
                    let pid = ctl2.GetProcessId().unwrap_or(0);
                    let key = if system { "system".to_string() } else { app_key(pid) };
                    let shown = if system { String::new() } else { ctl.GetDisplayName().map(|p| take_str(p.0)).unwrap_or_default() };
                    list.push((ctl2, vol, state == AudioSessionStateActive, system, key, shown));
                }
            }
        }
        // Avisos: registra nas sessões novas; esquece as que sumiram.
        let mut kept: Vec<Session> = Vec::new();
        for (ctl, vol, _, _, key, _) in &list {
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
        let device = o.devs.iter().find(|d| d.id == o.default_id).map(|d| d.name.clone()).unwrap_or_default();
        let device_id = o.default_id.clone();
        let devices: Vec<OutDevice> = o.devs.iter().map(|d| OutDevice { id: d.id.clone(), name: d.name.clone() }).collect();

        // Junta por app: volume = o maior; mudo só se todas estiverem mudas.
        let mut apps: Vec<MixApp> = Vec::new();
        for (_, vol, active, system, key, shown) in &list {
            let (v, mute) = unsafe { (vol.GetMasterVolume().unwrap_or(1.0), vol.GetMute().map(|b| b.as_bool()).unwrap_or(false)) };
            match apps.iter_mut().find(|a| a.key == *key) {
                Some(a) => {
                    a.volume = a.volume.max(v);
                    a.muted &= mute;
                    a.active |= *active;
                }
                None => {
                    let (name, icon) = if *system { (String::new(), None) } else { self.describe(key, shown) };
                    apps.push(MixApp { key: key.clone(), name, icon, volume: v, muted: mute, active: *active, system: *system });
                }
            }
        }
        // Como no Windows: os sons do sistema primeiro; depois os que estão
        // tocando e os outros, por nome.
        apps.sort_by_key(|a| (!a.system, !a.active, a.name.to_lowercase()));
        MixerState { device, device_id, devices, apps, fill: Vec::new() }
    }

    /// Nome e ícone do app `key`: o do pacote (pasta "Aplicativos"), o nome que
    /// a própria sessão dá ("RemoteApp Audio") ou a descrição do .exe.
    fn describe(&mut self, key: &str, shown: &str) -> (String, Option<Arc<Image>>) {
        if !self.names.contains_key(key) {
            let (name, icon) = match key.strip_prefix("app:") {
                Some(aumid) => {
                    let (n, i) = appicon::from_apps_folder(aumid);
                    (n.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| aumid.split('!').next().unwrap_or(aumid).to_string()), i.map(Arc::new))
                }
                None => {
                    let name = if !shown.is_empty() && !shown.starts_with('@') { shown.to_string() } else { app_name(key) };
                    (name, appicon::exe_icon(key).map(Arc::new))
                }
            };
            self.names.insert(key.to_string(), name);
            self.icons.insert(key.to_string(), icon);
        }
        (self.names[key].clone(), self.icons.get(key).cloned().flatten())
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        self.close();
    }
}

/// Chave do app de um processo: o pacote dele, se tiver (apps da Store e o
/// WebView2 que eles abrem), ou o caminho do .exe.
fn app_key(pid: u32) -> String {
    if let Some(aumid) = crate::audio::process::package_aumid(pid) {
        return format!("app:{aumid}");
    }
    appicon::exe_path(pid).unwrap_or_else(|| format!("pid:{pid}"))
}

/// Texto de um PWSTR do COM (e libera a memória dele).
fn take_str(p: *mut u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let s = windows::core::PCWSTR(p).to_string().unwrap_or_default();
        CoTaskMemFree(Some(p as _));
        s
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

#[cfg(test)]
mod probe {
    /// Sessões de áudio em TODAS as saídas ativas (para comparar com o Mixer
    /// de Volume do Windows). `cargo test --release mixer_all_devices -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn mixer_all_devices() {
        use windows::Win32::Media::Audio::*;
        use windows::Win32::System::Com::*;
        use windows::core::Interface;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let def = en.GetDefaultAudioEndpoint(eRender, eConsole).map(|d| super::device_id(&d)).unwrap_or_default();
            let comm = en.GetDefaultAudioEndpoint(eRender, eCommunications).map(|d| super::device_id(&d)).unwrap_or_default();
            let col = en.EnumAudioEndpoints(eRender, DEVICE_STATE(DEVICE_STATEMASK_ALL)).unwrap();
            for i in 0..col.GetCount().unwrap() {
                let dev = col.Item(i).unwrap();
                let id = super::device_id(&dev);
                let state = dev.GetState().map(|s| s.0).unwrap_or(0);
                println!("== {} estado_disp={state} {}{}", super::device_name(&dev), if id == def { "[padrão]" } else { "" }, if id == comm { "[comunicação]" } else { "" });
                let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { println!("   (sem sessões)"); continue };
                let Ok(se) = mgr.GetSessionEnumerator() else { println!("   (sem lista)"); continue };
                for k in 0..se.GetCount().unwrap() {
                    let ctl = se.GetSession(k).unwrap();
                    let c2: IAudioSessionControl2 = ctl.cast().unwrap();
                    let pid = c2.GetProcessId().unwrap_or(0);
                    let st = ctl.GetState().map(|s| s.0).unwrap_or(-1);
                    let path = crate::appicon::exe_path(pid).unwrap_or_default();
                    let dn = ctl.GetDisplayName().map(|p| p.to_string().unwrap_or_default()).unwrap_or_default();
                    let ic = ctl.GetIconPath().map(|p| p.to_string().unwrap_or_default()).unwrap_or_default();
                    let sid = c2.GetSessionIdentifier().map(|p| p.to_string().unwrap_or_default()).unwrap_or_default();
                    let pkg = crate::audio::process::package_aumid(pid).unwrap_or_default();
                    println!("   pid={pid:<6} estado={st} sistema={} {} | nome='{dn}' ícone='{ic}' pacote='{pkg}'", c2.IsSystemSoundsSession().0 == 0, path);
                    println!("        id={sid}");
                }
            }
        }
    }
}

