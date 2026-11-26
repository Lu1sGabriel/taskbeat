//! Som para as barras e volume da saída, pelo PulseAudio (o PipeWire atende
//! a mesma interface com o `pipewire-pulse`, padrão nas distros atuais).
//!
//! - Barras: grava o "monitor" da saída padrão (o que está tocando), em mono.
//!   Com "só o player", grava só o fluxo do app que está tocando (o mesmo
//!   recurso que o pavucontrol usa para os medidores de cada app); se não
//!   achar o fluxo dele, volta para a saída inteira. Não pede permissão.
//! - Só grava com as barras visíveis (`Shared::wants_audio`): parado, nada roda.
//! - Volume e mudo da saída padrão (o mesmo do painel do sistema).
//!
//! Usa o laço "threaded" da libpulse: ela tem a própria thread; as outras
//! travam o laço para mexer no contexto.

use super::system::log;
use crate::shared::Shared;
use libpulse_sys::*;
use std::ffi::{CStr, CString, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const RATE: u32 = 48000;
/// Verifica se precisa gravar (barras visíveis, player mudou) a cada…
const CHECK_US: u64 = 400_000;

/// Volume da saída padrão (0..1, 1 = 100%) e mudo, atualizados pelos avisos do servidor.
static VOL_LEVEL: AtomicU32 = AtomicU32::new(0);
static VOL_MUTED: AtomicBool = AtomicBool::new(false);
static VOL_KNOWN: AtomicBool = AtomicBool::new(false);

/// Processo e nomes do player atual (do MPRIS), para achar o fluxo de áudio dele.
static PLAYER: Mutex<Option<PlayerHint>> = Mutex::new(None);

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlayerHint {
    pub pid: u32,
    /// Nomes em minúsculas (identidade, arquivo .desktop, fim do nome no D-Bus).
    pub names: Vec<String>,
}

/// O player que está tocando agora (chamado pela thread de mídia).
pub fn set_player(h: Option<PlayerHint>) {
    *PLAYER.lock().unwrap() = h;
}

/// Volume atual: (nível 0..1, mudo), se já lido.
pub fn volume() -> Option<(f32, bool)> {
    VOL_KNOWN.load(Ordering::Relaxed).then(|| (f32::from_bits(VOL_LEVEL.load(Ordering::Relaxed)), VOL_MUTED.load(Ordering::Relaxed)))
}

/// Muda o volume da saída padrão (0..1).
pub fn set_volume(v: f32) {
    let v = v.clamp(0.0, 1.0);
    VOL_LEVEL.store(v.to_bits(), Ordering::Relaxed);
    with_pulse(|p| unsafe {
        let mut cv: pa_cvolume = std::mem::zeroed();
        pa_cvolume_set(&mut cv, p.channels.max(1) as u32, (v * PA_VOLUME_NORM as f32).round() as u32);
        let op = pa_context_set_sink_volume_by_name(p.ctx, c"@DEFAULT_SINK@".as_ptr(), &cv, None, null_mut());
        unref(op);
        // Mexer no volume tira o mudo (como os painéis do sistema).
        if VOL_MUTED.swap(false, Ordering::Relaxed) {
            unref(pa_context_set_sink_mute_by_name(p.ctx, c"@DEFAULT_SINK@".as_ptr(), 0, None, null_mut()));
        }
    });
}

/// Liga/desliga o mudo da saída padrão.
pub fn set_muted(m: bool) {
    VOL_MUTED.store(m, Ordering::Relaxed);
    with_pulse(|p| unsafe {
        unref(pa_context_set_sink_mute_by_name(p.ctx, c"@DEFAULT_SINK@".as_ptr(), m as i32, None, null_mut()));
    });
}

/// Lê o volume de novo (o card abriu).
pub fn refresh_volume() {
    with_pulse(|p| unsafe { query_sink(p) });
}

// ---------------------------------------------------------------- estado da libpulse

/// O que gravar.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// Tudo que toca na saída padrão.
    Monitor,
    /// Só o fluxo `idx` (do player), pelo monitor da saída `source`.
    Stream { idx: u32, source: String },
}

struct Pulse {
    ml: *mut pa_threaded_mainloop,
    api: *const pa_mainloop_api,
    ctx: *mut pa_context,
    stream: *mut pa_stream,
    target: Option<Target>,
    shared: Arc<Shared>,
    channels: u8,
    /// Fluxos de áudio dos apps (índice, saída, pid, nomes), da última leitura.
    inputs: Vec<(u32, u32, u32, String)>,
    inputs_fresh: bool,
    /// Monitores das saídas (índice da saída → nome do monitor).
    monitors: Vec<(u32, String)>,
    last_player: Option<PlayerHint>,
    ready: bool,
    /// O contexto caiu: reconectar na próxima verificação.
    ctx_retry: bool,
}

/// Ponteiro do estado (vive o app inteiro); só usado com o laço travado.
struct Handle(*mut Pulse);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

static PULSE: OnceLock<Handle> = OnceLock::new();

/// Roda `f` com o laço da libpulse travado (de qualquer thread).
fn with_pulse(f: impl FnOnce(&mut Pulse)) {
    let Some(h) = PULSE.get() else { return };
    unsafe {
        let p = &mut *h.0;
        pa_threaded_mainloop_lock(p.ml);
        if p.ready {
            f(p);
        }
        pa_threaded_mainloop_unlock(p.ml);
    }
}

fn unref(op: *mut pa_operation) {
    if !op.is_null() {
        unsafe { pa_operation_unref(op) };
    }
}

pub fn spawn(shared: Arc<Shared>) {
    shared.sample_rate.store(RATE, Ordering::Relaxed);
    unsafe {
        let ml = pa_threaded_mainloop_new();
        if ml.is_null() {
            log::warn("áudio: libpulse indisponível");
            return;
        }
        let api = pa_threaded_mainloop_get_api(ml);
        let p = Box::into_raw(Box::new(Pulse {
            ml,
            api,
            ctx: null_mut(),
            stream: null_mut(),
            target: None,
            shared,
            channels: 2,
            inputs: Vec::new(),
            inputs_fresh: false,
            monitors: Vec::new(),
            last_player: None,
            ready: false,
            ctx_retry: false,
        }));
        let _ = PULSE.set(Handle(p));
        pa_threaded_mainloop_set_name(ml, c"audio-pulse".as_ptr());
        pa_threaded_mainloop_lock(ml);
        connect(&mut *p);
        // Verificação periódica (barras ligadas? player mudou?).
        schedule_check(&mut *p);
        pa_threaded_mainloop_start(ml);
        pa_threaded_mainloop_unlock(ml);
    }
}

unsafe fn connect(p: &mut Pulse) {
    unsafe {
        if !p.ctx.is_null() {
            pa_context_set_state_callback(p.ctx, None, null_mut());
            pa_context_set_subscribe_callback(p.ctx, None, null_mut());
            pa_context_disconnect(p.ctx);
            pa_context_unref(p.ctx);
        }
        p.ready = false;
        p.stream = null_mut();
        p.target = None;
        let props = pa_proplist_new();
        pa_proplist_sets(props, c"application.name".as_ptr(), c"Taskbeat".as_ptr());
        pa_proplist_sets(props, c"application.id".as_ptr(), c"io.github.lu1sgabriel.taskbeat".as_ptr());
        pa_proplist_sets(props, c"application.icon_name".as_ptr(), c"taskbeat".as_ptr());
        p.ctx = pa_context_new_with_proplist(p.api, c"Taskbeat".as_ptr(), props);
        pa_proplist_free(props);
        if p.ctx.is_null() {
            return;
        }
        pa_context_set_state_callback(p.ctx, Some(on_context_state), (p as *mut Pulse).cast());
        if pa_context_connect(p.ctx, std::ptr::null(), PA_CONTEXT_NOFAIL, std::ptr::null()) < 0 {
            log::warn("áudio: não conectou ao servidor de som");
        }
    }
}

extern "C" fn on_context_state(c: *mut pa_context, ud: *mut c_void) {
    let p = unsafe { &mut *(ud as *mut Pulse) };
    match unsafe { pa_context_get_state(c) } {
        PA_CONTEXT_READY => unsafe {
            p.ready = true;
            log::info("áudio: conectado ao servidor de som");
            pa_context_set_subscribe_callback(c, Some(on_event), ud);
            unref(pa_context_subscribe(c, PA_SUBSCRIPTION_MASK_SINK | PA_SUBSCRIPTION_MASK_SINK_INPUT | PA_SUBSCRIPTION_MASK_SERVER, None, null_mut()));
            query_sink(p);
            query_inputs(p);
        },
        PA_CONTEXT_FAILED | PA_CONTEXT_TERMINATED => {
            p.ready = false;
            p.stream = null_mut();
            p.target = None;
            // Tenta de novo depois (o servidor reiniciou ou ainda não subiu).
            p.ctx_retry = true;
        }
        _ => {}
    }
}

extern "C" fn on_event(_c: *mut pa_context, t: pa_subscription_event_type_t, _idx: u32, ud: *mut c_void) {
    let p = unsafe { &mut *(ud as *mut Pulse) };
    match t & PA_SUBSCRIPTION_EVENT_FACILITY_MASK {
        PA_SUBSCRIPTION_EVENT_SINK | PA_SUBSCRIPTION_EVENT_SERVER => unsafe { query_sink(p) },
        PA_SUBSCRIPTION_EVENT_SINK_INPUT => {
            p.inputs_fresh = false;
            if p.target.is_some() {
                unsafe { query_inputs(p) };
            }
        }
        _ => {}
    }
}

/// Lê volume/mudo da saída padrão.
unsafe fn query_sink(p: &mut Pulse) {
    unsafe {
        unref(pa_context_get_sink_info_by_name(p.ctx, c"@DEFAULT_SINK@".as_ptr(), Some(on_sink), (p as *mut Pulse).cast()));
    }
}

extern "C" fn on_sink(_c: *mut pa_context, i: *const pa_sink_info, eol: i32, ud: *mut c_void) {
    if eol != 0 || i.is_null() {
        return;
    }
    let p = unsafe { &mut *(ud as *mut Pulse) };
    let i = unsafe { &*i };
    let avg = unsafe { pa_cvolume_avg(&i.volume) };
    p.channels = i.volume.channels.max(1);
    VOL_LEVEL.store((avg as f32 / PA_VOLUME_NORM as f32).min(1.0).to_bits(), Ordering::Relaxed);
    VOL_MUTED.store(i.mute != 0, Ordering::Relaxed);
    VOL_KNOWN.store(true, Ordering::Relaxed);
}

/// Lê os fluxos dos apps e os monitores das saídas (para "só o player").
unsafe fn query_inputs(p: &mut Pulse) {
    unsafe {
        // Monitores antes: as respostas chegam na ordem dos pedidos e o fim da
        // lista de fluxos já decide o que gravar.
        unref(pa_context_get_sink_info_list(p.ctx, Some(on_monitor), (p as *mut Pulse).cast()));
        p.inputs.clear();
        unref(pa_context_get_sink_input_info_list(p.ctx, Some(on_input), (p as *mut Pulse).cast()));
    }
}

fn prop(pl: *mut pa_proplist, key: &CStr) -> String {
    if pl.is_null() {
        return String::new();
    }
    let v = unsafe { pa_proplist_gets(pl, key.as_ptr()) };
    if v.is_null() { String::new() } else { unsafe { CStr::from_ptr(v) }.to_string_lossy().into_owned() }
}

extern "C" fn on_input(_c: *mut pa_context, i: *const pa_sink_input_info, eol: i32, ud: *mut c_void) {
    let p = unsafe { &mut *(ud as *mut Pulse) };
    if eol != 0 || i.is_null() {
        p.inputs_fresh = true;
        unsafe { update(p) };
        return;
    }
    let i = unsafe { &*i };
    // Os nossos próprios fluxos e os de eventos do sistema não contam.
    let role = prop(i.proplist, c"media.role");
    if role == "event" || prop(i.proplist, c"application.id") == "io.github.lu1sgabriel.taskbeat" {
        return;
    }
    let pid = prop(i.proplist, c"application.process.id").parse().unwrap_or(0);
    let names = [prop(i.proplist, c"application.name"), prop(i.proplist, c"application.process.binary"), prop(i.proplist, c"application.icon_name"), prop(i.proplist, c"pipewire.access.portal.app_id")]
        .join("\n")
        .to_lowercase();
    // Quem está tocando primeiro.
    let entry = (i.index, i.sink, pid, names);
    if i.corked == 0 { p.inputs.insert(0, entry) } else { p.inputs.push(entry) }
}

extern "C" fn on_monitor(_c: *mut pa_context, i: *const pa_sink_info, eol: i32, ud: *mut c_void) {
    let p = unsafe { &mut *(ud as *mut Pulse) };
    if eol != 0 || i.is_null() {
        return;
    }
    let i = unsafe { &*i };
    if i.monitor_source_name.is_null() {
        return;
    }
    let name = unsafe { CStr::from_ptr(i.monitor_source_name) }.to_string_lossy().into_owned();
    p.monitors.retain(|(s, _)| *s != i.index);
    p.monitors.push((i.index, name));
}

fn schedule_check(p: &mut Pulse) {
    unsafe {
        let mut tv: libc::timeval = std::mem::zeroed();
        pa_gettimeofday(&mut tv);
        pa_timeval_add(&mut tv, CHECK_US);
        if let Some(new) = (*p.api).time_new {
            new(p.api, &tv, Some(on_check), (p as *mut Pulse).cast());
        }
    }
}

extern "C" fn on_check(a: *const pa_mainloop_api, e: *mut pa_time_event, _tv: *const libc::timeval, ud: *mut c_void) {
    let p = unsafe { &mut *(ud as *mut Pulse) };
    unsafe {
        if let Some(free) = (*a).time_free {
            free(e);
        }
        if p.ctx_retry {
            p.ctx_retry = false;
            connect(p);
        } else if p.ready {
            let player = PLAYER.lock().unwrap().clone();
            if player != p.last_player {
                p.last_player = player;
                p.inputs_fresh = false;
            }
            if p.shared.wants_audio() && p.shared.player_only() && !p.inputs_fresh {
                query_inputs(p);
            } else {
                update(p);
            }
        }
    }
    if p.shared.is_running() {
        schedule_check(p);
    }
}

/// O processo `pid` é o `ancestor` ou descende dele (o áudio de navegadores
/// sai de um processo filho do que publica o MPRIS)?
fn descends_from(mut pid: u32, ancestor: u32) -> bool {
    for _ in 0..8 {
        if pid == ancestor {
            return true;
        }
        if pid <= 1 {
            return false;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return false };
        // "pid (nome) estado ppid …": o nome pode ter espaços e parênteses.
        let Some(rest) = stat.rsplit_once(')').map(|r| r.1) else { return false };
        match rest.split_whitespace().nth(1).and_then(|v| v.parse().ok()) {
            Some(pp) => pid = pp,
            None => return false,
        }
    }
    false
}

/// O que gravar agora.
fn desired(p: &Pulse) -> Option<Target> {
    // Só com as barras visíveis e música tocando (pausado não há o que mostrar).
    if !p.shared.wants_audio() || !p.shared.media.lock().unwrap().playing {
        return None;
    }
    if p.shared.player_only() {
        if let Some(pl) = &p.last_player {
            let hit = p.inputs.iter().find(|(_, _, pid, names)| {
                (pl.pid != 0 && *pid != 0 && descends_from(*pid, pl.pid)) || pl.names.iter().any(|n| n.len() >= 3 && names.lines().any(|l| l == n || l.starts_with(n.as_str())))
            });
            if let Some((idx, sink, _, _)) = hit {
                if let Some((_, source)) = p.monitors.iter().find(|(s, _)| s == sink) {
                    return Some(Target::Stream { idx: *idx, source: source.clone() });
                }
            }
        }
    }
    Some(Target::Monitor)
}

/// Liga, troca ou desliga a gravação conforme o necessário.
unsafe fn update(p: &mut Pulse) {
    let want = desired(p);
    if want == p.target && (want.is_none() || !p.stream.is_null()) {
        return;
    }
    unsafe {
        if !p.stream.is_null() {
            pa_stream_set_read_callback(p.stream, None, null_mut());
            pa_stream_set_state_callback(p.stream, None, null_mut());
            pa_stream_disconnect(p.stream);
            pa_stream_unref(p.stream);
            p.stream = null_mut();
        }
        p.target = want.clone();
        let Some(t) = want else {
            log::info("áudio: gravação parada");
            return;
        };
        let spec = pa_sample_spec { format: PA_SAMPLE_FLOAT32LE, rate: RATE, channels: 1 };
        let props = pa_proplist_new();
        pa_proplist_sets(props, c"application.id".as_ptr(), c"io.github.lu1sgabriel.taskbeat".as_ptr());
        pa_proplist_sets(props, c"media.role".as_ptr(), c"music".as_ptr());
        let s = pa_stream_new_with_proplist(p.ctx, c"Barras de áudio".as_ptr(), &spec, std::ptr::null(), props);
        pa_proplist_free(props);
        if s.is_null() {
            return;
        }
        let dev = match &t {
            Target::Monitor => CString::from(c"@DEFAULT_MONITOR@"),
            Target::Stream { idx, source } => {
                pa_stream_set_monitor_stream(s, *idx);
                CString::new(source.as_str()).unwrap_or_default()
            }
        };
        // ~21 ms por bloco: mais que um quadro (30/s) de som a cada vez, sem acordar à toa.
        let attr = pa_buffer_attr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: 1024 * 4 };
        pa_stream_set_read_callback(s, Some(on_read), (p as *mut Pulse).cast());
        pa_stream_set_state_callback(s, Some(on_stream_state), (p as *mut Pulse).cast());
        if pa_stream_connect_record(s, dev.as_ptr(), &attr, PA_STREAM_ADJUST_LATENCY | PA_STREAM_DONT_INHIBIT_AUTO_SUSPEND) < 0 {
            pa_stream_unref(s);
            log::warn("áudio: não deu para gravar");
            return;
        }
        p.stream = s;
        log::info(&format!("áudio: gravando {t:?}"));
    }
}

extern "C" fn on_stream_state(s: *mut pa_stream, ud: *mut c_void) {
    let p = unsafe { &mut *(ud as *mut Pulse) };
    if matches!(unsafe { pa_stream_get_state(s) }, PA_STREAM_FAILED | PA_STREAM_TERMINATED) && s == p.stream {
        // O fluxo do player acabou (ou a saída sumiu): escolhe de novo.
        unsafe {
            pa_stream_set_read_callback(s, None, null_mut());
            pa_stream_set_state_callback(s, None, null_mut());
            pa_stream_unref(s);
        }
        p.stream = null_mut();
        p.target = None;
        p.inputs_fresh = false;
    }
}

extern "C" fn on_read(s: *mut pa_stream, _n: usize, ud: *mut c_void) {
    let p = unsafe { &mut *(ud as *mut Pulse) };
    loop {
        let mut data: *const c_void = std::ptr::null();
        let mut n: usize = 0;
        if unsafe { pa_stream_peek(s, &mut data, &mut n) } < 0 || n == 0 {
            return;
        }
        if !data.is_null() {
            let samples = unsafe { std::slice::from_raw_parts(data as *const f32, n / 4) };
            let mut ring = p.shared.audio.lock().unwrap();
            for &v in samples {
                ring.push(if v.is_finite() { v } else { 0.0 });
            }
        }
        unsafe { pa_stream_drop(s) };
    }
}
