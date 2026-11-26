//! Atualização pelas releases do GitHub (`Lu1sGabriel/taskbeat`).
//!
//! Ao abrir e uma vez por dia, uma thread pergunta ao GitHub pela última
//! release (`/releases/latest`, que ignora as versões de teste; com betas, a
//! lista) e compara a versão (semver) com a deste executável.
//!
//! **Segurança.** Uma conta do GitHub invadida não pode virar um canal para
//! código malicioso: o Windows só instala um `Taskbeat-Setup-X.exe` cujo
//! `.sig` é uma assinatura ECDSA P-256 válida, conferida pela CNG do Windows
//! com a chave pública embutida no app (`assets/update-public-key.txt`). A
//! chave privada fica só com o mantenedor, fora do repositório, e assina a
//! mensagem de [`signed_message`]: nome do arquivo, versão e SHA-256 do exe.
//! Com a versão dentro da assinatura, um exe antigo assinado não pode ser
//! republicado como "versão nova". O pop-up só aparece depois que o exe foi
//! baixado e a assinatura conferida: sem a chave, ninguém faz o aviso surgir.
//!
//! No Mac, por enquanto, o aviso só abre a página da release (sem conta de
//! desenvolvedor da Apple, trocar o `.app` sozinho esbarraria no Gatekeeper).
//!
//! A consulta mostra o IP ao GitHub (dito nas configurações); nada mais sai.

#[cfg(windows)]
pub(crate) mod install;
#[cfg(windows)]
pub(crate) mod popup;
#[cfg(windows)]
pub(crate) mod sig;
#[cfg(windows)]
mod win;

use crate::shared::Shared;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtOrd};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const API: &str = "https://api.github.com/repos/Lu1sGabriel/taskbeat";
/// Onde ficam as páginas e os arquivos das releases.
const RELEASES: &str = "https://github.com/Lu1sGabriel/taskbeat/releases/";
/// Maior resposta aceita da API (a lista de releases tem poucas dezenas de KB).
const MAX_JSON: usize = 4 << 20;
/// Maior instalador aceito (o de hoje tem ~2,5 MB).
pub(crate) const MAX_EXE: usize = 64 << 20;
const DAY: u64 = 24 * 60 * 60;

// ---------------------------------------------------------------- versões

/// Versão semver: `1.4.0`, `1.4.0-beta.2` (com ou sem `v` na frente; o que
/// vem depois de `+` é ignorado, como manda o semver).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pre: Vec<Pre>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Pre {
    Num(u64),
    Text(String),
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let (s, build) = s.split_once('+').map_or((s, None), |(a, b)| (a, Some(b)));
        if build.is_some_and(|b| b.is_empty() || !b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')) {
            return None;
        }
        let (core, pre) = s.split_once('-').map_or((s, None), |(a, b)| (a, Some(b)));
        let num = |p: Option<&str>| -> Option<u64> { p.filter(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))?.parse().ok() };
        let mut it = core.split('.');
        let (major, minor, patch) = (num(it.next())?, num(it.next())?, num(it.next())?);
        if it.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(p) => p
                .split('.')
                .map(|id| {
                    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                        None
                    } else if id.bytes().all(|b| b.is_ascii_digit()) {
                        id.parse().ok().map(Pre::Num)
                    } else {
                        Some(Pre::Text(id.to_string()))
                    }
                })
                .collect::<Option<Vec<_>>>()?,
        };
        Some(Version { major, minor, patch, pre })
    }

    /// A deste executável.
    pub fn current() -> Version {
        // Teste de ponta a ponta: a "versão nova" é o mesmo código compilado
        // com outra versão (`TASKBEAT_E2E_VERSION` na hora de compilar).
        #[cfg(feature = "e2e")]
        if let Some(v) = option_env!("TASKBEAT_E2E_VERSION").and_then(Version::parse) {
            return v;
        }
        Version::parse(env!("CARGO_PKG_VERSION")).expect("versão do Cargo.toml")
    }

    /// Versão de teste (beta, rc…)?
    pub fn is_pre(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, p) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match p {
                Pre::Num(n) => write!(f, "{n}")?,
                Pre::Text(t) => f.write_str(t)?,
            }
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(o.major, o.minor, o.patch)).then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // 1.4.0-beta.1 < 1.4.0
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&o.pre) {
                    let c = match (a, b) {
                        (Pre::Num(x), Pre::Num(y)) => x.cmp(y),
                        (Pre::Num(_), Pre::Text(_)) => Ordering::Less,
                        (Pre::Text(_), Pre::Num(_)) => Ordering::Greater,
                        (Pre::Text(x), Pre::Text(y)) => x.cmp(y),
                    };
                    if c != Ordering::Equal {
                        return c;
                    }
                }
                self.pre.len().cmp(&o.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

// ---------------------------------------------------------------- GitHub

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

/// Resposta da API: uma release (`/releases/latest`) ou a lista (`/releases`).
pub fn parse_releases(json: &str) -> Result<Vec<Release>, String> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Body {
        Many(Vec<Release>),
        One(Box<Release>),
    }
    match serde_json::from_str::<Body>(json) {
        Ok(Body::Many(v)) => Ok(v),
        Ok(Body::One(r)) => Ok(vec![*r]),
        Err(e) => Err(format!("resposta do GitHub inválida: {e}")),
    }
}

/// Para qual sistema é o arquivo procurado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Mac,
}

impl Platform {
    pub const HERE: Platform = if cfg!(target_os = "macos") { Platform::Mac } else { Platform::Windows };

    /// Nome do arquivo da release: `Taskbeat-Setup-1.4.0.exe` / `Taskbeat-1.4.0.dmg`.
    pub fn file_name(self, v: &Version) -> String {
        match self {
            Platform::Windows => format!("Taskbeat-Setup-{v}.exe"),
            Platform::Mac => format!("Taskbeat-{v}.dmg"),
        }
    }
}

/// Versão nova encontrada.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub version: Version,
    /// Página da release (sempre no repositório do Taskbeat).
    pub page: String,
    /// Resumo das novidades (o primeiro parágrafo do texto da release).
    pub notes: String,
    pub file: Asset,
    /// A assinatura do arquivo (`<arquivo>.sig`), se a release tiver.
    pub sig: Option<Asset>,
}

/// Endereço de download aceito: só os arquivos das releases do repositório
/// (nos testes, também o servidor local que faz o papel do GitHub).
fn download_ok(url: &str) -> bool {
    if url.contains("..") {
        return false;
    }
    if url.starts_with(&format!("{RELEASES}download/")) {
        return true;
    }
    #[cfg(any(debug_assertions, feature = "e2e"))]
    if let Some(base) = test_env("TASKBEAT_UPDATE_API")
        && let Some(origin) = base.strip_prefix("http://").and_then(|r| r.split('/').next())
    {
        return url.starts_with(&format!("http://{origin}/"));
    }
    false
}

/// Página da release: a que o GitHub mandou, se for do repositório; senão,
/// montada a partir da versão.
fn page_of(r: &Release, v: &Version) -> String {
    let u = r.html_url.trim();
    let safe = u.strip_prefix(RELEASES).is_some_and(|rest| !rest.is_empty() && !rest.contains("..") && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b"/-._".contains(&b)));
    if safe { u.to_string() } else { format!("{RELEASES}tag/v{v}") }
}

/// A melhor release para este sistema: a de versão mais alta acima de
/// `current`, com o arquivo do sistema. Versões de teste só com `betas`.
pub fn choose(releases: &[Release], current: &Version, betas: bool, platform: Platform) -> Option<Offer> {
    let mut best: Option<Offer> = None;
    for r in releases.iter().filter(|r| !r.draft) {
        let Some(v) = Version::parse(&r.tag_name) else { continue };
        let test = r.prerelease || v.is_pre();
        if (test && !betas) || v <= *current || best.as_ref().is_some_and(|b| b.version >= v) {
            continue;
        }
        let name = platform.file_name(&v);
        let find = |n: &str| r.assets.iter().find(|a| a.name == n && download_ok(&a.browser_download_url)).cloned();
        let Some(file) = find(&name).filter(|a| a.size as usize <= MAX_EXE) else { continue };
        let sig = find(&format!("{name}.sig"));
        best = Some(Offer { page: page_of(r, &v), notes: summary(r.body.as_deref().unwrap_or(""), 280), version: v, file, sig });
    }
    best
}

/// Tira caracteres de controle e de direção do texto (que poderiam disfarçar
/// o que aparece no aviso) e junta os espaços.
fn clean(s: &str) -> String {
    let bidi = |c: char| matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    let s: String = s.chars().map(|c| if c.is_control() || bidi(c) { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Markdown simples → texto: `[texto](link)` vira "texto", somem `**`, `__`,
/// `` ` `` e tags HTML.
fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find(['[', '<', '!']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(t) = tail.strip_prefix("![") {
            // Imagem: some inteira.
            match t.find("](").and_then(|j| t[j..].find(')').map(|k| j + k)) {
                Some(end) => rest = &t[end + 1..],
                None => {
                    rest = t;
                }
            }
        } else if let Some(t) = tail.strip_prefix('[') {
            match t.find("](").and_then(|j| t[j..].find(')').map(|k| (j, j + k))) {
                Some((j, end)) => {
                    out.push_str(&t[..j]);
                    rest = &t[end + 1..];
                }
                None => {
                    out.push('[');
                    rest = t;
                }
            }
        } else if tail.starts_with('<') {
            match tail.find('>') {
                Some(end) => rest = &tail[end + 1..],
                None => {
                    rest = "";
                }
            }
        } else {
            out.push('!');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace("__", "").replace('`', "")
}

/// Resumo das novidades: o primeiro parágrafo de texto do corpo da release
/// (pula títulos, imagens, HTML, citações e tabelas), em texto simples e com
/// no máximo `max` caracteres.
pub fn summary(body: &str, max: usize) -> String {
    let body = body.replace("\r\n", "\n");
    let mut text = String::new();
    for para in body.split("\n\n") {
        let lines: Vec<&str> = para.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let Some(first) = lines.first() else { continue };
        if ["#", "<", ">", "|", "```", "---", "***"].iter().any(|p| first.starts_with(p)) {
            continue;
        }
        let bullet = |l: &str| l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).map(str::to_string);
        let joined: Vec<String> = lines.iter().map(|l| bullet(l).map(|b| format!("• {b}")).unwrap_or_else(|| l.to_string())).collect();
        text = clean(&plain(&joined.join(" ")));
        if !text.is_empty() {
            break;
        }
    }
    if text.chars().count() > max {
        let cut: String = text.chars().take(max).collect();
        // Corta na última palavra inteira.
        let cut = cut.rsplit_once(' ').map(|(a, _)| a.to_string()).unwrap_or(cut);
        text = format!("{}…", cut.trim_end_matches([',', ';', ':', '.', ' ']));
    }
    text
}

// ---------------------------------------------------------------- assinatura

/// O que a chave privada assina (o `build-installer.ps1` monta igual): o
/// nome do arquivo, a versão e o SHA-256 do exe, em hexadecimal minúsculo.
pub fn signed_message(file: &str, version: &Version, sha256_hex: &str) -> String {
    format!("taskbeat-update-v1\nfile: {file}\nversion: {version}\nsha256: {sha256_hex}\n")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hexadecimal → bytes (espaços e quebras de linha são ignorados).
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    let val = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    digits.chunks(2).map(|p| Some(val(p[0])? << 4 | val(p[1])?)).collect()
}

/// Chave pública embutida: X‖Y da curva P-256 (64 bytes), em hexadecimal, na
/// primeira linha de `assets/update-public-key.txt` que não começa com `#`.
pub fn public_key() -> Option<[u8; 64]> {
    #[cfg(any(debug_assertions, feature = "e2e"))]
    if let Some(k) = test_env("TASKBEAT_UPDATE_PUBKEY") {
        return unhex(&k)?.try_into().ok();
    }
    parse_public_key(include_str!("../../assets/update-public-key.txt"))
}

fn parse_public_key(text: &str) -> Option<[u8; 64]> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
    unhex(line)?.try_into().ok()
}

// ---------------------------------------------------------------- estado e registro

/// Testes (só em builds de desenvolvimento ou com `--features e2e`): variáveis
/// que apontam o atualizador para um servidor local e uma chave de teste.
#[cfg(any(debug_assertions, feature = "e2e"))]
pub fn test_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn config_dir() -> Option<PathBuf> {
    crate::config::Config::path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Quando procurou pela última vez e qual versão a pessoa pulou.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    /// Segundos desde 1970.
    pub last_check: u64,
    /// "Pular esta versão" (não avisa mais dela; uma mais nova avisa).
    pub skipped: String,
}

impl State {
    fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("update.toml"))
    }

    pub fn load() -> State {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = toml::to_string(self) {
            let _ = std::fs::write(p, s);
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Registro das atualizações (`update.log` junto da config, sempre ligado:
/// uma assinatura recusada precisa ficar anotada). Passou de 64 KB, o antigo
/// vira `update.old.log`.
pub fn log(msg: &str) {
    use std::io::Write;
    #[cfg(debug_assertions)]
    eprintln!("[update] {msg}");
    #[cfg(windows)]
    crate::audio::log::info(&format!("atualização: {msg}"));
    #[cfg(target_os = "macos")]
    crate::mac::system::log::info(&format!("atualização: {msg}"));
    let Some(dir) = config_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("update.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::rename(&path, dir.join("update.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {} {msg}", now_secs(), Version::current());
    }
}

// ---------------------------------------------------------------- procura

fn api_get(path: &str) -> Result<Vec<Release>, String> {
    #[cfg(any(debug_assertions, feature = "e2e"))]
    let api = test_env("TASKBEAT_UPDATE_API").unwrap_or_else(|| API.to_string());
    #[cfg(not(any(debug_assertions, feature = "e2e")))]
    let api = API.to_string();
    let url = format!("{api}{path}");
    let r = crate::http::get(&url, &[("Accept", "application/vnd.github+json"), ("X-GitHub-Api-Version", "2022-11-28")], MAX_JSON)?;
    match r.status {
        200 => parse_releases(&r.text()),
        // Nenhuma release publicada.
        404 => Ok(Vec::new()),
        s => Err(format!("GitHub respondeu {s}")),
    }
}

/// Pergunta ao GitHub. Sem betas, usa `/releases/latest`; se a última não
/// tiver o arquivo deste sistema (uma release só do Mac, por exemplo), olha a
/// lista para achar a mais nova que tenha.
pub fn find(current: &Version, betas: bool) -> Result<Option<Offer>, String> {
    let list = "/releases?per_page=30";
    if betas {
        return Ok(choose(&api_get(list)?, current, true, Platform::HERE));
    }
    let latest = api_get("/releases/latest")?;
    if let Some(o) = choose(&latest, current, false, Platform::HERE) {
        return Ok(Some(o));
    }
    let newer = latest.iter().filter_map(|r| Version::parse(&r.tag_name)).any(|v| v > *current);
    if !newer {
        return Ok(None);
    }
    Ok(choose(&api_get(list)?, current, false, Platform::HERE))
}

static THREAD: OnceLock<std::thread::Thread> = OnceLock::new();
static MANUAL: AtomicBool = AtomicBool::new(false);

/// "Procurar agora" (configurações): procura já e mostra o resultado mesmo
/// sem versão nova.
pub fn check_now() {
    MANUAL.store(true, AtOrd::Relaxed);
    if let Some(t) = THREAD.get() {
        t.unpark();
    }
}

/// O que fazer com a versão nova encontrada (cada sistema do seu jeito).
fn offer(shared: &Arc<Shared>, o: Offer, manual: bool) {
    #[cfg(windows)]
    win::offer(shared, o, manual);
    #[cfg(target_os = "macos")]
    {
        let _ = (shared, manual);
        crate::mac::update::offer(o);
    }
}

/// Resultado de "Procurar agora" sem versão nova (ou com erro).
fn tell(shared: &Arc<Shared>, result: Result<(), String>) {
    #[cfg(windows)]
    win::tell(shared, result);
    #[cfg(target_os = "macos")]
    {
        let _ = shared;
        crate::mac::update::tell(result);
    }
}

/// Thread da atualização: procura ~30 s depois de abrir e depois uma vez por
/// dia (acorda de hora em hora só para conferir o relógio).
pub fn spawn(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("updater".into()).spawn(move || run(shared));
}

fn run(shared: Arc<Shared>) {
    let _ = THREAD.set(std::thread::current());
    #[cfg(windows)]
    {
        crate::system::background_thread();
        win::after_start(&shared);
    }
    #[allow(unused_mut)]
    let mut delay = Duration::from_secs(30);
    #[cfg(any(debug_assertions, feature = "e2e"))]
    if let Some(s) = test_env("TASKBEAT_UPDATE_DELAY").and_then(|s| s.parse().ok()) {
        delay = Duration::from_secs(s);
    }
    let start = std::time::Instant::now();
    while start.elapsed() < delay && !MANUAL.load(AtOrd::Relaxed) && shared.running.load(AtOrd::Relaxed) {
        std::thread::park_timeout(delay.saturating_sub(start.elapsed()));
    }
    let mut first = true;
    while shared.running.load(AtOrd::Relaxed) {
        let manual = MANUAL.swap(false, AtOrd::Relaxed);
        let (enabled, beta) = {
            let c = shared.config.lock().unwrap();
            (c.update_check, c.update_beta)
        };
        let mut state = State::load();
        let due = first || now_secs().saturating_sub(state.last_check) >= DAY;
        if manual || enabled && due {
            first = false;
            state.last_check = now_secs();
            state.save();
            let current = Version::current();
            // Quem já está num beta continua recebendo betas.
            match find(&current, beta || current.is_pre()) {
                Ok(Some(o)) if manual || o.version.to_string() != state.skipped => {
                    log(&format!("versão {} disponível ({})", o.version, o.page));
                    offer(&shared, o, manual);
                }
                Ok(Some(o)) => log(&format!("versão {} disponível, mas foi pulada", o.version)),
                Ok(None) => {
                    if manual {
                        tell(&shared, Ok(()));
                    }
                }
                Err(e) => {
                    log(&format!("não foi possível procurar: {e}"));
                    if manual {
                        tell(&shared, Err(e));
                    }
                }
            }
            // A consulta carrega o WinHTTP e o TLS do Windows: as páginas que
            // ficaram sem uso voltam para o sistema (como depois de abrir o app).
            #[cfg(windows)]
            crate::system::trim_memory();
        }
        std::thread::park_timeout(Duration::from_secs(60 * 60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn le_versoes() {
        assert_eq!(v("1.3.0"), Version { major: 1, minor: 3, patch: 0, pre: vec![] });
        assert_eq!(v("v1.3.0").to_string(), "1.3.0");
        assert_eq!(v(" V2.10.33 ").to_string(), "2.10.33");
        assert_eq!(v("1.3.0-beta.3").to_string(), "1.3.0-beta.3");
        assert_eq!(v("1.3.0-rc.1+build.5").to_string(), "1.3.0-rc.1");
        assert!(v("1.3.0-beta.3").is_pre());
        assert!(!v("1.3.0").is_pre());
        for bad in ["", "1", "1.3", "1.3.0.1", "a.b.c", "1.3.x", "1.3.0-", "1.3.0-beta..1", "1.3.0-be ta", "1.3.0+", "-1.3.0", "1.3.0-beta/1", "9999999999.0.0"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn compara_versoes_como_o_semver() {
        // Ordem do semver.org (seção 11) e alguns casos do Taskbeat.
        let order = [
            "0.9.9",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.2.0",
            "1.3.0-beta.1",
            "1.3.0-beta.3",
            "1.3.0",
            "1.10.0",
            "2.0.0",
        ];
        for w in order.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
            assert!(v(w[1]) > v(w[0]));
        }
        assert_eq!(v("1.3.0").cmp(&v("v1.3.0+x")), Ordering::Equal);
        assert!(Version::current() >= v("1.3.0"));
    }

    /// Recortes de respostas reais da API (v1.3.0 e um beta do Mac).
    const LATEST: &str = r#"{
        "url": "https://api.github.com/repos/Lu1sGabriel/taskbeat/releases/1",
        "html_url": "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.3.0",
        "tag_name": "v1.3.0", "name": "Taskbeat 1.3.0", "draft": false, "prerelease": false,
        "author": {"login": "Lu1sGabriel", "id": 1},
        "assets": [
            {"name": "Taskbeat-1.3.0.dmg", "size": 2376281, "browser_download_url": "https://github.com/Lu1sGabriel/taskbeat/releases/download/v1.3.0/Taskbeat-1.3.0.dmg"},
            {"name": "Taskbeat-Setup-1.3.0.exe", "size": 2544640, "browser_download_url": "https://github.com/Lu1sGabriel/taskbeat/releases/download/v1.3.0/Taskbeat-Setup-1.3.0.exe"},
            {"name": "Taskbeat-Setup-1.3.0.exe.sig", "size": 129, "browser_download_url": "https://github.com/Lu1sGabriel/taskbeat/releases/download/v1.3.0/Taskbeat-Setup-1.3.0.exe.sig"}
        ],
        "body": "A primeira versão do **Taskbeat** para **Windows e Mac juntos**. No Windows, o visual ficou bem mais bonito e chegou o **mixer de volume**; no Mac, sai a versão estável depois de três betas.\r\n\r\n<p align=\"center\">\r\n  <img src=\"https://raw.githubusercontent.com/x/demo.gif\" width=\"720\" alt=\"demo\">\r\n</p>\r\n\r\n## Novidades\r\n\r\n- Mixer"
    }"#;

    const LIST: &str = r#"[
        {"tag_name": "v1.4.0-beta.1", "prerelease": true, "draft": false, "html_url": "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.4.0-beta.1", "body": "> ⚠️ **Beta.** Teste\n\nBeta do Windows.",
         "assets": [{"name": "Taskbeat-Setup-1.4.0-beta.1.exe", "size": 3, "browser_download_url": "https://github.com/Lu1sGabriel/taskbeat/releases/download/v1.4.0-beta.1/Taskbeat-Setup-1.4.0-beta.1.exe"}]},
        {"tag_name": "v1.3.1-beta.1", "prerelease": true, "draft": false, "html_url": "x", "body": null,
         "assets": [{"name": "Taskbeat-1.3.1-beta.1.dmg", "size": 3, "browser_download_url": "https://github.com/Lu1sGabriel/taskbeat/releases/download/v1.3.1-beta.1/Taskbeat-1.3.1-beta.1.dmg"}]},
        {"tag_name": "v9.9.9", "prerelease": false, "draft": true, "html_url": "", "assets": [{"name": "Taskbeat-Setup-9.9.9.exe", "size": 3, "browser_download_url": "https://github.com/Lu1sGabriel/taskbeat/releases/download/v9.9.9/Taskbeat-Setup-9.9.9.exe"}]},
        {"tag_name": "v1.3.0", "prerelease": false, "draft": false, "html_url": "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.3.0", "body": "Estável.",
         "assets": [{"name": "Taskbeat-Setup-1.3.0.exe", "size": 3, "browser_download_url": "https://github.com/Lu1sGabriel/taskbeat/releases/download/v1.3.0/Taskbeat-Setup-1.3.0.exe"}]}
    ]"#;

    #[test]
    fn le_a_resposta_do_github() {
        let r = parse_releases(LATEST).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].tag_name, "v1.3.0");
        assert_eq!(r[0].assets.len(), 3);
        let o = choose(&r, &v("1.2.0"), false, Platform::Windows).unwrap();
        assert_eq!(o.version, v("1.3.0"));
        assert_eq!(o.file.name, "Taskbeat-Setup-1.3.0.exe");
        assert_eq!(o.sig.as_ref().unwrap().name, "Taskbeat-Setup-1.3.0.exe.sig");
        assert_eq!(o.page, "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.3.0");
        assert!(o.notes.starts_with("A primeira versão do Taskbeat para Windows e Mac juntos."), "{}", o.notes);
        assert!(!o.notes.contains("**") && !o.notes.contains('<'));
        let mac = choose(&r, &v("1.3.0-beta.3"), false, Platform::Mac).unwrap();
        assert_eq!((mac.file.name.as_str(), mac.sig.is_none()), ("Taskbeat-1.3.0.dmg", true));
        // Mesma versão ou mais nova instalada: nada a oferecer.
        assert!(choose(&r, &v("1.3.0"), false, Platform::Windows).is_none());
        assert!(choose(&r, &v("1.4.0"), false, Platform::Windows).is_none());
        // Lixo e formatos inesperados viram erro, não pânico.
        assert!(parse_releases("").is_err());
        assert!(parse_releases("{\"message\": \"Not Found\"}").is_ok_and(|r| choose(&r, &v("0.1.0"), true, Platform::Windows).is_none()));
        assert!(parse_releases("[1, 2]").is_err());
    }

    #[test]
    fn escolhe_a_release_certa() {
        let r = parse_releases(LIST).unwrap();
        assert_eq!(r.len(), 4);
        // Sem betas: a 1.3.0 (o rascunho 9.9.9 nunca conta).
        assert_eq!(choose(&r, &v("1.2.0"), false, Platform::Windows).unwrap().version, v("1.3.0"));
        // Com betas: o beta do Windows; o do Mac não tem exe.
        let o = choose(&r, &v("1.2.0"), true, Platform::Windows).unwrap();
        assert_eq!(o.version, v("1.4.0-beta.1"));
        assert_eq!(o.notes, "Beta do Windows.");
        assert!(o.sig.is_none());
        // No Mac, o único com .dmg.
        let o = choose(&r, &v("1.3.0"), true, Platform::Mac).unwrap();
        assert_eq!(o.version, v("1.3.1-beta.1"));
        // Página fora do repositório é trocada pela do repositório.
        assert_eq!(o.page, "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.3.1-beta.1");
    }

    #[test]
    fn so_baixa_do_repositorio() {
        let mut r = parse_releases(LATEST).unwrap();
        for a in &mut r[0].assets {
            a.browser_download_url = a.browser_download_url.replace("github.com/Lu1sGabriel/taskbeat", "github.com/outro/taskbeat");
        }
        assert!(choose(&r, &v("1.2.0"), false, Platform::Windows).is_none());
        r[0].assets[1].browser_download_url = "https://evil.example/releases/download/v1.3.0/Taskbeat-Setup-1.3.0.exe".into();
        assert!(choose(&r, &v("1.2.0"), false, Platform::Windows).is_none());
        // Página que aponta para outro lugar não é usada.
        let mut r = parse_releases(LATEST).unwrap();
        r[0].html_url = "https://evil.example/".into();
        assert_eq!(choose(&r, &v("1.2.0"), false, Platform::Windows).unwrap().page, "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.3.0");
        r[0].html_url = "https://github.com/Lu1sGabriel/taskbeat/releases/../../evil".into();
        assert_eq!(choose(&r, &v("1.2.0"), false, Platform::Windows).unwrap().page, "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v1.3.0");
        // Instalador gigante: recusado antes de baixar.
        let mut r = parse_releases(LATEST).unwrap();
        r[0].assets[1].size = (MAX_EXE + 1) as u64;
        assert!(choose(&r, &v("1.2.0"), false, Platform::Windows).is_none());
    }

    #[test]
    fn resumo_das_novidades() {
        assert_eq!(summary("## Novidades\n\n- **Um** item\n- [Dois](https://x) itens\n\nFim.", 100), "• Um item • Dois itens");
        assert_eq!(summary("<p>x</p>\n\n![img](a.png) Texto com `código` e <b>negrito</b>.", 100), "Texto com código e negrito.");
        assert_eq!(summary("Linha um\ncontinua\u{202E}aqui\u{0007}.", 100), "Linha um continua aqui .");
        let long = "palavra ".repeat(100);
        let s = summary(&long, 50);
        assert!(s.ends_with('…') && s.chars().count() <= 51, "{s}");
        assert_eq!(summary("", 10), "");
        assert_eq!(summary("# Só título", 10), "");
        assert_eq!(summary("Sem [link fechado e ! solto", 100), "Sem [link fechado e ! solto");
    }

    #[test]
    fn mensagem_assinada_e_hex() {
        let m = signed_message("Taskbeat-Setup-1.4.0.exe", &v("v1.4.0"), "ab01");
        assert_eq!(m, "taskbeat-update-v1\nfile: Taskbeat-Setup-1.4.0.exe\nversion: 1.4.0\nsha256: ab01\n");
        assert_eq!(hex(&[0, 1, 0xab, 0xff]), "0001abff");
        assert_eq!(unhex("0001ABff\n"), Some(vec![0, 1, 0xab, 0xff]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
        let k = "11".repeat(64);
        assert_eq!(parse_public_key(&format!("# comentário\n\n{k}\n")), Some([0x11; 64]));
        assert_eq!(parse_public_key("# sem chave\n"), None);
        assert_eq!(parse_public_key("1234"), None);
    }

    #[test]
    fn estado_vira_toml() {
        let s = State { last_check: 42, skipped: "1.4.0".into() };
        let t = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<State>(&t).unwrap(), s);
        assert_eq!(toml::from_str::<State>("").unwrap(), State::default());
    }
}
