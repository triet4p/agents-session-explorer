//! Minimal HTTP transport for one typed API call.
//!
//! Windows uses WinHTTP, so HTTPS, system trust and proxy handling come from
//! the OS and no TLS library is linked. Other platforms support the validated
//! loopback `http` path only and report HTTPS as unavailable; there is no
//! plaintext fallback for a remote host.
//!
//! Both implementations disable or refuse redirects and return the response
//! body without interpreting it. Credentials are added as one header at send
//! time and are never part of a request description, report or error.

use std::time::Duration;

use crate::api::ApiError;
use crate::secret::ApiKey;
use crate::wire::WireRequest;

pub struct HttpOutcome {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Wrapper that scrubs the formatted auth header after use. The `ApiKey`
/// itself is still the authoritative secret and scrubs its own copy.
struct SecretHeader(String);

impl SecretHeader {
    fn new(value: String) -> Self {
        SecretHeader(value)
    }
    fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for SecretHeader {
    fn drop(&mut self) {
        // Best-effort in-process scrub of the formatted header copy.
        unsafe {
            for byte in self.0.as_bytes_mut() {
                std::ptr::write_volatile(byte, 0);
            }
        }
    }
}

pub fn send(
    request: &WireRequest,
    key: &ApiKey,
    timeout: Duration,
) -> Result<HttpOutcome, ApiError> {
    let timeout = Duration::from_millis(timeout.as_millis().max(1) as u64);
    let header = SecretHeader::new(request.auth.header_value(key.expose()));
    imp::send(request, header.expose(), timeout)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::time::Duration;

    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
        WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY,
        WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_DISABLE_FEATURE,
        WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    };

    use crate::api::{ApiError, MAX_RESPONSE_BYTES};
    use crate::wire::WireRequest;

    use super::HttpOutcome;

    struct Handle(*mut c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        std::ffi::OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn last_error() -> i32 {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(0)
    }

    fn check(label: &'static str, ok: i32) -> Result<(), ApiError> {
        if ok == 0 {
            Err(ApiError::Failed(format!("{label}:{}", last_error())))
        } else {
            Ok(())
        }
    }

    pub(super) fn send(
        request: &WireRequest,
        auth_header: &str,
        timeout: Duration,
    ) -> Result<HttpOutcome, ApiError> {
        let agent = wide("ase-llm/1");
        let access_type = if crate::url::is_loopback_host(&request.host) {
            WINHTTP_ACCESS_TYPE_NO_PROXY
        } else {
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY
        };
        let session = Handle(unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                access_type,
                std::ptr::null(),
                std::ptr::null(),
                0,
            )
        });
        if session.0.is_null() {
            return Err(ApiError::Failed(format!("winhttp_open:{}", last_error())));
        }
        let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
        check(
            "winhttp_timeouts",
            unsafe { WinHttpSetTimeouts(session.0, millis, millis, millis, millis) },
        )?;

        let host = wide(&request.host);
        let port: u16 = request.port;
        let connection = Handle(unsafe { WinHttpConnect(session.0, host.as_ptr(), port, 0) });
        if connection.0.is_null() {
            return Err(ApiError::Failed(format!(
                "winhttp_connect:{}",
                last_error()
            )));
        }

        let verb = wide("POST");
        let path = wide(&request.path_and_query);
        let mut flags: u32 = 0;
        if request.scheme == "https" {
            flags |= WINHTTP_FLAG_SECURE;
        }
        let handle = Handle(unsafe {
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                flags,
            )
        });
        if handle.0.is_null() {
            return Err(ApiError::Failed(format!(
                "winhttp_open_request:{}",
                last_error()
            )));
        }

        let disable: u32 = WINHTTP_DISABLE_REDIRECTS;
        check(
            "winhttp_disable_redirects",
            unsafe {
                WinHttpSetOption(
                    handle.0,
                    WINHTTP_OPTION_DISABLE_FEATURE,
                    &disable as *const u32 as *const c_void,
                    std::mem::size_of::<u32>() as u32,
                )
            },
        )?;
        let policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        check(
            "winhttp_redirect_policy",
            unsafe {
                WinHttpSetOption(
                    handle.0,
                    WINHTTP_OPTION_REDIRECT_POLICY,
                    &policy as *const u32 as *const c_void,
                    std::mem::size_of::<u32>() as u32,
                )
            },
        )?;

        let mut header_text = String::new();
        for (name, value) in &request.headers {
            header_text.push_str(name);
            header_text.push_str(": ");
            header_text.push_str(value);
            header_text.push_str("\r\n");
        }
        header_text.push_str(request.auth.header_name());
        header_text.push_str(": ");
        header_text.push_str(auth_header);
        header_text.push_str("\r\n");
        let headers = wide(&header_text);
        let headers_len = (headers.len() - 1) as u32;

        let body = &request.body;
        let (body_ptr, body_len) = if body.is_empty() {
            (std::ptr::null(), 0u32)
        } else {
            (body.as_ptr() as *const c_void, body.len() as u32)
        };
        check(
            "winhttp_send",
            unsafe {
                WinHttpSendRequest(
                    handle.0,
                    headers.as_ptr(),
                    headers_len,
                    body_ptr,
                    body_len,
                    body_len,
                    0,
                )
            },
        )?;
        // ERROR_WINHTTP_REDIRECT_FAILED: the request was redirected and
        // redirects are disabled. Report the refusal without following it.
        let received = unsafe { WinHttpReceiveResponse(handle.0, std::ptr::null_mut()) };
        if received == 0 {
            let code = last_error();
            if code == 12112 {
                return Err(ApiError::Rejected("redirect_refused".into()));
            }
            return Err(ApiError::Failed(format!("winhttp_receive:{code}")));
        }

        let mut status: u32 = 0;
        let mut status_len: u32 = std::mem::size_of::<u32>() as u32;
        check(
            "winhttp_status",
            unsafe {
                WinHttpQueryHeaders(
                    handle.0,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    std::ptr::null(),
                    &mut status as *mut u32 as *mut c_void,
                    &mut status_len,
                    std::ptr::null_mut(),
                )
            },
        )?;

        let mut body_out: Vec<u8> = Vec::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let mut read: u32 = 0;
            let ok = unsafe {
                WinHttpReadData(
                    handle.0,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len() as u32,
                    &mut read,
                )
            };
            if ok == 0 {
                // ERROR_HANDLE_EOF marks the end of a response without an
                // explicit zero-length read.
                if last_error() == 38 {
                    break;
                }
                return Err(ApiError::Failed(format!(
                    "winhttp_read:{}",
                    last_error()
                )));
            }
            if read == 0 {
                break;
            }
            body_out.extend_from_slice(&buffer[..read as usize]);
            if body_out.len() > MAX_RESPONSE_BYTES {
                return Err(ApiError::Rejected("response_too_large".into()));
            }
        }
        Ok(HttpOutcome {
            status: status as u16,
            body: body_out,
        })
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
    use std::time::Duration;

    use crate::api::{ApiError, MAX_RESPONSE_BYTES};
    use crate::wire::WireRequest;

    use super::HttpOutcome;

    fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, ApiError> {
        let mut out = Vec::new();
        let mut cursor = 0usize;
        loop {
            let line_end = data[cursor..]
                .windows(2)
                .position(|window| window == b"\r\n")
                .map(|position| cursor + position)
                .ok_or_else(|| ApiError::Rejected("chunked_missing_size".into()))?;
            let size_text = String::from_utf8_lossy(&data[cursor..line_end]);
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| ApiError::Rejected("chunked_bad_size".into()))?;
            cursor = line_end + 2;
            if size == 0 {
                return Ok(out);
            }
            if cursor + size > data.len() {
                return Err(ApiError::Rejected("chunked_truncated".into()));
            }
            out.extend_from_slice(&data[cursor..cursor + size]);
            cursor += size;
            if data.get(cursor..cursor + 2) != Some(b"\r\n") {
                return Err(ApiError::Rejected("chunked_missing_terminator".into()));
            }
            cursor += 2;
            if out.len() > MAX_RESPONSE_BYTES {
                return Err(ApiError::Rejected("response_too_large".into()));
            }
        }
    }

    pub(super) fn send(
        request: &WireRequest,
        auth_header: &str,
        timeout: Duration,
    ) -> Result<HttpOutcome, ApiError> {
        if request.scheme != "http" {
            return Err(ApiError::Unavailable(
                "https_transport_unavailable_on_platform".into(),
            ));
        }
        let address = format!("{}:{}", request.host, request.port);
        let addresses: Vec<SocketAddr> = address
            .to_socket_addrs()
            .map_err(|_| ApiError::Failed("resolve_host".into()))?
            .collect();
        let mut stream = None;
        for candidate in addresses {
            if let Ok(connection) = TcpStream::connect_timeout(&candidate, timeout) {
                stream = Some(connection);
                break;
            }
        }
        let mut stream = stream.ok_or_else(|| ApiError::Failed("connect".into()))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| ApiError::Failed("read_timeout".into()))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|_| ApiError::Failed("write_timeout".into()))?;

        let host_header = if (request.scheme == "http" && request.port == 80)
            || (request.scheme == "https" && request.port == 443)
        {
            request.host.clone()
        } else {
            format!("{}:{}", request.host, request.port)
        };
        let mut head = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\n",
            request.path_and_query, host_header
        );
        for (name, value) in &request.headers {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str(request.auth.header_name());
        head.push_str(": ");
        head.push_str(auth_header);
        head.push_str("\r\n");
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            request.body.len()
        ));
        stream
            .write_all(head.as_bytes())
            .map_err(|_| ApiError::Failed("write".into()))?;
        stream
            .write_all(&request.body)
            .map_err(|_| ApiError::Failed("write_body".into()))?;

        let mut raw = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    raw.extend_from_slice(&buffer[..size]);
                    if raw.len() > MAX_RESPONSE_BYTES {
                        return Err(ApiError::Rejected("response_too_large".into()));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(ApiError::Failed("read_timeout".into()))
                }
                Err(_) => return Err(ApiError::Failed("read".into())),
            }
        }
        let split = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| ApiError::Rejected("response_headers_missing".into()))?;
        let head = String::from_utf8_lossy(&raw[..split]);
        let body = &raw[split + 4..];
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| ApiError::Rejected("status_unparsable".into()))?;
        let mut chunked = false;
        let mut content_length: Option<usize> = None;
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                let value = value.trim();
                if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
                    chunked = true;
                }
                if name == "content-length" {
                    content_length = value.parse::<usize>().ok();
                }
            }
        }
        let body = if chunked {
            decode_chunked(body)?
        } else if let Some(length) = content_length {
            if body.len() < length {
                return Err(ApiError::Rejected("response_truncated".into()));
            }
            body[..length].to_vec()
        } else {
            body.to_vec()
        };
        Ok(HttpOutcome { status, body })
    }
}
