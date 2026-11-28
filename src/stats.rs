//! Medição de quadros e despertares (PR-00), a base de comparação dos PRs
//! `perf`. Liga com `TASKBEAT_FRAMESTATS=1`.
//!
//! - **Desligada**, cada chamada custa a leitura de um `OnceLock<bool>` (uma
//!   carga atômica) e um `if`: nada de relógio, trava nem alocação.
//! - **Ligada**, guarda o tempo de desenho e de apresentação de cada quadro por
//!   superfície e conta os despertares por origem. A cada 60 s acrescenta um
//!   bloco em `stats.log`, na pasta da config, com p50/p95/p99 e os
//!   despertares por segundo (e, no Windows, os handles do kernel, GDI e USER).
//!
//! Quem chama mede o próprio tempo (`Instant::now()` só quando `enabled()`).

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Período de cada bloco do `stats.log`.
const PERIOD: Duration = Duration::from_secs(60);
/// Passou disso, o `stats.log` vira `stats.old.log`.
const MAX_LOG: u64 = 1 << 20;

/// Medição ligada (`TASKBEAT_FRAMESTATS=1`)?
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TASKBEAT_FRAMESTATS").is_some_and(|v| v == "1"))
}

/// Um quadro de `surface`: quanto levou o desenho e a apresentação.
pub fn frame(surface: &'static str, draw: Duration, present: Duration) {
    if enabled() {
        record(|s| s.frame(surface, draw, present));
    }
}

/// Uma thread acordou sozinha (timer, polling, evento do sistema).
pub fn wake(source: &'static str) {
    if enabled() {
        record(|s| s.wake(source));
    }
}

fn record(f: impl FnOnce(&mut FrameStats)) {
    static STATS: Mutex<Option<FrameStats>> = Mutex::new(None);
    let now = Instant::now();
    let report = {
        let mut g = STATS.lock().unwrap_or_else(|e| e.into_inner());
        let s = g.get_or_insert_with(|| FrameStats::new(now));
        f(s);
        (now.saturating_duration_since(s.since) >= PERIOD).then(|| s.report(now))
    };
    // Grava fora da trava: as outras threads não esperam o disco.
    if let Some(text) = report {
        append(&text);
    }
}

fn append(text: &str) {
    use std::io::Write;
    let Some(dir) = crate::config::Config::path().and_then(|p| p.parent().map(|d| d.to_path_buf())) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("stats.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG) {
        let _ = std::fs::rename(&path, dir.join("stats.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(text.as_bytes());
    }
}

/// Os números de um período.
pub struct FrameStats {
    /// Por superfície: tempos de desenho e de apresentação, em µs.
    surfaces: Vec<(&'static str, Vec<u32>, Vec<u32>)>,
    wakes: Vec<(&'static str, u64)>,
    since: Instant,
}

impl FrameStats {
    pub fn new(now: Instant) -> Self {
        Self { surfaces: Vec::new(), wakes: Vec::new(), since: now }
    }

    pub fn frame(&mut self, surface: &'static str, draw: Duration, present: Duration) {
        let us = |d: Duration| d.as_micros().min(u32::MAX as u128) as u32;
        match self.surfaces.iter_mut().find(|e| e.0 == surface) {
            Some(e) => {
                e.1.push(us(draw));
                e.2.push(us(present));
            }
            None => self.surfaces.push((surface, vec![us(draw)], vec![us(present)])),
        }
    }

    pub fn wake(&mut self, source: &'static str) {
        match self.wakes.iter_mut().find(|e| e.0 == source) {
            Some(e) => e.1 += 1,
            None => self.wakes.push((source, 1)),
        }
    }

    /// O bloco do período que termina em `now`, e zera para o próximo.
    pub fn report(&mut self, now: Instant) -> String {
        use std::fmt::Write;
        let secs = now.saturating_duration_since(self.since).as_secs_f64().max(0.001);
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut out = format!("t={unix} periodo={secs:.0}s\n");
        for (name, draw, present) in &mut self.surfaces {
            draw.sort_unstable();
            present.sort_unstable();
            let _ = writeln!(
                out,
                "  quadros {name}: {} ({:.1}/s) desenho p50/p95/p99 {}/{}/{} µs, apresentação {}/{}/{} µs",
                draw.len(),
                draw.len() as f64 / secs,
                pct(draw, 0.50),
                pct(draw, 0.95),
                pct(draw, 0.99),
                pct(present, 0.50),
                pct(present, 0.95),
                pct(present, 0.99),
            );
        }
        for (name, n) in &self.wakes {
            let _ = writeln!(out, "  despertares {name}: {n} ({:.2}/s)", *n as f64 / secs);
        }
        if let Some(p) = process() {
            let _ = writeln!(out, "  processo: {p}");
        }
        self.surfaces.clear();
        self.wakes.clear();
        self.since = now;
        out
    }
}

/// Percentil `p` (0..1) de uma lista já ordenada (0 se vazia).
fn pct(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// Handles do kernel, GDI e USER do próprio processo (o working set fica no
/// `tools/measure.ps1`, que não pede feature nova no `windows`).
#[cfg(windows)]
fn process() -> Option<String> {
    use windows::Win32::System::Threading::{GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources, GetProcessHandleCount};
    // SAFETY: `GetCurrentProcess` devolve um pseudo-handle que não precisa ser
    // fechado; as três funções só leem contadores do próprio processo.
    unsafe {
        let me = GetCurrentProcess();
        let mut handles = 0u32;
        let handles = GetProcessHandleCount(me, &mut handles).map(|_| handles.to_string()).unwrap_or_else(|_| "?".into());
        // 0 é ambíguo (erro ou nenhum objeto), mas o processo sempre tem janelas.
        let gdi = GetGuiResources(me, GR_GDIOBJECTS);
        let user = GetGuiResources(me, GR_USEROBJECTS);
        Some(format!("handles {handles}, GDI {gdi}, USER {user}"))
    }
}

#[cfg(not(windows))]
fn process() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentis_e_bloco() {
        let t0 = Instant::now();
        let mut s = FrameStats::new(t0);
        for i in 1..=100u64 {
            s.frame("widget", Duration::from_micros(i), Duration::from_micros(2 * i));
        }
        s.frame("card", Duration::from_millis(5), Duration::ZERO);
        s.wake("audio");
        s.wake("audio");
        let r = s.report(t0 + Duration::from_secs(10));
        assert!(r.contains("quadros widget: 100 (10.0/s) desenho p50/p95/p99 51/95/99 µs, apresentação 102/190/198 µs"), "{r}");
        assert!(r.contains("quadros card: 1"), "{r}");
        assert!(r.contains("despertares audio: 2 (0.20/s)"), "{r}");
        // Zerou para o próximo período.
        assert!(s.surfaces.is_empty() && s.wakes.is_empty());
        assert_eq!(pct(&[], 0.5), 0);
        assert_eq!(pct(&[7], 0.99), 7);
    }

    /// Desligada (o padrão dos testes): as chamadas não fazem nada.
    #[test]
    fn desligada_nao_faz_nada() {
        if enabled() {
            return;
        }
        frame("widget", Duration::from_millis(1), Duration::from_millis(1));
        wake("audio");
    }
}
