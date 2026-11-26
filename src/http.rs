//! GET por HTTPS sem bibliotecas de rede: WinHTTP no Windows, o `curl` do
//! sistema no Mac. Pequeno de propósito e comum a quem precisar de rede (o
//! atualizador; a letra das músicas também pode usar).
//!
//! - Só `https://`. `http://` é aceito apenas para o próprio PC (127.0.0.1,
//!   localhost), nos testes com um servidor local.
//! - Segue redirecionamentos (os downloads das releases do GitHub passam por
//!   outro domínio), nunca de HTTPS para HTTP.
//! - Corpo em bytes, com limite de tamanho: passou do limite, desiste.
//! - Usa o proxy do sistema e desiste depois de alguns segundos sem resposta.

/// Resposta: código HTTP e corpo.
#[derive(Clone, Debug, Default)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Corpo como texto (UTF-8; bytes inválidos viram "�").
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// User-Agent padrão (o GitHub recusa pedidos sem um).
pub fn user_agent() -> String {
    format!("Taskbeat/{} (+https://github.com/Lu1sGabriel/taskbeat)", env!("CARGO_PKG_VERSION"))
}

/// Endereço já separado em partes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Url<'a> {
    pub secure: bool,
    pub host: &'a str,
    pub port: u16,
    /// Caminho com a consulta (`/a/b?c=d`); "/" se vier vazio.
    pub path: &'a str,
}

fn loopback(host: &str) -> bool {
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost")
}

/// Separa `https://host[:porta]/caminho`. Recusa usuário/senha no endereço,
/// IPv6 entre colchetes e `http://` fora do próprio PC.
pub(crate) fn parse_url(url: &str) -> Result<Url<'_>, String> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("endereço sem https: {url}"));
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.is_empty() || authority.contains(['@', '[', ']', ' ', '\\']) {
        return Err(format!("endereço inválido: {url}"));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| format!("porta inválida: {url}"))?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    if host.is_empty() || port == 0 {
        return Err(format!("endereço inválido: {url}"));
    }
    if !secure && !loopback(host) {
        return Err(format!("só HTTPS: {url}"));
    }
    let path = path.split('#').next().unwrap_or("");
    let path = if path.is_empty() { "/" } else { path };
    if path.chars().any(|c| c.is_control() || c == ' ') {
        return Err(format!("endereço inválido: {url}"));
    }
    Ok(Url { secure, host, port, path })
}

/// Cabeçalhos aceitos: nome e valor sem quebras de linha (não dá para injetar
/// outro cabeçalho pelo valor).
fn header_ok(name: &str, value: &str) -> bool {
    !name.is_empty() && !name.contains([':', '\r', '\n']) && !value.contains(['\r', '\n'])
}

/// GET em `url` com os cabeçalhos dados (um `User-Agent` neles substitui o
/// padrão). Corpo maior que `max_bytes` é erro.
pub fn get(url: &str, headers: &[(&str, &str)], max_bytes: usize) -> Result<Response, String> {
    let u = parse_url(url)?;
    if let Some((n, _)) = headers.iter().find(|(n, v)| !header_ok(n, v)) {
        return Err(format!("cabeçalho inválido: {n}"));
    }
    let ua = headers.iter().find(|(n, _)| n.eq_ignore_ascii_case("user-agent")).map(|(_, v)| v.to_string()).unwrap_or_else(user_agent);
    let rest: Vec<(&str, &str)> = headers.iter().copied().filter(|(n, _)| !n.eq_ignore_ascii_case("user-agent")).collect();
    imp::get(&u, &ua, &rest, max_bytes)
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod imp {
    use super::{Response, Url};
    use windows::Win32::Networking::WinHttp::*;
    use windows::core::{HSTRING, PCWSTR, w};

    struct Handle(*mut core::ffi::c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let _ = WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn open(h: *mut core::ffi::c_void, what: &str) -> Result<Handle, String> {
        if h.is_null() { Err(format!("{what}: {}", windows::core::Error::from_thread())) } else { Ok(Handle(h)) }
    }

    fn header_u32(req: &Handle, info: u32) -> Option<u32> {
        let mut v = 0u32;
        let mut len = std::mem::size_of::<u32>() as u32;
        let mut idx = 0u32;
        unsafe { WinHttpQueryHeaders(req.0, info | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut v as *mut u32 as *mut _), &mut len, &mut idx).ok().map(|_| v) }
    }

    pub fn get(u: &Url, ua: &str, headers: &[(&str, &str)], max: usize) -> Result<Response, String> {
        unsafe {
            let session = open(WinHttpOpen(&HSTRING::from(ua), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0), "WinHttpOpen")?;
            // Resolver, conectar e enviar: 10 s; cada leitura: 30 s.
            let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 30_000);
            let flags = (WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE).to_le_bytes();
            let _ = WinHttpSetOption(Some(session.0), WINHTTP_OPTION_DECOMPRESSION, Some(&flags));
            let conn = open(WinHttpConnect(session.0, &HSTRING::from(u.host), u.port, 0), "WinHttpConnect")?;
            let secure = if u.secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
            let req = open(WinHttpOpenRequest(conn.0, w!("GET"), &HSTRING::from(u.path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), secure), "WinHttpOpenRequest")?;
            let extra: String = headers.iter().map(|(n, v)| format!("{n}: {v}\r\n")).collect();
            let extra: Vec<u16> = extra.encode_utf16().collect();
            WinHttpSendRequest(req.0, (!extra.is_empty()).then_some(&extra[..]), None, 0, 0, 0).map_err(|e| format!("envio: {e}"))?;
            WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|e| format!("resposta: {e}"))?;
            let status = header_u32(&req, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0) as u16;
            if header_u32(&req, WINHTTP_QUERY_CONTENT_LENGTH).is_some_and(|n| n as usize > max) {
                return Err("resposta grande demais".into());
            }
            let mut body = Vec::new();
            loop {
                let mut avail = 0u32;
                WinHttpQueryDataAvailable(req.0, &mut avail).map_err(|e| format!("leitura: {e}"))?;
                if avail == 0 {
                    break;
                }
                if body.len() + avail as usize > max {
                    return Err("resposta grande demais".into());
                }
                let start = body.len();
                body.resize(start + avail as usize, 0u8);
                let mut read = 0u32;
                WinHttpReadData(req.0, body[start..].as_mut_ptr() as *mut _, avail, &mut read).map_err(|e| format!("leitura: {e}"))?;
                body.truncate(start + read as usize);
            }
            Ok(Response { status, body })
        }
    }
}

// ---------------------------------------------------------------- Mac

#[cfg(target_os = "macos")]
mod imp {
    use super::{Response, Url};
    use std::process::Command;

    /// O `curl` que vem com o macOS (7.79 no macOS 12): `%{stderr}` no `-w`
    /// manda o código HTTP para a saída de erro e o corpo fica intacto.
    pub fn get(u: &Url, ua: &str, headers: &[(&str, &str)], max: usize) -> Result<Response, String> {
        let url = format!("{}://{}:{}{}", if u.secure { "https" } else { "http" }, u.host, u.port, u.path);
        let proto = if u.secure { "=https" } else { "=http,https" };
        let mut cmd = Command::new("/usr/bin/curl");
        cmd.args(["-sS", "-L", "--max-redirs", "8", "--proto", proto, "--proto-redir", "=https", "--compressed"])
            .args(["--connect-timeout", "10", "--max-time", "180", "--max-filesize", &max.to_string()])
            .args(["-A", ua, "-w", "%{stderr}\n%{http_code}"]);
        for (n, v) in headers {
            cmd.arg("-H").arg(format!("{n}: {v}"));
        }
        let out = cmd.arg("--").arg(&url).output().map_err(|e| format!("curl: {e}"))?;
        let err = String::from_utf8_lossy(&out.stderr);
        let status = err.lines().last().and_then(|l| l.trim().parse::<u16>().ok()).unwrap_or(0);
        if !out.status.success() || status == 0 {
            let msg = err.lines().find(|l| !l.trim().is_empty() && l.trim().parse::<u16>().is_err()).unwrap_or("falhou");
            return Err(format!("curl: {}", msg.trim()));
        }
        if out.stdout.len() > max {
            return Err("resposta grande demais".into());
        }
        Ok(Response { status, body: out.stdout })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separa_enderecos() {
        let u = parse_url("https://api.github.com/repos/a/b/releases/latest").unwrap();
        assert_eq!(u, Url { secure: true, host: "api.github.com", port: 443, path: "/repos/a/b/releases/latest" });
        let u = parse_url("https://example.com").unwrap();
        assert_eq!((u.host, u.path), ("example.com", "/"));
        let u = parse_url("https://example.com?x=1#frag").unwrap();
        assert_eq!(u.path, "?x=1");
        let u = parse_url("http://127.0.0.1:8123/r?per_page=5").unwrap();
        assert_eq!(u, Url { secure: false, host: "127.0.0.1", port: 8123, path: "/r?per_page=5" });
        assert!(parse_url("http://localhost:9/").is_ok());
    }

    #[test]
    fn recusa_enderecos_perigosos() {
        for bad in [
            "http://github.com/x",
            "ftp://github.com/x",
            "github.com/x",
            "https://user:pw@github.com/x",
            "https://[::1]/x",
            "https://github.com:0/x",
            "https://github.com:99999/x",
            "https:///x",
            "https://github.com/a b",
            "https://github.com/a\r\nX: y",
        ] {
            assert!(parse_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cabecalhos_sem_quebra_de_linha() {
        assert!(header_ok("Accept", "application/json"));
        assert!(!header_ok("Accept", "a\r\nX-Evil: 1"));
        assert!(!header_ok("A:B", "c"));
        assert!(get("https://example.com", &[("X", "a\nb")], 10).is_err());
    }

    /// Pede a última release ao GitHub de verdade (precisa de internet):
    /// `cargo test http_ao_vivo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn http_ao_vivo() {
        let r = get("https://api.github.com/repos/Lu1sGabriel/taskbeat/releases/latest", &[("Accept", "application/vnd.github+json")], 1 << 20).unwrap();
        println!("status {} ({} bytes)", r.status, r.body.len());
        assert_eq!(r.status, 200);
        assert!(r.text().contains("tag_name"));
        // Download de uma release: o GitHub redireciona para outro domínio.
        let url = "https://github.com/Lu1sGabriel/taskbeat/releases/download/v1.3.0/Taskbeat-Setup-1.3.0.exe";
        let r = get(url, &[("Accept", "application/octet-stream")], 64 << 20).unwrap();
        println!("download: status {} ({} bytes)", r.status, r.body.len());
        assert_eq!((r.status, r.body.len()), (200, 2_544_640));
        assert_eq!(&r.body[..2], b"MZ");
        // Limite de tamanho respeitado.
        assert!(get(url, &[], 1 << 20).is_err());
    }
}
