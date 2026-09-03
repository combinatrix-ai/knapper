//! The local control plane for the optional Chrome external-reference bridge.
//!
//! This module contains only the wire contracts and the user-owned runtime
//! directory rules.  It deliberately has no provider access: the native host
//! invokes the normal `knapper resolve` binary after Chrome has re-checked the
//! user-selected tab and text control. A resolved value is never part of a client
//! request or response; it is carried only in the Chrome-owned native
//! messaging pipe.

#![allow(dead_code)]

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const SOCKET_FILENAME: &str = "knapper-chrome.sock";
pub const HOST_NAME: &str = "com.knapper.chrome";
pub const MAX_FRAME: usize = 1024 * 1024;
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const MAX_TIMEOUT_MS: u64 = 120_000;

/// The only request the Codex-facing client can submit.  In particular,
/// there is no target selector and no value field: the extension uses the exact
/// text control the user selected while accepting mode is on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClientRequest {
    pub kind: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_origin: Option<String>,
    #[serde(default)]
    pub timeout_ms: u64,
}

/// Safe, model-visible result.  No provider output, command output, URI
/// locator, or page text belongs in this structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClientResponse {
    pub status: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

impl ClientResponse {
    pub fn ok(request_id: String, origin: String) -> Self {
        Self {
            status: "filled".into(),
            request_id,
            code: None,
            origin: Some(origin),
            mode: None,
        }
    }

    pub fn error(request_id: String, code: &str) -> Self {
        Self {
            status: "error".into(),
            request_id,
            code: Some(code.into()),
            origin: None,
            mode: None,
        }
    }

    pub fn state(request_id: String, mode: &str, origin: Option<String>) -> Self {
        Self {
            status: "ok".into(),
            request_id,
            code: None,
            origin,
            mode: Some(mode.into()),
        }
    }
}

/// Native Messaging messages.  `value` exists in exactly one direction and
/// is private to the Chrome-owned pipe.  Never serialize this enum for the
/// Codex-facing Unix socket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NativeMessage {
    Hello,
    HelloAck,
    Selecting {
        tab_id: i64,
        url: String,
        origin: String,
    },
    Arm {
        tab_id: i64,
        url: String,
        origin: String,
    },
    Armed,
    PrepareFill {
        request_id: String,
        reference: String,
        expected_origin: String,
        expected_url: String,
        timeout_ms: u64,
    },
    TargetReady {
        request_id: String,
    },
    TargetRejected {
        request_id: String,
        code: String,
    },
    Fill {
        request_id: String,
        value: String,
    },
    Filled {
        request_id: String,
    },
    FillRejected {
        request_id: String,
        code: String,
    },
}

/// Read one Native Messaging frame. Chrome specifies a native-endian u32
/// byte length followed by UTF-8 JSON. The same bounded framing is used for
/// the local Unix socket so a client cannot smuggle delimiters or an
/// unbounded body into the host.
pub fn read_frame<R: Read>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let size = u32::from_ne_bytes(length) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message frame is empty or too large",
        ));
    }
    let mut body = vec![0_u8; size];
    reader.read_exact(&mut body)?;
    Ok(body)
}

pub fn write_frame<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message frame is empty or too large",
        ));
    }
    let size = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "message frame is too large"))?;
    writer.write_all(&size.to_ne_bytes())?;
    writer.write_all(body)?;
    writer.flush()
}

pub fn write_json<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let body = serde_json::to_vec(value)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    write_frame(writer, &body)
}

pub fn read_json<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> io::Result<T> {
    let body = read_frame(reader)?;
    serde_json::from_slice(&body)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))
}

/// Generate a correlation id without adding a dependency or accepting an id
/// from the caller.  It is not a credential; its only purpose is matching a
/// single socket request to a single Chrome response.
pub fn request_id() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{:x}-{:x}-{:x}", ticks, std::process::id(), counter)
}

pub fn validate_reference(reference: &str) -> bool {
    if reference.len() > 4096 {
        return false;
    }
    let Some(rest) = reference.strip_prefix("knapper://") else {
        return false;
    };
    let Some((provider, locator)) = rest.split_once('/') else {
        return false;
    };
    let mut provider_bytes = provider.bytes();
    let provider_ok = provider_bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && provider_bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });
    let locator_ok = !locator.is_empty()
        && locator.len() <= 128
        && locator
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        && locator
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    provider_ok && locator_ok
}

/// The bridge accepts web origins only.  The extension performs the
/// authoritative comparison against `location.origin`; this check prevents
/// malformed values and non-web pages entering the protocol in the first
/// place.
pub fn validate_origin(origin: &str) -> bool {
    if origin.len() > 2048
        || origin
            .as_bytes()
            .iter()
            .any(|byte| byte.is_ascii_whitespace())
    {
        return false;
    }
    let Some(rest) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty()
        && !rest.contains(['/', '?', '#', '@', '[', ']'])
        && rest.chars().all(|ch| ch.is_ascii_graphic())
}

/// Check that a page URL belongs to the exact web origin.  This is kept
/// deliberately small (the extension repeats the check with the browser's
/// URL parser): a prefix such as `https://example.test.attacker` must not be
/// accepted as `https://example.test`.
pub fn url_matches_origin(url: &str, origin: &str) -> bool {
    if !validate_origin(origin)
        || url.len() > 16 * 1024
        || url
            .as_bytes()
            .iter()
            .any(|byte| byte.is_ascii_whitespace() || *byte == 0)
    {
        return false;
    }
    let Some(rest) = url.strip_prefix(origin) else {
        return false;
    };
    rest.is_empty() || rest.starts_with(['/', '?', '#'])
}

pub fn validate_timeout(timeout_ms: u64) -> bool {
    (1..=MAX_TIMEOUT_MS).contains(&timeout_ms)
}

#[cfg(unix)]
pub fn runtime_dir() -> io::Result<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    // Never tighten permissions on XDG_RUNTIME_DIR or the provider config
    // directory itself. The bridge owns only this dedicated child directory.
    let candidate = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|path| std::path::Path::new(path).is_absolute())
        .map(PathBuf::from)
        .map(|path| path.join("knapper"))
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|path| std::path::Path::new(path).is_absolute())
                .map(PathBuf::from)
                .map(|path| path.join("knapper/runtime"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".config/knapper/runtime"))
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no user runtime directory"))?;

    std::fs::create_dir_all(&candidate)?;
    std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o700))?;
    let metadata = std::fs::symlink_metadata(&candidate)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory is not a user-owned directory",
        ));
    }
    // Refuse group/world access even if chmod failed to tighten an existing
    // directory, rather than silently placing a socket in a shared location.
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory is accessible by another user",
        ));
    }
    Ok(candidate)
}

#[cfg(unix)]
pub fn socket_path() -> io::Result<PathBuf> {
    let path = runtime_dir()?.join(SOCKET_FILENAME);
    if path.as_os_str().len() > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix socket path is too long",
        ));
    }
    Ok(path)
}

#[cfg(not(unix))]
pub fn unsupported() -> ! {
    eprintln!("knapper Chrome bridge is currently supported on macOS/Linux only");
    std::process::exit(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn native_messages_use_native_endian_length_prefix() {
        let body = br#"{"type":"hello"}"#;
        let mut encoded = Vec::new();
        write_frame(&mut encoded, body).unwrap();
        assert_eq!(&encoded[..4], &(body.len() as u32).to_ne_bytes());
        assert_eq!(read_frame(&mut Cursor::new(encoded)).unwrap(), body);
    }

    #[test]
    fn framing_rejects_empty_and_oversized_messages() {
        let mut empty = (0_u32).to_ne_bytes().to_vec();
        assert!(read_frame(&mut Cursor::new(&mut empty)).is_err());

        let too_large = ((MAX_FRAME + 1) as u32).to_ne_bytes().to_vec();
        assert!(read_frame(&mut Cursor::new(too_large)).is_err());
        assert!(write_frame(&mut Vec::new(), &vec![0_u8; MAX_FRAME + 1]).is_err());
    }

    #[test]
    fn status_json_has_no_value_field_or_secret() {
        let response = ClientResponse::error("request-1".into(), "target_rejected");
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            json,
            r#"{"status":"error","request_id":"request-1","code":"target_rejected"}"#
        );
        assert!(!json.contains("value"));
        assert!(!json.contains("secret"));

        let fill = NativeMessage::Fill {
            request_id: "request-1".into(),
            value: "secret-value".into(),
        };
        let native_json = serde_json::to_string(&fill).unwrap();
        assert!(native_json.contains("secret-value"));
        assert!(!json.contains("secret-value"));
    }

    #[test]
    fn protocol_inputs_are_bounded_and_web_only() {
        assert!(validate_reference("knapper://personal/address.home"));
        assert!(!validate_reference("https://example.test/value"));
        assert!(!validate_reference("knapper://personal/\0secret"));
        assert!(validate_origin("https://example.test:8443"));
        assert!(!validate_origin("https://example.test/path"));
        assert!(!validate_origin("file://example.test"));
        assert!(url_matches_origin(
            "https://example.test/path",
            "https://example.test"
        ));
        assert!(!url_matches_origin(
            "https://example.test.attacker/path",
            "https://example.test"
        ));
        assert!(!url_matches_origin(
            "https://example.test/path",
            "https://other.test"
        ));
        assert!(validate_timeout(1));
        assert!(validate_timeout(MAX_TIMEOUT_MS));
        assert!(!validate_timeout(0));
        assert!(!validate_timeout(MAX_TIMEOUT_MS + 1));
    }

    #[test]
    fn client_request_can_omit_optional_origin_and_reject_unknown_fields() {
        let request = ClientRequest {
            kind: "fill".into(),
            request_id: "request-1".into(),
            reference: "knapper://personal/address.home".into(),
            expected_origin: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains("expected_origin"));
        assert_eq!(
            serde_json::from_str::<ClientRequest>(&json).unwrap(),
            request
        );
        assert!(serde_json::from_str::<ClientResponse>(
            r#"{"status":"filled","request_id":"request-1","value":"secret"}"#
        )
        .is_err());
    }
}
