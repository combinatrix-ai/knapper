//! Codex-facing client for the optional Chrome external-reference bridge.
//!
//! The only request fields accepted here are a `knapper://` reference, an
//! expected web origin, and a bounded timeout.  The response is status-only;
//! the resolved value never crosses this Unix socket.

#[path = "../chrome_bridge.rs"]
mod chrome_bridge;

#[cfg(unix)]
mod unix_client {
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    use clap::{error::ErrorKind, Parser};

    use super::chrome_bridge::{
        read_json, request_id, socket_path, validate_origin, validate_reference, validate_timeout,
        write_json, ClientRequest, ClientResponse, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS,
    };

    #[derive(Debug, Parser)]
    #[command(
        name = "knapper-chrome-client",
        version,
        about = "Fill the user-selected Chrome text control through a local bridge"
    )]
    struct Cli {
        /// A knapper:// reference. Its value never enters this process's
        /// stdout or stderr.
        reference: Option<String>,
        /// Exact expected origin, such as https://example.com:8443. If
        /// omitted, use the origin armed by the extension action.
        #[arg(long = "expected-origin")]
        expected_origin: Option<String>,
        /// Whole-operation timeout in seconds (default 30, max 120).
        #[arg(long = "timeout")]
        timeout: Option<f64>,
    }

    pub fn run() -> Result<(), String> {
        let request_id = request_id();
        let cli = match Cli::try_parse() {
            Ok(cli) => cli,
            Err(error) => {
                // Clap's normal error renderer echoes command-line details.
                // The bridge contract promises one JSON result on every
                // operational path, so keep malformed input generic too.
                if matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) {
                    println!("{error}");
                    return Ok(());
                }
                return emit_error(request_id, "invalid_request", "invalid request");
            }
        };

        if cli.reference.as_deref() == Some("status") {
            return run_status(request_id);
        }

        let Some(reference) = cli.reference else {
            return emit_error(
                request_id,
                "invalid_request",
                "a knapper reference is required",
            );
        };
        let timeout_ms = match timeout_ms(cli.timeout.unwrap_or(DEFAULT_TIMEOUT_MS as f64 / 1000.0))
        {
            Ok(timeout_ms) => timeout_ms,
            Err(_) => return emit_error(request_id, "invalid_request", "invalid request"),
        };
        if !validate_reference(&reference)
            || cli
                .expected_origin
                .as_deref()
                .is_some_and(|origin| !validate_origin(origin))
        {
            return emit_error(request_id, "invalid_request", "invalid request");
        }

        let path = match socket_path() {
            Ok(path) => path,
            Err(_) => {
                return emit_error(
                    request_id,
                    "not_connected",
                    "Chrome bridge is not available",
                )
            }
        };
        let mut stream =
            match UnixStream::connect(path) {
                Ok(stream) => stream,
                Err(_) => return emit_error(
                    request_id,
                    "not_connected",
                    "Chrome bridge is not connected; select a text field in the extension first",
                ),
            };
        if stream
            .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
            .is_err()
            || stream
                .set_write_timeout(Some(Duration::from_millis(timeout_ms)))
                .is_err()
        {
            return emit_error(
                request_id,
                "bridge_failed",
                "Chrome bridge request could not be started",
            );
        }

        let request = ClientRequest {
            kind: "fill".into(),
            request_id: request_id.clone(),
            reference,
            expected_origin: cli.expected_origin,
            timeout_ms,
        };
        if write_json(&mut stream, &request).is_err() {
            return emit_error(
                request_id,
                "bridge_failed",
                "Chrome bridge request could not be sent",
            );
        }
        let response: ClientResponse = match read_json(&mut stream) {
            Ok(response) => response,
            Err(_) => {
                return emit_error(
                    request_id,
                    "bridge_failed",
                    "Chrome bridge did not return a safe status",
                )
            }
        };

        if response.request_id != request_id {
            return emit_error(
                request_id,
                "protocol_error",
                "Chrome bridge returned an invalid status",
            );
        }
        if response.status == "filled"
            && response.code.is_none()
            && response.origin.as_deref().is_some_and(validate_origin)
        {
            let body = serde_json::to_string(&ClientResponse::ok(
                request_id.clone(),
                response.origin.expect("origin checked above"),
            ))
            .map_err(|_| "could not encode bridge status".to_string())?;
            println!("{body}");
            return Ok(());
        }

        let code = response
            .code
            .as_deref()
            .filter(|code| safe_code(code))
            .unwrap_or("bridge_failed");
        emit_error(
            request_id,
            code,
            "Chrome bridge did not fill the selected control",
        )
    }

    fn run_status(request_id: String) -> Result<(), String> {
        let path = match socket_path() {
            Ok(path) => path,
            Err(_) => {
                return emit_error(
                    request_id,
                    "not_connected",
                    "Chrome bridge is not available",
                )
            }
        };
        let mut stream = match UnixStream::connect(path) {
            Ok(stream) => stream,
            Err(_) => {
                return emit_error(
                    request_id,
                    "not_connected",
                    "Chrome bridge is not connected",
                )
            }
        };
        // A status request has no provider work. Keep the read bound finite so
        // a host that is shutting down cannot leave the CLI hanging.
        let timeout_ms = DEFAULT_TIMEOUT_MS;
        if stream
            .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
            .is_err()
            || stream
                .set_write_timeout(Some(Duration::from_millis(timeout_ms)))
                .is_err()
        {
            return emit_error(
                request_id,
                "bridge_failed",
                "Chrome bridge status could not be started",
            );
        }

        let request = ClientRequest {
            kind: "status".into(),
            request_id: request_id.clone(),
            reference: String::new(),
            expected_origin: None,
            timeout_ms: 0,
        };
        if write_json(&mut stream, &request).is_err() {
            return emit_error(
                request_id,
                "bridge_failed",
                "Chrome bridge status could not be sent",
            );
        }
        let response: ClientResponse = match read_json(&mut stream) {
            Ok(response) => response,
            Err(_) => {
                return emit_error(
                    request_id,
                    "bridge_failed",
                    "Chrome bridge did not return a safe status",
                )
            }
        };
        if response.request_id != request_id {
            return emit_error(
                request_id,
                "protocol_error",
                "Chrome bridge returned an invalid status",
            );
        }
        if response.status == "ok"
            && response.code.is_none()
            && response.mode.as_deref().is_some_and(valid_mode)
            && response.origin.as_deref().map_or(true, validate_origin)
        {
            let body = serde_json::to_string(&response)
                .map_err(|_| "could not encode bridge status".to_string())?;
            println!("{body}");
            return Ok(());
        }
        let code = response
            .code
            .as_deref()
            .filter(|code| safe_code(code))
            .unwrap_or("bridge_failed");
        emit_error(
            request_id,
            code,
            "Chrome bridge did not return a safe status",
        )
    }

    fn emit_error(request_id: String, code: &str, message: &str) -> Result<(), String> {
        let response = ClientResponse::error(request_id, code);
        println!(
            "{}",
            serde_json::to_string(&response)
                .map_err(|_| "could not encode bridge status".to_string())?
        );
        Err(message.into())
    }

    fn safe_code(code: &str) -> bool {
        !code.is_empty()
            && code.len() <= 64
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }

    fn valid_mode(mode: &str) -> bool {
        matches!(
            mode,
            "off" | "selecting" | "armed" | "resolving" | "filling"
        )
    }

    fn timeout_ms(seconds: f64) -> Result<u64, String> {
        if !seconds.is_finite() || seconds <= 0.0 || seconds > (MAX_TIMEOUT_MS as f64 / 1000.0) {
            return Err(format!(
                "timeout must be between 0 and {} seconds",
                MAX_TIMEOUT_MS / 1000
            ));
        }
        let millis = (seconds * 1000.0).ceil();
        if millis < 1.0 || millis > u64::MAX as f64 {
            return Err("invalid timeout".into());
        }
        let millis = millis as u64;
        if !validate_timeout(millis) {
            return Err("invalid timeout".into());
        }
        Ok(millis)
    }
}

#[cfg(unix)]
fn main() {
    if let Err(message) = unix_client::run() {
        eprintln!("{message}");
        std::process::exit(4);
    }
}

#[cfg(not(unix))]
fn main() {
    chrome_bridge::unsupported();
}
