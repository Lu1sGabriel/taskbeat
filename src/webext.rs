//! Extensão do Taskbeat para o navegador (pasta `extension/`): o estado real
//! do aleatório e do repetir do YouTube Music, lido da própria página.
//!
//! Caminho das mensagens (JSON com 4 bytes de tamanho na frente, o formato do
//! Native Messaging dos navegadores Chromium):
//!
//! ```text
//! aba do YouTube Music ─ service worker da extensão ─ stdin/stdout ─
//!   Taskbeat.exe (modo host, aberto pelo navegador) ─ pipe nomeado ─ Taskbeat
//! ```
//!
//! O navegador abre o próprio `Taskbeat.exe` como "host" (com a origem da
//! extensão como argumento). Esse processo só repassa as mensagens entre o
//! navegador e o Taskbeat em execução, por um pipe nomeado desta sessão do
//! Windows (só o usuário tem acesso; nenhuma porta de rede). Sem o Taskbeat
//! aberto, o host espera um evento nomeado que o Taskbeat sinaliza ao abrir:
//! nada de tentar de novo a cada segundo.
//!
//! A extensão é opcional: sem ela, `webplayer.rs` continua lendo a página pela
//! UI Automation. Com ela conectada e a música batendo com a sessão do
//! Windows, o estado e os comandos vêm dela.

use crate::audio::log;
use crate::shared::{Shared, WebCmd};
use serde::Deserialize;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenSessionId, TokenUser};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, INFINITE, OpenProcessToken, ResetEvent, SetEvent, WaitForSingleObject};
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// Nome do host no registro do navegador (`NativeMessagingHosts\<nome>`).
pub const HOST_NAME: &str = "com.taskbeat.ytmusic";
/// IDs da extensão que podem abrir o host: o da pasta `extension/` carregada
/// sem compactação (fixo pela chave pública do `manifest.json`) e, depois de
/// publicada, o que a Chrome Web Store der (ver `extension/store/PUBLICAR.md`).
pub const EXTENSION_IDS: &[&str] = &["abofmfclmpmmbgecemcglhcklnmcapfh"];
/// Página da extensão na Chrome Web Store (vazia até publicar).
pub const STORE_URL: &str = "";
/// Sem a loja: as instruções para carregar a extensão sem compactação.
const GUIDE_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/tree/main/extension#readme");

/// Onde instalar a extensão: a loja, quando publicada; senão, as instruções.
pub fn install_url() -> &'static str {
    if STORE_URL.is_empty() { GUIDE_URL } else { STORE_URL }
}

/// Os arquivos da extensão, dentro do próprio executável (sem a pasta
/// `store/`, que é só para publicar): enquanto ela não está na Chrome Web
/// Store, o Taskbeat grava a pasta para o usuário carregar sem compactação.
const EXT_FILES: &[(&str, &[u8])] = &[
    ("manifest.json", include_bytes!("../extension/manifest.json")),
    ("content.js", include_bytes!("../extension/content.js")),
    ("sw.js", include_bytes!("../extension/sw.js")),
    ("README.md", include_bytes!("../extension/README.md")),
    ("icons/icon-16.png", include_bytes!("../extension/icons/icon-16.png")),
    ("icons/icon-48.png", include_bytes!("../extension/icons/icon-48.png")),
    ("icons/icon-128.png", include_bytes!("../extension/icons/icon-128.png")),
    ("_locales/de/messages.json", include_bytes!("../extension/_locales/de/messages.json")),
    ("_locales/en/messages.json", include_bytes!("../extension/_locales/en/messages.json")),
    ("_locales/es/messages.json", include_bytes!("../extension/_locales/es/messages.json")),
    ("_locales/pt_BR/messages.json", include_bytes!("../extension/_locales/pt_BR/messages.json")),
];

/// Grava a extensão na pasta `dir` (só os arquivos que mudaram: o navegador
/// lê dali enquanto ela está carregada).
pub fn write_extension(dir: &std::path::Path) -> io::Result<()> {
    for (name, bytes) in EXT_FILES {
        let p = dir.join(name);
        if std::fs::read(&p).ok().as_deref() == Some(*bytes) {
            continue;
        }
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, bytes)?;
    }
    Ok(())
}

/// Pasta `extension` ao lado do Taskbeat.exe (a instalação a cria).
fn installed_extension_dir() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("extension"))
}

/// Grava a pasta da extensão para carregar sem compactação: ao lado do
/// Taskbeat.exe ou, sem permissão de escrita lá, em `%LOCALAPPDATA%\Taskbeat`.
pub fn extension_folder() -> Option<std::path::PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Taskbeat").join("extension"));
    [installed_extension_dir(), local].into_iter().flatten().find(|d| write_extension(d).is_ok())
}

/// "Instalar extensão": a página da loja quando ela estiver publicada; até
/// lá, a pasta da extensão (no Explorador) e o passo a passo para carregá-la.
pub fn open_install_page() {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        if STORE_URL.is_empty()
            && let Some(dir) = extension_folder()
        {
            ShellExecuteW(None, windows::core::w!("open"), &HSTRING::from(dir.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        ShellExecuteW(None, windows::core::w!("open"), &HSTRING::from(install_url()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}
/// Maior mensagem aceita (o navegador manda no máximo 1 MB ao host; as nossas
/// têm poucas centenas de bytes).
pub const MAX_FRAME: usize = 1 << 20;

// ---------------------------------------------------------------- mensagens

/// Lê uma mensagem (4 bytes de tamanho, na ordem do processador, e o JSON).
/// `None`: o outro lado fechou entre duas mensagens.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let n = u32::from_ne_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("mensagem de {n} bytes")));
    }
    let mut body = vec![0; n];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Escreve uma mensagem de uma vez (tamanho e corpo juntos: duas threads não
/// misturam pedaços).
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_ne_bytes());
    buf.extend_from_slice(body);
    w.write_all(&buf)?;
    w.flush()
}

/// Mensagens da extensão para o Taskbeat.
#[derive(Deserialize, Debug, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum FromExt {
    /// A extensão conectou (ou reconectou): nome do navegador.
    Hello {
        #[serde(default)]
        browser: String,
    },
    /// Estado de uma aba do YouTube Music.
    Player(PlayerMsg),
    /// A aba fechou ou saiu do YouTube Music.
    Gone { id: String },
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug, PartialEq, Default)]
pub struct PlayerMsg {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub playing: bool,
    /// `None`: a página não tem mais o atributo (o YouTube mudou a página).
    #[serde(default)]
    pub shuffle: Option<bool>,
    /// "NONE", "ALL" ou "ONE".
    #[serde(default)]
    pub repeat: Option<String>,
    /// Endereço da capa em alta resolução (extensão 1.1+; vazio nas antigas).
    #[serde(default)]
    pub art: String,
}

pub fn parse(body: &[u8]) -> Option<FromExt> {
    serde_json::from_slice(body).ok()
}

/// Repetir da página no formato do Taskbeat (0 = desligado, 1 = uma, 2 = tudo).
fn repeat_code(s: &str) -> Option<u8> {
    match s {
        "NONE" => Some(0),
        "ONE" => Some(1),
        "ALL" => Some(2),
        _ => None,
    }
}

/// Comando para a aba `id`: "shuffle" (alterna) ou "repeat" (próximo do ciclo).
fn command_body(id: &str, cmd: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "t": "cmd", "id": id, "cmd": cmd })).unwrap_or_default()
}

// ---------------------------------------------------------------- players

/// Uma aba do YouTube Music contada pela extensão.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtPlayer {
    /// Conexão (navegador) de onde veio.
    pub client: u64,
    /// Aba, no navegador.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub shuffle: Option<bool>,
    pub repeat: Option<u8>,
    /// Capa em alta resolução (endereço; vazio se a extensão não mandou).
    pub art: String,
    /// Ordem da última mudança (a mais recente ganha num empate).
    pub seq: u64,
}

/// As abas e os navegadores conectados (sem os pipes: dá para testar).
#[derive(Default)]
pub struct Book {
    browsers: Vec<(u64, String)>,
    players: Vec<ExtPlayer>,
    seq: u64,
}

impl Book {
    pub const fn new() -> Self {
        Self { browsers: Vec::new(), players: Vec::new(), seq: 0 }
    }

    /// Aplica uma mensagem da conexão `client`. `true`: algo mudou.
    pub fn apply(&mut self, client: u64, msg: FromExt) -> bool {
        match msg {
            FromExt::Hello { browser } => {
                match self.browsers.iter_mut().find(|(c, _)| *c == client) {
                    Some((_, b)) => *b = browser,
                    None => self.browsers.push((client, browser)),
                }
                false
            }
            FromExt::Player(m) => {
                let new = ExtPlayer {
                    client,
                    id: m.id,
                    title: m.title,
                    artist: m.artist,
                    playing: m.playing,
                    shuffle: m.shuffle,
                    repeat: m.repeat.as_deref().and_then(repeat_code),
                    art: m.art,
                    seq: 0,
                };
                let old = self.players.iter_mut().find(|p| p.client == client && p.id == new.id);
                if old.as_ref().is_some_and(|o| ExtPlayer { seq: 0, ..(*o).clone() } == new) {
                    return false;
                }
                self.seq += 1;
                let new = ExtPlayer { seq: self.seq, ..new };
                match old {
                    Some(o) => *o = new,
                    None => self.players.push(new),
                }
                true
            }
            FromExt::Gone { id } => {
                let before = self.players.len();
                self.players.retain(|p| !(p.client == client && p.id == id));
                before != self.players.len()
            }
            FromExt::Other => false,
        }
    }

    /// A conexão fechou: some com o navegador e as abas dele.
    pub fn remove_client(&mut self, client: u64) -> bool {
        self.browsers.retain(|(c, _)| *c != client);
        let before = self.players.len();
        self.players.retain(|p| p.client != client);
        before != self.players.len()
    }

    fn browser(&self, client: u64) -> Option<&str> {
        self.browsers.iter().find(|(c, _)| *c == client).map(|(_, b)| b.as_str())
    }

    /// A aba que toca a música da sessão do Windows (`aumid`, `title`): mesmo
    /// título (`same_title`), do mesmo navegador da sessão, com o estado dos
    /// botões lido. Várias: a que está tocando, depois a que mudou por último.
    pub fn pick(&self, aumid: &str, title: &str) -> Option<&ExtPlayer> {
        let session = browser_key(aumid)?;
        self.players
            .iter()
            .filter(|p| p.shuffle.is_some() || p.repeat.is_some())
            .filter(|p| self.browser(p.client).and_then(browser_key).is_none_or(|b| b == session))
            .filter(|p| crate::webplayer::same_title(&p.title, title))
            .max_by_key(|p| (p.playing, p.seq))
    }

    /// Alguma aba do navegador da sessão (a música dela pode chegar um pouco
    /// depois da do Windows).
    pub fn any_from(&self, aumid: &str) -> bool {
        let Some(session) = browser_key(aumid) else { return false };
        self.players.iter().any(|p| self.browser(p.client).and_then(browser_key).is_none_or(|b| b == session))
    }
}

/// Qual navegador, pelo AUMID da sessão ("Brave", "Brave._crx_…", "MSEdge",
/// "Chrome") ou pelo nome que a extensão manda ("Brave", "Edge", "Chrome").
fn browser_key(s: &str) -> Option<&'static str> {
    let s = s.to_lowercase();
    [("edge", "edge"), ("brave", "brave"), ("vivaldi", "vivaldi"), ("opera", "opera"), ("chrom", "chrome")].into_iter().find(|(k, _)| s.contains(k)).map(|(_, v)| v)
}

// ---------------------------------------------------------------- estado global

struct Conn {
    id: u64,
    pipe: Arc<Pipe>,
}

struct Registry {
    book: Book,
    conns: Vec<Conn>,
    next: u64,
}

static REG: Mutex<Registry> = Mutex::new(Registry { book: Book::new(), conns: Vec::new(), next: 1 });

/// Estado da aba que toca a música da sessão, se a extensão conhece uma.
pub fn lookup(aumid: &str, title: &str) -> Option<ExtPlayer> {
    REG.lock().unwrap().book.pick(aumid, title).cloned()
}

/// A extensão tem abas do navegador desta sessão (vale esperar por ela).
pub fn has_players(aumid: &str) -> bool {
    REG.lock().unwrap().book.any_from(aumid)
}

/// Navegadores com a extensão conectada agora (para as configurações).
pub fn connected() -> Vec<String> {
    let r = REG.lock().unwrap();
    let mut v: Vec<String> = r.book.browsers.iter().map(|(_, b)| b.clone()).filter(|b| !b.is_empty()).collect();
    // Dois perfis do mesmo navegador contam uma vez.
    v.sort();
    v.dedup();
    v
}

/// Manda "shuffle" ou "repeat" para a aba `p`. `false`: a conexão sumiu.
pub fn command(p: &ExtPlayer, cmd: &str) -> bool {
    let pipe = REG.lock().unwrap().conns.iter().find(|c| c.id == p.client).map(|c| c.pipe.clone());
    let Some(pipe) = pipe else { return false };
    let ok = write_frame(&mut &*pipe, &command_body(&p.id, cmd)).is_ok();
    log::info(&format!("extensão: {cmd} na aba {} ({ok})", p.id));
    ok
}

// ---------------------------------------------------------------- pipe

/// Pipe com E/S sobreposta: uma thread lê enquanto outra escreve (num handle
/// síncrono, a escrita esperaria a leitura terminar).
struct Pipe {
    h: HANDLE,
    write: Mutex<()>,
}

unsafe impl Send for Pipe {}
unsafe impl Sync for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.h);
        }
    }
}

fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error((e.code().0 & 0xFFFF) as i32)
}

impl Pipe {
    fn new(h: HANDLE) -> Self {
        Self { h, write: Mutex::new(()) }
    }

    /// Uma leitura ou escrita, esperando ela terminar.
    fn io(&self, read: Option<&mut [u8]>, write: Option<&[u8]>) -> io::Result<usize> {
        unsafe {
            let ev = CreateEventW(None, true, false, PCWSTR::null()).map_err(win_err)?;
            let mut ov = OVERLAPPED { hEvent: ev, ..Default::default() };
            let r = match (read, write) {
                (Some(b), _) => ReadFile(self.h, Some(b), None, Some(&mut ov)),
                (_, Some(b)) => WriteFile(self.h, Some(b), None, Some(&mut ov)),
                _ => Ok(()),
            };
            let out = match r {
                Err(e) if e.code() != ERROR_IO_PENDING.to_hresult() => Err(e),
                _ => {
                    let mut n = 0u32;
                    GetOverlappedResult(self.h, &ov, &mut n, true).map(|()| n as usize)
                }
            };
            let _ = CloseHandle(ev);
            out.map_err(win_err)
        }
    }
}

impl Read for &Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.io(Some(buf), None) {
            // O outro lado fechou: fim (como um arquivo).
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE.0 as i32) => Ok(0),
            r => r,
        }
    }
}

impl Write for &Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io(None, Some(buf))
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        let _g = self.write.lock().unwrap();
        let mut rest = buf;
        while !rest.is_empty() {
            match self.io(None, Some(rest))? {
                0 => return Err(io::ErrorKind::WriteZero.into()),
                n => rest = &rest[n..],
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Usuário (SID em texto) e sessão do Windows deste processo.
fn user_and_session() -> Option<(String, u32)> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut buf = vec![0u8; 256];
        let mut len = 0u32;
        let user = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as _), buf.len() as u32, &mut len).ok().and_then(|()| {
            let tu = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut s = PWSTR::null();
            ConvertSidToStringSidW(tu.User.Sid, &mut s).ok()?;
            let out = s.to_string().ok();
            let _ = LocalFree(Some(HLOCAL(s.0 as _)));
            out
        });
        let mut session = 0u32;
        let ok = GetTokenInformation(token, TokenSessionId, Some(&mut session as *mut u32 as _), 4, &mut len).is_ok();
        let _ = CloseHandle(token);
        Some((user?, if ok { session } else { 0 }))
    }
}

/// Pipe desta sessão do Windows (o nome de pipe vale para a máquina toda).
fn pipe_name(session: u32) -> String {
    format!(r"\\.\pipe\Taskbeat.Extension.{session}")
}

/// Evento que o Taskbeat sinaliza a cada vez que o pipe fica pronto para uma
/// conexão nova (o host espera por ele em vez de tentar de tempos em tempos).
const READY_EVENT: &str = r"Local\Taskbeat.Extension.Ready";

// ---------------------------------------------------------------- Taskbeat (servidor)

/// Abre o pipe e atende os hosts dos navegadores (um por navegador com a
/// extensão), numa thread que fica parada esperando conexão.
pub fn spawn(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("browser-extension".into()).spawn(move || {
        crate::system::background_thread();
        if let Err(e) = serve(&shared) {
            log::warn(&format!("extensão do navegador: pipe indisponível ({e})"));
        }
    });
}

fn serve(shared: &Arc<Shared>) -> io::Result<()> {
    let (sid, session) = user_and_session().ok_or_else(|| io::Error::other("usuário"))?;
    let name = HSTRING::from(pipe_name(session));
    // Só este usuário (e o sistema) abre o pipe.
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")), SDDL_REVISION_1, &mut sd, None) }.map_err(win_err)?;
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
    let ready = unsafe { CreateEventW(None, true, false, &HSTRING::from(READY_EVENT)) }.map_err(win_err)?;
    let mut first = true;
    let mut tries = 0;
    while shared.is_running() {
        let flags = FILE_FLAG_OVERLAPPED | PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { Default::default() };
        let h = unsafe { CreateNamedPipeW(&name, flags, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 16 * 1024, 16 * 1024, 0, Some(&sa)) };
        if h.is_invalid() {
            let e = io::Error::last_os_error();
            if first {
                // Outro processo já tem esse pipe: o Taskbeat anterior ainda
                // fechando (tenta por alguns segundos) ou outra cópia aberta.
                tries += 1;
                if tries >= 10 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
            log::warn(&format!("extensão do navegador: pipe ({e})"));
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        first = false;
        let pipe = Arc::new(Pipe::new(h));
        // Pronto para um navegador: acorda os hosts que estão esperando.
        unsafe {
            let _ = SetEvent(ready);
        }
        if let Err(e) = accept(&pipe) {
            log::warn(&format!("extensão do navegador: conexão ({e})"));
            continue;
        }
        let shared = shared.clone();
        let _ = std::thread::Builder::new().name("browser-extension-conn".into()).spawn(move || {
            crate::system::background_thread();
            client(&shared, pipe);
        });
    }
    Ok(())
}

/// Espera um host conectar no pipe.
fn accept(pipe: &Pipe) -> io::Result<()> {
    unsafe {
        let ev = CreateEventW(None, true, false, PCWSTR::null()).map_err(win_err)?;
        let mut ov = OVERLAPPED { hEvent: ev, ..Default::default() };
        let r = match ConnectNamedPipe(pipe.h, Some(&mut ov)) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                let mut n = 0u32;
                GetOverlappedResult(pipe.h, &ov, &mut n, true)
            }
            Err(e) => Err(e),
        };
        let _ = CloseHandle(ev);
        r.map_err(win_err)
    }
}

/// Uma conexão (um navegador): lê as mensagens até ela fechar.
fn client(shared: &Arc<Shared>, pipe: Arc<Pipe>) {
    let id = {
        let mut r = REG.lock().unwrap();
        let id = r.next;
        r.next += 1;
        r.conns.push(Conn { id, pipe: pipe.clone() });
        id
    };
    log::info(&format!("extensão do navegador: conexão {id} aberta"));
    loop {
        let body = match read_frame(&mut &*pipe) {
            Ok(Some(b)) => b,
            Ok(None) => break,
            Err(e) => {
                log::warn(&format!("extensão do navegador: conexão {id} ({e})"));
                break;
            }
        };
        let Some(msg) = parse(&body) else { continue };
        if log::verbose() {
            log::info(&format!("extensão do navegador ({id}): {msg:?}"));
        }
        let hello = matches!(msg, FromExt::Hello { .. });
        let changed = REG.lock().unwrap().book.apply(id, msg);
        if changed || hello {
            shared.send_web(WebCmd::Ext);
        }
    }
    {
        let mut r = REG.lock().unwrap();
        r.conns.retain(|c| c.id != id);
        r.book.remove_client(id);
    }
    log::info(&format!("extensão do navegador: conexão {id} fechada"));
    shared.send_web(WebCmd::Ext);
}

// ---------------------------------------------------------------- host (aberto pelo navegador)

/// Este processo foi aberto pelo navegador como host do Native Messaging? O
/// navegador passa a origem da extensão (`chrome-extension://<id>/`) como
/// primeiro argumento (e, no Windows, `--parent-window=<n>`); `--native-host`
/// serve para testar à mão.
pub fn host_mode() -> bool {
    std::env::args().skip(1).any(|a| a.starts_with("chrome-extension://") || a == "--native-host")
}

/// Repassa as mensagens entre o navegador (stdin/stdout) e o Taskbeat (pipe)
/// até o navegador fechar a conexão. O navegador liga stdin/stdout a pipes
/// dele, então o subsistema "windows" do executável não atrapalha.
pub fn run_host() {
    crate::system::background_thread();
    let slot: Arc<Mutex<Option<Arc<Pipe>>>> = Arc::new(Mutex::new(None));
    {
        let slot = slot.clone();
        let _ = std::thread::Builder::new().name("host-pipe".into()).spawn(move || {
            crate::system::background_thread();
            host_pipe(&slot);
        });
    }
    // Do navegador para o Taskbeat (sem o Taskbeat aberto, descarta: ao
    // conectar, a extensão manda tudo de novo).
    let mut stdin = io::stdin().lock();
    loop {
        match read_frame(&mut stdin) {
            Ok(Some(body)) => {
                let pipe = slot.lock().unwrap().clone();
                if let Some(p) = pipe {
                    let _ = write_frame(&mut &*p, &body);
                }
            }
            // O navegador fechou a conexão (ou a aba/extensão): fim.
            Ok(None) | Err(_) => std::process::exit(0),
        }
    }
}

fn to_browser(body: &[u8]) -> io::Result<()> {
    write_frame(&mut io::stdout().lock(), body)
}

/// Conecta no Taskbeat (esperando ele abrir) e repassa o que ele manda ao
/// navegador. Só esta thread escreve no stdout.
fn host_pipe(slot: &Mutex<Option<Arc<Pipe>>>) {
    if to_browser(br#"{"t":"host"}"#).is_err() {
        std::process::exit(0);
    }
    let Some((_, session)) = user_and_session() else { return };
    let name = HSTRING::from(pipe_name(session));
    let Ok(ready) = (unsafe { CreateEventW(None, true, false, &HSTRING::from(READY_EVENT)) }) else { return };
    loop {
        // Parado esperando: devolve ao Windows a memória que só serviu para abrir.
        crate::system::trim_memory();
        let pipe = Arc::new(Pipe::new(connect_wait(&name, ready)));
        *slot.lock().unwrap() = Some(pipe.clone());
        // O Taskbeat acabou de conectar: a extensão manda tudo que sabe.
        if to_browser(br#"{"t":"sync"}"#).is_err() {
            std::process::exit(0);
        }
        while let Ok(Some(body)) = read_frame(&mut &*pipe) {
            if to_browser(&body).is_err() {
                std::process::exit(0);
            }
        }
        // O Taskbeat fechou: espera ele voltar.
        *slot.lock().unwrap() = None;
    }
}

/// Conecta no pipe do Taskbeat; sem ele aberto, dorme até o evento de pronto.
fn connect_wait(name: &HSTRING, ready: HANDLE) -> HANDLE {
    let open = || unsafe {
        CreateFileW(name, (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, None)
    };
    loop {
        match open() {
            Ok(h) => return h,
            // Existe, mas atendendo outro navegador: a próxima instância vem já.
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => unsafe {
                let _ = WaitNamedPipeW(name, 2000);
            },
            Err(_) => unsafe {
                // Fecha o evento e tenta mais uma vez antes de dormir (o
                // Taskbeat pode ter aberto o pipe entre a tentativa e o
                // ResetEvent); a pausa evita girar sem parar se algo der errado.
                let _ = ResetEvent(ready);
                if let Ok(h) = open() {
                    return h;
                }
                WaitForSingleObject(ready, INFINITE);
                std::thread::sleep(std::time::Duration::from_millis(300));
            },
        }
    }
}

// ---------------------------------------------------------------- registro no navegador

/// Conteúdo do manifesto do host (`NativeMessagingHosts`), apontando para `exe`.
pub fn host_manifest(exe: &std::path::Path) -> String {
    let origins: Vec<String> = EXTENSION_IDS.iter().map(|id| format!("chrome-extension://{id}/")).collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "name": HOST_NAME,
        "description": "Taskbeat: aleatório e repetir do YouTube Music",
        "path": exe.display().to_string(),
        "type": "stdio",
        "allowed_origins": origins,
    }))
    .unwrap_or_default()
}

/// Chaves do registro (em HKCU) onde os navegadores procuram o host. O Brave
/// e outros Chromium no Windows leem a do Chrome (conferido no `chrome.dll`
/// do Brave: só `Google\Chrome` e `Chromium`); o Edge tem a dele.
pub fn registry_keys() -> [String; 3] {
    [r"Software\Google\Chrome", r"Software\Chromium", r"Software\Microsoft\Edge"].map(|b| format!(r"{b}\NativeMessagingHosts\{HOST_NAME}"))
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::shared::Shared;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// O caminho inteiro sem o navegador: o `taskbeat.exe` de release
    /// (subsistema "windows") aberto como o Chrome abre um host (`cmd /d /c`,
    /// stdin/stdout em pipes), esperando o Taskbeat; o servidor do pipe sobe
    /// depois e o host conecta pelo evento; mensagens nos dois sentidos; o host
    /// sai quando o "navegador" fecha o stdin. Precisa de `cargo build --release`
    /// e de nenhum Taskbeat com a extensão aberto (o pipe é da sessão).
    /// `cargo test host_relay -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn host_relay() {
        let exe = std::env::var("TASKBEAT_EXE").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(r"target\release\taskbeat.exe"));
        assert!(exe.exists(), "sem {}", exe.display());
        let mut child = Command::new("cmd.exe")
            .arg("/d")
            .arg("/c")
            .arg(&exe)
            .arg(format!("chrome-extension://{}/", EXTENSION_IDS[0]))
            .arg("--parent-window=0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let (tx_out, rx_out) = std::sync::mpsc::channel::<serde_json::Value>();
        let mut stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            while let Ok(Some(b)) = read_frame(&mut stdout) {
                let _ = tx_out.send(serde_json::from_slice(&b).unwrap());
            }
        });
        let next = |what: &str| {
            let t = Instant::now();
            let v = rx_out.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| panic!("sem \"{what}\" do host"));
            println!("host → navegador: {v} em {:?}", t.elapsed());
            v
        };
        // O host abriu (mesmo sem o Taskbeat).
        assert_eq!(next("host")["t"], "host");
        std::thread::sleep(Duration::from_millis(700));
        // O "Taskbeat" abre o pipe: o host acorda pelo evento e pede tudo.
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let (tx, rx) = std::sync::mpsc::channel();
        *shared.web_tx.lock().unwrap() = Some(tx);
        let t0 = Instant::now();
        spawn(shared.clone());
        assert_eq!(next("sync")["t"], "sync");
        println!("conectou {:?} depois de o pipe abrir", t0.elapsed());
        // Do navegador para o Taskbeat.
        write_frame(&mut stdin, br#"{"t":"hello","browser":"Brave","version":"1.0.0"}"#).unwrap();
        write_frame(&mut stdin, br#"{"t":"player","id":"7","title":"Thanatos","artist":"M83","playing":true,"shuffle":true,"repeat":"ONE"}"#).unwrap();
        let t = Instant::now();
        while lookup("Brave", "Thanatos").is_none() {
            assert!(t.elapsed() < Duration::from_secs(5), "o estado não chegou");
            let _ = rx.recv_timeout(Duration::from_millis(200));
        }
        println!("estado chegou ao Taskbeat em {:?}", t.elapsed());
        let p = lookup("Brave", "Thanatos").unwrap();
        assert_eq!((p.shuffle, p.repeat, p.playing), (Some(true), Some(1), true));
        assert_eq!(connected(), vec!["Brave".to_string()]);
        // Do Taskbeat para a aba.
        assert!(command(&p, "repeat"));
        assert_eq!(next("cmd"), serde_json::json!({"t":"cmd","id":"7","cmd":"repeat"}));
        // O navegador fecha a conexão: o host sai, e o Taskbeat esquece a aba.
        drop(stdin);
        let t = Instant::now();
        loop {
            if let Some(st) = child.try_wait().unwrap() {
                println!("cmd/host saiu ({st}) em {:?}", t.elapsed());
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(5), "o host não saiu");
            std::thread::sleep(Duration::from_millis(50));
        }
        let t = Instant::now();
        while lookup("Brave", "Thanatos").is_some() {
            assert!(t.elapsed() < Duration::from_secs(5), "a aba não sumiu");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(connected().is_empty());
        shared.running.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mensagens_com_tamanho() {
        let mut buf = Vec::new();
        write_frame(&mut buf, br#"{"t":"sync"}"#).unwrap();
        write_frame(&mut buf, b"").unwrap();
        assert_eq!(&buf[..4], &12u32.to_ne_bytes());
        let mut r = &buf[..];
        assert_eq!(read_frame(&mut r).unwrap().as_deref(), Some(&br#"{"t":"sync"}"#[..]));
        assert_eq!(read_frame(&mut r).unwrap().as_deref(), Some(&b""[..]));
        // Fim limpo entre mensagens.
        assert_eq!(read_frame(&mut r).unwrap(), None);
        // Cortada no meio do tamanho ou do corpo: erro.
        assert!(read_frame(&mut &buf[..2]).is_err());
        assert!(read_frame(&mut &buf[..10]).is_err());
        // Tamanho absurdo: recusa sem alocar.
        let big = ((MAX_FRAME + 1) as u32).to_ne_bytes();
        assert!(read_frame(&mut &big[..]).is_err());
    }

    /// Leitor que entrega um byte por vez (como um pipe devagar).
    struct Trickle<'a>(&'a [u8]);
    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() || buf.is_empty() {
                return Ok(0);
            }
            buf[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }

    #[test]
    fn mensagem_aos_pedacos() {
        let mut buf = Vec::new();
        write_frame(&mut buf, "{\"t\":\"player\",\"id\":\"1\",\"title\":\"Canção\"}".as_bytes()).unwrap();
        let body = read_frame(&mut Trickle(&buf)).unwrap().unwrap();
        assert!(matches!(parse(&body), Some(FromExt::Player(PlayerMsg { title, .. })) if title == "Canção"));
    }

    /// A pasta gravada tem todos os arquivos do `manifest.json` (ícones e
    /// traduções) e nada da pasta da loja; gravar de novo não muda nada.
    #[test]
    fn pasta_da_extensao() {
        let dir = std::env::temp_dir().join(format!("taskbeat-ext-teste-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_extension(&dir).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        for icon in manifest["icons"].as_object().unwrap().values() {
            assert!(dir.join(icon.as_str().unwrap()).exists(), "{icon}");
        }
        assert!(dir.join(manifest["background"]["service_worker"].as_str().unwrap()).exists());
        for js in manifest["content_scripts"][0]["js"].as_array().unwrap() {
            assert!(dir.join(js.as_str().unwrap()).exists(), "{js}");
        }
        for lang in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/extension/_locales")).unwrap() {
            let lang = lang.unwrap().file_name();
            assert!(dir.join("_locales").join(&lang).join("messages.json").exists(), "{lang:?}");
        }
        assert!(!dir.join("store").exists());
        let before = std::fs::metadata(dir.join("sw.js")).unwrap().modified().unwrap();
        write_extension(&dir).unwrap();
        assert_eq!(std::fs::metadata(dir.join("sw.js")).unwrap().modified().unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn estado_da_pagina() {
        let m = parse(br#"{"t":"player","id":"42","title":"Thanatos","artist":"M83","album":"x","playing":true,"shuffle":true,"repeat":"ONE"}"#);
        assert_eq!(
            m,
            Some(FromExt::Player(PlayerMsg { id: "42".into(), title: "Thanatos".into(), artist: "M83".into(), playing: true, shuffle: Some(true), repeat: Some("ONE".into()), art: String::new() }))
        );
        // A capa em alta resolução (extensão 1.1).
        let m = parse(br#"{"t":"player","id":"9","title":"Swim","art":"https://lh3.googleusercontent.com/x=w1200-h1200-l90-rj"}"#);
        assert!(matches!(m, Some(FromExt::Player(PlayerMsg { ref art, .. })) if art.ends_with("=w1200-h1200-l90-rj")));
        // Página que mudou (sem os atributos): null.
        let m = parse(br#"{"t":"player","id":"1","title":"A","shuffle":null,"repeat":null}"#);
        assert!(matches!(m, Some(FromExt::Player(PlayerMsg { shuffle: None, repeat: None, .. }))));
        assert_eq!(parse(br#"{"t":"hello","browser":"Brave","version":"1.0.0"}"#), Some(FromExt::Hello { browser: "Brave".into() }));
        assert_eq!(parse(br#"{"t":"gone","id":"42"}"#), Some(FromExt::Gone { id: "42".into() }));
        // Tipos novos (de uma extensão mais nova) são ignorados; lixo não passa.
        assert_eq!(parse(br#"{"t":"algo-novo","x":1}"#), Some(FromExt::Other));
        assert_eq!(parse(b"nada"), None);
        assert_eq!(parse(br#"{"t":"player"}"#), None);
        assert_eq!(repeat_code("NONE"), Some(0));
        assert_eq!(repeat_code("ONE"), Some(1));
        assert_eq!(repeat_code("ALL"), Some(2));
        assert_eq!(repeat_code("?"), None);
        let cmd: serde_json::Value = serde_json::from_slice(&command_body("7", "repeat")).unwrap();
        assert_eq!(cmd, serde_json::json!({"t":"cmd","id":"7","cmd":"repeat"}));
    }

    fn player(id: &str, title: &str, playing: bool, shuffle: Option<bool>) -> FromExt {
        FromExt::Player(PlayerMsg { id: id.into(), title: title.into(), artist: String::new(), playing, shuffle, repeat: shuffle.map(|_| "ALL".into()), art: String::new() })
    }

    #[test]
    fn aba_da_sessao() {
        let mut b = Book::new();
        assert!(!b.apply(1, FromExt::Hello { browser: "Brave".into() }));
        assert!(b.apply(1, player("10", "Thanatos", false, Some(true))));
        // Mesma coisa de novo: nada mudou.
        assert!(!b.apply(1, player("10", "Thanatos", false, Some(true))));
        let p = b.pick("Brave._crx_cinhimbnkkghhklpknlkffjgod", "Thanatos").unwrap();
        assert_eq!((p.id.as_str(), p.shuffle, p.repeat), ("10", Some(true), Some(2)));
        assert!(b.pick("Brave", "thanatos").is_some());
        // Outra música, outro navegador ou um app que não é navegador: não.
        assert!(b.pick("Brave", "Midnight City").is_none());
        assert!(b.pick("MSEdge", "Thanatos").is_none());
        assert!(b.pick("Spotify.exe", "Thanatos").is_none());
        // Duas abas com a mesma música: a que toca ganha, depois a mais recente.
        b.apply(1, player("11", "Thanatos", true, Some(false)));
        assert_eq!(b.pick("Brave", "Thanatos").unwrap().id, "11");
        b.apply(1, player("11", "Thanatos", false, Some(false)));
        b.apply(1, player("10", "Thanatos", false, Some(true)));
        assert_eq!(b.pick("Brave", "Thanatos").unwrap().id, "11");
        b.apply(1, player("10", "Thanatos", false, Some(false)));
        assert_eq!(b.pick("Brave", "Thanatos").unwrap().id, "10");
        // Página sem os atributos: não vale (volta ao caminho antigo).
        b.apply(1, player("10", "Thanatos", false, None));
        b.apply(1, player("11", "Thanatos", false, None));
        assert!(b.pick("Brave", "Thanatos").is_none());
        assert!(b.any_from("Brave"));
        assert!(!b.any_from("MSEdge"));
        // A aba fechou; depois o navegador.
        b.apply(1, player("10", "Thanatos", true, Some(true)));
        assert!(b.apply(1, FromExt::Gone { id: "10".into() }));
        assert!(b.pick("Brave", "Thanatos").is_none());
        b.apply(2, FromExt::Hello { browser: "Edge".into() });
        b.apply(2, player("5", "Thanatos", true, Some(true)));
        assert_eq!(b.pick("MSEdge", "Thanatos").unwrap().client, 2);
        assert!(b.pick("Brave", "Thanatos").is_none());
        assert!(b.remove_client(2));
        assert!(b.pick("MSEdge", "Thanatos").is_none());
    }

    #[test]
    fn navegador_da_sessao() {
        assert_eq!(browser_key("Brave._crx_cinhimbnkkghhklpknlkffjgod"), Some("brave"));
        assert_eq!(browser_key("MSEdge"), Some("edge"));
        assert_eq!(browser_key("Edge"), Some("edge"));
        assert_eq!(browser_key("Chrome"), Some("chrome"));
        assert_eq!(browser_key("Chromium"), Some("chrome"));
        assert_eq!(browser_key("Spotify.exe"), None);
        let m: serde_json::Value = serde_json::from_str(&host_manifest(std::path::Path::new(r"C:\Apps\Taskbeat\Taskbeat.exe"))).unwrap();
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(m["path"], r"C:\Apps\Taskbeat\Taskbeat.exe");
        assert_eq!(m["allowed_origins"][0], "chrome-extension://abofmfclmpmmbgecemcglhcklnmcapfh/");
        // Sem a loja, o botão leva às instruções no repositório.
        assert_eq!(install_url(), "https://github.com/Lu1sGabriel/taskbeat/tree/main/extension#readme");
    }
}
