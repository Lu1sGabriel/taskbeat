//! Letras guardadas em disco (uma por arquivo) e a pasta de letras do usuário.
//!
//! Cache: `%LOCALAPPDATA%\Taskbeat\lyrics-cache` (Windows),
//! `~/Library/Caches/Taskbeat/lyrics` (Mac), `~/.cache/taskbeat/lyrics` (Linux).
//! Guarda também "sem letra", que vale por 7 dias (o LRCLIB recebe letras
//! novas). Passando de `MAX_FILES` arquivos, os mais antigos saem.
//!
//! Pasta do usuário: arquivos `Artista - Música.lrc` (ou `Música.lrc`, ou
//! `.txt` com o texto simples) que valem mais que a busca.

use super::{Line, Lyrics, Track, lrclib, norm, parse_lrc, parse_plain};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Quanto tempo um "sem letra" vale antes de procurar de novo.
const NOT_FOUND_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_FILES: usize = 3000;
const HEADER: &str = "#taskbeat-lyrics 1";

pub fn cache_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Taskbeat").join("lyrics-cache"))
    }
    #[cfg(target_os = "macos")]
    {
        Some(PathBuf::from(std::env::var_os("HOME")?).join("Library").join("Caches").join("Taskbeat").join("lyrics"))
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".cache")))?;
        Some(base.join("taskbeat").join("lyrics"))
    }
}

/// Pasta onde o usuário pode pôr as próprias letras (.lrc / .txt).
pub fn user_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Taskbeat").join("lyrics"))
    }
    #[cfg(target_os = "macos")]
    {
        Some(PathBuf::from(std::env::var_os("HOME")?).join("Library").join("Application Support").join("Taskbeat").join("lyrics"))
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local").join("share")))?;
        Some(base.join("taskbeat").join("lyrics"))
    }
}

/// FNV-1a de 64 bits (nome do arquivo de cada música).
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `[mm:ss.xx]` de um tempo em segundos.
fn stamp(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!("[{:02}:{:02}.{:02}]", cs / 6000, (cs / 100) % 60, cs % 100)
}

/// Texto do arquivo do cache para uma letra (`None`: não se guarda, ex.: falhou).
pub fn encode(l: &Lyrics, key: &str, now: u64) -> Option<String> {
    let (kind, body) = match l {
        Lyrics::Synced(lines) => ("synced", lines.iter().map(|x| format!("{}{}", stamp(x.t), x.text)).collect::<Vec<_>>().join("\n")),
        Lyrics::Plain(lines) => ("plain", lines.join("\n")),
        Lyrics::Instrumental => ("instrumental", String::new()),
        Lyrics::NotFound => ("none", String::new()),
        _ => return None,
    };
    Some(format!("{HEADER}\n#kind {kind}\n#time {now}\n#key {}\n{body}\n", key.replace('\n', " | ")))
}

/// Lê um arquivo do cache. `None` se não for nosso ou se o "sem letra" venceu.
pub fn decode(text: &str, now: u64) -> Option<Lyrics> {
    let mut lines = text.lines();
    if lines.next()? != HEADER {
        return None;
    }
    let (mut kind, mut time) = ("", 0u64);
    let mut body = Vec::new();
    for l in lines {
        if body.is_empty() {
            if let Some(v) = l.strip_prefix("#kind ") {
                kind = v.trim();
                continue;
            }
            if let Some(v) = l.strip_prefix("#time ") {
                time = v.trim().parse().unwrap_or(0);
                continue;
            }
            if l.starts_with("#key ") {
                continue;
            }
        }
        body.push(l);
    }
    let body = body.join("\n");
    match kind {
        "synced" => Some(Lyrics::Synced(parse_lrc(&body))),
        "plain" => Some(Lyrics::Plain(parse_plain(&body))),
        "instrumental" => Some(Lyrics::Instrumental),
        "none" => (now.saturating_sub(time) < NOT_FOUND_TTL.as_secs()).then_some(Lyrics::NotFound),
        _ => None,
    }
}

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{:016x}.lrc", fnv(key)))
    }

    pub fn get(&self, key: &str) -> Option<Lyrics> {
        let text = std::fs::read_to_string(self.file(key)).ok()?;
        decode(&text, unix_now())
    }

    pub fn put(&self, key: &str, l: &Lyrics) {
        let Some(text) = encode(l, key, unix_now()) else { return };
        // Atômico: uma gravação interrompida não deixa um `.lrc` pela metade.
        let _ = crate::files::write_atomic(&self.file(key), text.as_bytes());
    }

    /// Apaga tudo; devolve quantos arquivos saíram.
    pub fn clear(&self) -> usize {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return 0 };
        rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "lrc")).filter(|e| std::fs::remove_file(e.path()).is_ok()).count()
    }

    /// Com mais de `MAX_FILES` arquivos, apaga os mais antigos (fica com 80%).
    pub fn prune(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<(SystemTime, PathBuf)> =
            rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).filter(|(_, p)| p.extension().is_some_and(|x| x == "lrc")).collect();
        if files.len() <= MAX_FILES {
            return;
        }
        files.sort();
        let extra = files.len() - MAX_FILES * 4 / 5;
        for (_, p) in files.into_iter().take(extra) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Nome de arquivo comparável: minúsculas, sem pontuação nem espaços repetidos.
fn file_norm(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    norm(&s)
}

/// Letra da pasta do usuário para a música, se tiver: `Artista - Música.lrc`,
/// `Música.lrc` (ou `.txt`, texto simples). O nome é comparado sem caixa nem
/// pontuação, com e sem os enfeites de vídeo no título.
pub fn local(dir: &Path, t: &Track) -> Option<Lyrics> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut names = Vec::new();
    for (a, b) in std::iter::once((t.artist.clone(), t.title.clone())).chain(lrclib::candidates(t)) {
        names.push(file_norm(&format!("{a} - {b}")));
        names.push(file_norm(&b));
    }
    let mut best: Option<(usize, PathBuf)> = None;
    for e in rd.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("lrc" | "txt")) {
            continue;
        }
        let stem = file_norm(p.file_stem().and_then(|x| x.to_str()).unwrap_or_default());
        // Quanto mais cedo na lista, melhor ("Artista - Música" vence "Música").
        if let Some(rank) = names.iter().position(|n| !n.is_empty() && *n == stem) {
            let rank = rank * 2 + usize::from(ext.as_deref() == Some("txt"));
            if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                best = Some((rank, p));
            }
        }
    }
    let (_, p) = best?;
    let bytes = std::fs::read(&p).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<Line> = parse_lrc(&text);
    if lines.iter().any(|l| !l.text.is_empty()) {
        return Some(Lyrics::Synced(lines));
    }
    let plain = parse_plain(&text);
    (!plain.is_empty()).then_some(Lyrics::Plain(plain))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_e_volta_no_arquivo() {
        let key = "m83\nmidnight city\n";
        let l = Lyrics::Synced(parse_lrc("[00:01.50]Um\n[01:02.25]Dois\n[01:10.00]"));
        let text = encode(&l, key, 100).unwrap();
        assert!(text.contains("[01:02.25]Dois"));
        assert_eq!(decode(&text, 100), Some(l));
        let p = Lyrics::Plain(vec!["a".into(), "".into(), "b".into()]);
        assert_eq!(decode(&encode(&p, key, 100).unwrap(), 100), Some(p));
        assert_eq!(decode(&encode(&Lyrics::Instrumental, key, 100).unwrap(), 100), Some(Lyrics::Instrumental));
        // "Sem letra" vence em 7 dias; o que falhou não é guardado.
        let none = encode(&Lyrics::NotFound, key, 100).unwrap();
        assert_eq!(decode(&none, 100 + 3600), Some(Lyrics::NotFound));
        assert_eq!(decode(&none, 100 + 8 * 24 * 3600), None);
        assert!(encode(&Lyrics::Failed, key, 100).is_none());
        assert!(decode("outra coisa", 0).is_none());
    }

    #[test]
    fn grava_le_e_limpa() {
        let dir = std::env::temp_dir().join(format!("taskbeat-lyrics-test-{}", std::process::id()));
        let c = Cache::new(dir.clone());
        let key = Track { artist: "A".into(), title: "B".into(), ..Default::default() }.key();
        assert_eq!(c.get(&key), None);
        c.put(&key, &Lyrics::Plain(vec!["x".into()]));
        assert_eq!(c.get(&key), Some(Lyrics::Plain(vec!["x".into()])));
        assert_eq!(c.clear(), 1);
        assert_eq!(c.get(&key), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pasta_do_usuario() {
        let dir = std::env::temp_dir().join(format!("taskbeat-lyrics-local-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Coldplay - Yellow.lrc"), "[00:01.00]Look at the stars").unwrap();
        std::fs::write(dir.join("Yellow.txt"), "Texto").unwrap();
        std::fs::write(dir.join("Outra.lrc"), "[00:01.00]x").unwrap();
        let t = Track { artist: "ColdplayVEVO".into(), title: "Coldplay - Yellow (Official Video)".into(), ..Default::default() };
        assert!(matches!(local(&dir, &t), Some(Lyrics::Synced(ref l)) if l[0].text == "Look at the stars"));
        let t2 = Track { artist: "Outro artista".into(), title: "yellow".into(), ..Default::default() };
        assert_eq!(local(&dir, &t2), Some(Lyrics::Plain(vec!["Texto".into()])));
        let t3 = Track { artist: "X".into(), title: "Nada".into(), ..Default::default() };
        assert_eq!(local(&dir, &t3), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
