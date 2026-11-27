//! GET por HTTPS com o WinHTTP do próprio Windows (sem bibliotecas de rede).
//! Usa o proxy do sistema, aceita gzip e desiste depois de alguns segundos.

use super::fetch::Response;
use windows::Win32::Networking::WinHttp::*;
use windows::core::{HSTRING, PCWSTR, w};

/// Maior resposta aceita (uma letra tem poucos KB; uma busca, algumas dezenas).
const MAX_BODY: usize = 2 << 20;

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

pub fn get(host: &str, path: &str, user_agent: &str) -> Result<Response, String> {
    unsafe {
        let session = open(WinHttpOpen(&HSTRING::from(user_agent), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0), "WinHttpOpen")?;
        let _ = WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 8000);
        let flags = (WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE).to_le_bytes();
        let _ = WinHttpSetOption(Some(session.0), WINHTTP_OPTION_DECOMPRESSION, Some(&flags));
        let conn = open(WinHttpConnect(session.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0), "WinHttpConnect")?;
        let req = open(WinHttpOpenRequest(conn.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE), "WinHttpOpenRequest")?;
        let headers: Vec<u16> = "Accept: application/json\r\n".encode_utf16().collect();
        WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0).map_err(|e| format!("envio: {e}"))?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|e| format!("resposta: {e}"))?;
        let status = header_u32(&req, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0) as u16;
        let retry_after = header_u32(&req, WINHTTP_QUERY_RETRY_AFTER).map(|v| v as u64);
        let mut body = Vec::new();
        loop {
            let mut avail = 0u32;
            WinHttpQueryDataAvailable(req.0, &mut avail).map_err(|e| format!("leitura: {e}"))?;
            if avail == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + avail as usize, 0u8);
            let mut read = 0u32;
            WinHttpReadData(req.0, body[start..].as_mut_ptr() as *mut _, avail, &mut read).map_err(|e| format!("leitura: {e}"))?;
            body.truncate(start + read as usize);
            if body.len() > MAX_BODY {
                return Err("resposta grande demais".into());
            }
        }
        Ok(Response { status, body: String::from_utf8_lossy(&body).into_owned(), retry_after })
    }
}

#[cfg(test)]
mod tests {
    /// Pede uma letra de verdade ao LRCLIB (precisa de internet):
    /// `cargo test lrclib_ao_vivo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lrclib_ao_vivo() {
        let ua = super::super::lrclib::user_agent();
        let r = super::get("lrclib.net", "/api/get?artist_name=Coldplay&track_name=Yellow&duration=269", &ua).unwrap();
        println!("status {} ({} bytes)", r.status, r.body.len());
        assert_eq!(r.status, 200);
        let rec = super::super::lrclib::parse_record(&r.body).unwrap();
        assert!(rec.lyrics().is_text());
        let r = super::get("lrclib.net", "/api/get?artist_name=Ningu%C3%A9m&track_name=N%C3%A3o%20existe%20xyz%20123", &ua).unwrap();
        assert_eq!(r.status, 404);
    }
}
