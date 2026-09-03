//! Native Messaging host for the optional knapper Chrome bridge.
//!
//! Chrome owns this process's stdin/stdout.  Every byte on stdout is a
//! length-prefixed Native Messaging JSON frame; diagnostics are deliberately
//! generic and never contain provider output, page text, or a resolved value.
//! Codex talks to the host through the user-only Unix socket instead.

#[path = "../chrome_bridge.rs"]
mod chrome_bridge;

#[cfg(unix)]
mod unix_host {
    use std::io::{self, BufWriter, Read};
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::chrome_bridge::{
        self, read_frame, read_json, socket_path, write_json, ApiRequest, ClientFormAction,
        ClientRequest, ClientResponse, ClientResult, NativeFormAction, NativeMessage, MAX_FRAME,
        MAX_TIMEOUT_MS,
    };

    const POLL: Duration = Duration::from_millis(10);
    // Leave room for the Native Messaging JSON envelope and request id.
    const MAX_VALUE: usize = MAX_FRAME - 4096;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct BrowserContext {
        #[allow(dead_code)]
        tab_id: i64,
        url: String,
        origin: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum HostMode {
        Off,
        Pick,
        All,
        Selecting,
        Armed,
        Resolving,
        Filling,
    }

    impl HostMode {
        fn as_str(self) -> &'static str {
            match self {
                Self::Off => "off",
                Self::Pick => "pick",
                Self::All => "all",
                Self::Selecting => "selecting",
                Self::Armed => "armed",
                Self::Resolving => "resolving",
                Self::Filling => "filling",
            }
        }
    }

    struct ClientEnvelope {
        request: ClientRequest,
        response: Sender<ClientResponse>,
    }

    enum HostEvent {
        Client(ClientEnvelope),
        Native(io::Result<Vec<u8>>),
        Resolved {
            request_id: String,
            value: Result<String, ResolveFailure>,
        },
        ApiPrepared {
            request_id: String,
            actions: Result<Vec<NativeFormAction>, ResolveFailure>,
        },
    }

    struct Pending {
        request: ClientRequest,
        expected_origin: String,
        response: Sender<ClientResponse>,
        deadline: Instant,
        phase: Phase,
    }

    enum Phase {
        Preparing,
        Resolving,
        Filling,
        ApiResolving,
        ApiWaiting,
    }

    #[derive(Debug)]
    enum ResolveFailure {
        Failed,
        TimedOut,
        InvalidOutput,
    }

    struct NativeWriter {
        output: BufWriter<io::Stdout>,
    }

    impl NativeWriter {
        fn send(&mut self, message: &NativeMessage) -> io::Result<()> {
            write_json(&mut self.output, message)
        }
    }

    pub fn run() -> Result<(), String> {
        let path =
            socket_path().map_err(|_| "cannot determine the user runtime socket".to_string())?;
        let listener = bind_socket(&path)?;

        let (events_tx, events_rx) = mpsc::channel();
        spawn_socket_listener(listener, events_tx.clone());
        spawn_native_reader(events_tx.clone());

        let stdout = io::stdout();
        let writer = Arc::new(Mutex::new(NativeWriter {
            output: BufWriter::new(stdout),
        }));

        // The extension opens the Native Messaging port first.  The host is
        // intentionally useful only while that Chrome-owned pipe is alive.
        let mut context: Option<BrowserContext> = None;
        let mut mode = HostMode::Off;
        let mut pending: Option<Pending> = None;

        loop {
            match events_rx.recv_timeout(POLL) {
                Ok(event) => {
                    if !handle_event(
                        event,
                        &writer,
                        &mut context,
                        &mut mode,
                        &mut pending,
                        &events_tx,
                    )? {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if pending
                .as_ref()
                .is_some_and(|item| Instant::now() >= item.deadline)
            {
                if let Some(item) = pending.take() {
                    if !matches!(item.phase, Phase::ApiResolving | Phase::ApiWaiting) {
                        mode = if context.is_some() {
                            HostMode::Armed
                        } else {
                            HostMode::Off
                        };
                    }
                    send_client_error(item.response, item.request.request_id, "timeout");
                }
            }
        }

        // The socket is private to this host instance.  Remove only the
        // exact socket we created; a regular file or symlink is never touched.
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    fn handle_event(
        event: HostEvent,
        writer: &Arc<Mutex<NativeWriter>>,
        context: &mut Option<BrowserContext>,
        mode: &mut HostMode,
        pending: &mut Option<Pending>,
        events_tx: &Sender<HostEvent>,
    ) -> Result<bool, String> {
        match event {
            HostEvent::Client(envelope) => {
                let request = envelope.request;
                if request.kind == "status" {
                    if !valid_request_id(&request.request_id) {
                        send_client_error(envelope.response, request.request_id, "invalid_request");
                    } else {
                        let origin = context.as_ref().map(|item| item.origin.clone());
                        send_client_response(
                            envelope.response,
                            ClientResponse::state(request.request_id, mode.as_str(), origin),
                        );
                    }
                    return Ok(true);
                }
                if request.kind == "api" {
                    let api = request.api.clone();
                    if !request.reference.is_empty()
                        || request.expected_origin.is_some()
                        || !chrome_bridge::validate_timeout(request.timeout_ms)
                        || !valid_request_id(&request.request_id)
                        || api
                            .as_ref()
                            .map_or(true, |api| !chrome_bridge::validate_api_request(api))
                    {
                        send_client_error(envelope.response, request.request_id, "invalid_request");
                        return Ok(true);
                    }
                    if *mode != HostMode::All {
                        send_client_error(envelope.response, request.request_id, "not_ready");
                        return Ok(true);
                    }
                    if pending.is_some() {
                        send_client_error(envelope.response, request.request_id, "busy");
                        return Ok(true);
                    }
                    let api = api.expect("API checked above");
                    let request_id = request.request_id.clone();
                    let timeout_ms = request.timeout_ms;
                    let phase = if matches!(api, ApiRequest::FormPerform { .. }) {
                        Phase::ApiResolving
                    } else {
                        Phase::ApiWaiting
                    };
                    *pending = Some(Pending {
                        request,
                        expected_origin: String::new(),
                        response: envelope.response,
                        deadline: Instant::now() + Duration::from_millis(timeout_ms),
                        phase,
                    });
                    match api {
                        ApiRequest::TabsList { origin } => {
                            send_native(writer, &NativeMessage::TabsList { request_id, origin })?
                        }
                        ApiRequest::FormSnapshot { tab_id } => send_native(
                            writer,
                            &NativeMessage::FormSnapshot { request_id, tab_id },
                        )?,
                        ApiRequest::FormPerform {
                            tab_id: _,
                            document_id: _,
                            actions,
                        } => {
                            let tx = events_tx.clone();
                            thread::spawn(move || {
                                let actions = prepare_native_actions(actions, timeout_ms);
                                let _ = tx.send(HostEvent::ApiPrepared {
                                    request_id,
                                    actions,
                                });
                            });
                        }
                        ApiRequest::FormSubmit {
                            tab_id,
                            document_id,
                            form_id,
                        } => send_native(
                            writer,
                            &NativeMessage::FormSubmit {
                                request_id,
                                tab_id,
                                document_id,
                                form_id,
                            },
                        )?,
                    }
                    return Ok(true);
                }
                if request.kind != "fill"
                    || !chrome_bridge::validate_reference(&request.reference)
                    || request
                        .expected_origin
                        .as_deref()
                        .is_some_and(|origin| !chrome_bridge::validate_origin(origin))
                    || !chrome_bridge::validate_timeout(request.timeout_ms)
                    || !valid_request_id(&request.request_id)
                    || request.api.is_some()
                {
                    send_client_error(envelope.response, request.request_id, "invalid_request");
                    return Ok(true);
                }
                if pending.is_some() {
                    send_client_error(envelope.response, request.request_id, "busy");
                    return Ok(true);
                }
                let Some(armed_tab) = context.as_ref() else {
                    send_client_error(envelope.response, request.request_id, "not_armed");
                    return Ok(true);
                };
                if *mode != HostMode::Armed {
                    send_client_error(envelope.response, request.request_id, "not_ready");
                    return Ok(true);
                }
                let expected_origin = request
                    .expected_origin
                    .as_deref()
                    .unwrap_or(&armed_tab.origin);
                if expected_origin != armed_tab.origin {
                    send_client_error(envelope.response, request.request_id, "origin_mismatch");
                    return Ok(true);
                }

                let timeout = Duration::from_millis(request.timeout_ms);
                let request_id = request.request_id.clone();
                *pending = Some(Pending {
                    request: request.clone(),
                    expected_origin: expected_origin.to_string(),
                    response: envelope.response,
                    deadline: Instant::now() + timeout,
                    phase: Phase::Preparing,
                });
                *mode = HostMode::Armed;
                send_native(
                    writer,
                    &NativeMessage::PrepareFill {
                        request_id,
                        reference: request.reference,
                        expected_origin: expected_origin.to_string(),
                        expected_url: armed_tab.url.clone(),
                        timeout_ms: request.timeout_ms,
                    },
                )?;
            }
            HostEvent::Native(result) => {
                let body = match result {
                    Ok(body) => body,
                    Err(_) => return Ok(false),
                };
                let message: NativeMessage = match serde_json::from_slice(&body) {
                    Ok(message) => message,
                    Err(_) => return Ok(false),
                };
                if !handle_native_message(message, writer, context, mode, pending, events_tx)? {
                    return Ok(false);
                }
            }
            HostEvent::Resolved { request_id, value } => {
                let Some(item) = pending.as_mut() else {
                    return Ok(true);
                };
                if item.request.request_id != request_id || !matches!(item.phase, Phase::Resolving)
                {
                    return Ok(true);
                }
                match value {
                    Ok(value) => {
                        item.phase = Phase::Filling;
                        *mode = HostMode::Filling;
                        send_native(writer, &NativeMessage::Fill { request_id, value })?;
                    }
                    Err(ResolveFailure::TimedOut) => {
                        let item = pending.take().expect("pending item exists");
                        *mode = if context.is_some() {
                            HostMode::Armed
                        } else {
                            HostMode::Off
                        };
                        send_client_error(
                            item.response,
                            item.request.request_id,
                            "provider_timeout",
                        );
                    }
                    Err(ResolveFailure::Failed | ResolveFailure::InvalidOutput) => {
                        let item = pending.take().expect("pending item exists");
                        *mode = if context.is_some() {
                            HostMode::Armed
                        } else {
                            HostMode::Off
                        };
                        send_client_error(
                            item.response,
                            item.request.request_id,
                            "provider_failed",
                        );
                    }
                }
            }
            HostEvent::ApiPrepared {
                request_id,
                actions,
            } => {
                let Some(item) = pending.as_mut() else {
                    return Ok(true);
                };
                if item.request.request_id != request_id
                    || !matches!(item.phase, Phase::ApiResolving)
                {
                    return Ok(true);
                }
                match actions {
                    Ok(actions) => {
                        let Some(ApiRequest::FormPerform {
                            tab_id,
                            document_id,
                            ..
                        }) = item.request.api.as_ref()
                        else {
                            return Err("invalid pending API request".into());
                        };
                        item.phase = Phase::ApiWaiting;
                        send_native(
                            writer,
                            &NativeMessage::FormPerform {
                                request_id,
                                tab_id: *tab_id,
                                document_id: document_id.clone(),
                                actions,
                            },
                        )?;
                    }
                    Err(ResolveFailure::TimedOut) => {
                        let item = pending.take().expect("pending item exists");
                        send_client_error(
                            item.response,
                            item.request.request_id,
                            "provider_timeout",
                        );
                    }
                    Err(ResolveFailure::Failed | ResolveFailure::InvalidOutput) => {
                        let item = pending.take().expect("pending item exists");
                        send_client_error(
                            item.response,
                            item.request.request_id,
                            "provider_failed",
                        );
                    }
                }
            }
        }
        Ok(true)
    }

    fn handle_native_message(
        message: NativeMessage,
        writer: &Arc<Mutex<NativeWriter>>,
        context: &mut Option<BrowserContext>,
        mode: &mut HostMode,
        pending: &mut Option<Pending>,
        events_tx: &Sender<HostEvent>,
    ) -> Result<bool, String> {
        match message {
            NativeMessage::Hello => send_native(writer, &NativeMessage::HelloAck)?,
            NativeMessage::Mode { mode: announced } => match announced.as_str() {
                "off" => {
                    cancel_pending(pending, "mode_changed");
                    *context = None;
                    *mode = HostMode::Off;
                }
                "pick" => {
                    cancel_pending(pending, "mode_changed");
                    *context = None;
                    *mode = HostMode::Pick;
                }
                "all" => {
                    if *mode != HostMode::All {
                        cancel_pending(pending, "mode_changed");
                    }
                    *context = None;
                    *mode = HostMode::All;
                }
                _ => return Ok(false),
            },
            NativeMessage::Selecting {
                tab_id,
                url,
                origin,
            } => {
                if !valid_tab_id(tab_id)
                    || !chrome_bridge::url_matches_origin(&url, &origin)
                    || url.as_bytes().contains(&0)
                {
                    cancel_pending(pending, "selection_changed");
                    *context = None;
                    *mode = HostMode::Off;
                } else {
                    // The extension announces its next selection before it
                    // acknowledges the fill that just completed. Preserve
                    // that in-flight client request when the context is the
                    // same and the host is waiting for `Filled`.
                    let returning_from_fill = matches!(
                        pending.as_ref().map(|item| &item.phase),
                        Some(Phase::Filling)
                    ) && context.as_ref().is_some_and(|current| {
                        current.tab_id == tab_id && current.url == url && current.origin == origin
                    });
                    if !returning_from_fill {
                        cancel_pending(pending, "selection_changed");
                    }
                    *context = Some(BrowserContext {
                        tab_id,
                        url,
                        origin,
                    });
                    *mode = HostMode::Selecting;
                }
            }
            NativeMessage::Arm {
                tab_id,
                url,
                origin,
            } => {
                if !valid_tab_id(tab_id)
                    || !chrome_bridge::url_matches_origin(&url, &origin)
                    || url.as_bytes().contains(&0)
                {
                    cancel_pending(pending, "selection_changed");
                    *context = None;
                    *mode = HostMode::Off;
                    send_native(
                        writer,
                        &NativeMessage::TargetRejected {
                            request_id: "arm".into(),
                            code: "invalid_arm".into(),
                        },
                    )?;
                } else {
                    cancel_pending(pending, "selection_changed");
                    *context = Some(BrowserContext {
                        tab_id,
                        url,
                        origin,
                    });
                    *mode = HostMode::Armed;
                    send_native(writer, &NativeMessage::Armed)?;
                }
            }
            NativeMessage::TargetReady { request_id } => {
                let Some(item) = pending.as_mut() else {
                    return Ok(true);
                };
                if item.request.request_id != request_id || !matches!(item.phase, Phase::Preparing)
                {
                    return Ok(true);
                }
                let remaining_ms = item
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis();
                if remaining_ms == 0 {
                    let item = pending.take().expect("pending item exists");
                    *mode = if context.is_some() {
                        HostMode::Armed
                    } else {
                        HostMode::Off
                    };
                    send_client_error(item.response, item.request.request_id, "timeout");
                    return Ok(true);
                }
                item.phase = Phase::Resolving;
                *mode = HostMode::Resolving;
                let reference = item.request.reference.clone();
                let tx = events_tx.clone();
                // Leave a small guard window for knapper to terminate its own
                // provider process group before this host's outer deadline.
                // Knapper and providers intentionally use separate groups.
                let provider_timeout_ms = remaining_ms
                    .min(MAX_TIMEOUT_MS as u128)
                    .saturating_sub(100)
                    .max(1) as u64;
                thread::spawn(move || {
                    let value = resolve_value(&reference, provider_timeout_ms);
                    let _ = tx.send(HostEvent::Resolved { request_id, value });
                });
            }
            NativeMessage::TargetRejected {
                request_id,
                code: _,
            }
            | NativeMessage::FillRejected {
                request_id,
                code: _,
            } => {
                if pending
                    .as_ref()
                    .is_some_and(|item| item.request.request_id == request_id)
                {
                    let item = pending.take().expect("pending item exists");
                    *mode = if context.is_some() {
                        HostMode::Armed
                    } else {
                        HostMode::Off
                    };
                    send_client_error(item.response, item.request.request_id, "target_rejected");
                }
            }
            NativeMessage::Filled { request_id } => {
                if pending
                    .as_ref()
                    .is_some_and(|item| item.request.request_id == request_id)
                {
                    let item = pending.take().expect("pending item exists");
                    send_client_response(
                        item.response,
                        ClientResponse::ok(item.request.request_id, item.expected_origin),
                    );
                    // Newer extensions send Selecting before Filled, so do
                    // not overwrite that state while acknowledging the
                    // completed request. Older extensions remain Filling
                    // until the next selection/arm message arrives.
                    if *mode != HostMode::Selecting {
                        *mode = HostMode::Filling;
                    }
                }
            }
            NativeMessage::TabsListed { request_id, tabs } => {
                let Some(item) = pending.as_ref() else {
                    return Ok(true);
                };
                let expected_origin = match item.request.api.as_ref() {
                    Some(ApiRequest::TabsList { origin })
                        if item.request.request_id == request_id
                            && matches!(item.phase, Phase::ApiWaiting) =>
                    {
                        origin
                    }
                    _ => return Ok(true),
                };
                let result = ClientResult::TabsList { tabs };
                let valid = chrome_bridge::validate_client_result(&result)
                    && match (&result, expected_origin) {
                        (ClientResult::TabsList { tabs }, Some(origin)) => {
                            tabs.iter().all(|tab| &tab.origin == origin)
                        }
                        _ => true,
                    };
                finish_api_result(pending, request_id, result, valid);
            }
            NativeMessage::FormSnapshotted {
                request_id,
                snapshot,
            } => {
                let valid_request = pending.as_ref().is_some_and(|item| {
                    item.request.request_id == request_id
                        && matches!(item.phase, Phase::ApiWaiting)
                        && matches!(
                            item.request.api.as_ref(),
                            Some(ApiRequest::FormSnapshot { tab_id }) if *tab_id == snapshot.tab_id
                        )
                });
                if !valid_request {
                    return Ok(true);
                }
                let result = ClientResult::FormSnapshot { snapshot };
                let valid = chrome_bridge::validate_client_result(&result);
                finish_api_result(pending, request_id, result, valid);
            }
            NativeMessage::FormPerformed {
                request_id,
                document_id,
                results,
            } => {
                let valid_request = pending.as_ref().is_some_and(|item| {
                    item.request.request_id == request_id
                        && matches!(item.phase, Phase::ApiWaiting)
                        && matches!(
                            item.request.api.as_ref(),
                            Some(ApiRequest::FormPerform {
                                document_id: expected,
                                actions,
                                ..
                            }) if expected == &document_id
                                && results_match_actions(actions, &results)
                        )
                });
                if !valid_request {
                    return Ok(true);
                }
                let result = ClientResult::FormPerform {
                    document_id,
                    results,
                };
                let valid = chrome_bridge::validate_client_result(&result);
                finish_api_result(pending, request_id, result, valid);
            }
            NativeMessage::FormSubmitted {
                request_id,
                document_id,
                status,
            } => {
                let valid_request = pending.as_ref().is_some_and(|item| {
                    item.request.request_id == request_id
                        && matches!(item.phase, Phase::ApiWaiting)
                        && matches!(
                            item.request.api.as_ref(),
                            Some(ApiRequest::FormSubmit { document_id: expected, .. })
                                if expected == &document_id
                        )
                });
                if !valid_request {
                    return Ok(true);
                }
                let result = ClientResult::FormSubmit {
                    document_id,
                    state: status,
                };
                let valid = chrome_bridge::validate_client_result(&result);
                finish_api_result(pending, request_id, result, valid);
            }
            NativeMessage::ApiRejected {
                request_id,
                code: _,
            } => {
                if pending.as_ref().is_some_and(|item| {
                    item.request.request_id == request_id
                        && matches!(item.phase, Phase::ApiResolving | Phase::ApiWaiting)
                }) {
                    let item = pending.take().expect("pending item exists");
                    send_client_error(item.response, item.request.request_id, "api_rejected");
                }
            }
            // These are host-originated messages.  Receiving them is a
            // protocol violation; terminate rather than guessing state.
            NativeMessage::HelloAck
            | NativeMessage::Armed
            | NativeMessage::PrepareFill { .. }
            | NativeMessage::Fill { .. }
            | NativeMessage::TabsList { .. }
            | NativeMessage::FormSnapshot { .. }
            | NativeMessage::FormPerform { .. }
            | NativeMessage::FormSubmit { .. } => return Ok(false),
        }
        Ok(true)
    }

    fn finish_api_result(
        pending: &mut Option<Pending>,
        request_id: String,
        result: ClientResult,
        valid: bool,
    ) {
        let item = pending.take().expect("API pending item exists");
        if valid {
            send_client_response(item.response, ClientResponse::api(request_id, result));
        } else {
            send_client_error(item.response, request_id, "protocol_error");
        }
    }

    fn cancel_pending(pending: &mut Option<Pending>, code: &str) {
        if let Some(item) = pending.take() {
            send_client_error(item.response, item.request.request_id, code);
        }
    }

    fn send_native(
        writer: &Arc<Mutex<NativeWriter>>,
        message: &NativeMessage,
    ) -> Result<(), String> {
        let mut writer = writer
            .lock()
            .map_err(|_| "native writer unavailable".to_string())?;
        writer
            .send(message)
            .map_err(|_| "native messaging pipe unavailable".to_string())
    }

    fn send_client_response(sender: Sender<ClientResponse>, response: ClientResponse) {
        let _ = sender.send(response);
    }

    fn send_client_error(sender: Sender<ClientResponse>, request_id: String, code: &str) {
        send_client_response(sender, ClientResponse::error(request_id, code));
    }

    fn valid_request_id(request_id: &str) -> bool {
        !request_id.is_empty()
            && request_id.len() <= 128
            && request_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }

    fn valid_tab_id(tab_id: i64) -> bool {
        tab_id >= 0
    }

    fn spawn_native_reader(events_tx: Sender<HostEvent>) {
        thread::spawn(move || {
            let mut stdin = io::stdin().lock();
            loop {
                let result = read_frame(&mut stdin);
                let done = result.is_err();
                if events_tx.send(HostEvent::Native(result)).is_err() || done {
                    break;
                }
            }
        });
    }

    fn spawn_socket_listener(listener: UnixListener, events_tx: Sender<HostEvent>) {
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let tx = events_tx.clone();
                thread::spawn(move || serve_client(stream, tx));
            }
        });
    }

    fn serve_client(mut stream: UnixStream, events_tx: Sender<HostEvent>) {
        let request: ClientRequest = match read_json(&mut stream) {
            Ok(request) => request,
            Err(_) => return,
        };
        let (response_tx, response_rx) = mpsc::channel();
        if events_tx
            .send(HostEvent::Client(ClientEnvelope {
                request,
                response: response_tx,
            }))
            .is_err()
        {
            return;
        }
        let Ok(response) = response_rx.recv() else {
            return;
        };
        let _ = write_json(&mut stream, &response);
    }

    fn bind_socket(path: &Path) -> Result<UnixListener, String> {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if !metadata.file_type().is_socket() {
                return Err("refusing an existing non-socket runtime path".into());
            }
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err("refusing a socket owned by another user".into());
            }
            match UnixStream::connect(path) {
                Ok(_) => return Err("another Chrome bridge host is already running".into()),
                Err(_) => std::fs::remove_file(path)
                    .map_err(|_| "cannot remove a stale user-owned socket".to_string())?,
            }
        }

        let listener = UnixListener::bind(path)
            .map_err(|error| format!("cannot bind the user runtime socket ({error})"))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "cannot secure the runtime socket".to_string())?;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| "cannot inspect the runtime socket".to_string())?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err("runtime socket is not user-only".into());
        }
        Ok(listener)
    }

    fn resolve_binary() -> Result<PathBuf, ResolveFailure> {
        if let Some(path) = std::env::var_os("KNAPPER_BINARY") {
            let path = PathBuf::from(path);
            if path.is_absolute() && path.is_file() {
                return Ok(path);
            }
            return Err(ResolveFailure::Failed);
        }
        let host = std::env::current_exe().map_err(|_| ResolveFailure::Failed)?;
        let sibling = host.parent().ok_or(ResolveFailure::Failed)?.join("knapper");
        if sibling.is_file() {
            Ok(sibling)
        } else {
            Err(ResolveFailure::Failed)
        }
    }

    fn prepare_native_actions(
        actions: Vec<ClientFormAction>,
        timeout_ms: u64,
    ) -> Result<Vec<NativeFormAction>, ResolveFailure> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_TIMEOUT_MS));
        let mut native = Vec::with_capacity(actions.len());
        for action in actions {
            let action = match action {
                ClientFormAction::SetFrom {
                    target_id,
                    reference,
                } => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(ResolveFailure::TimedOut);
                    }
                    NativeFormAction::SetValue {
                        target_id,
                        value: resolve_value(&reference, remaining.as_millis().max(1) as u64)?,
                        opaque: true,
                    }
                }
                ClientFormAction::SetValue { target_id, value } => NativeFormAction::SetValue {
                    target_id,
                    value,
                    opaque: false,
                },
                ClientFormAction::SelectOption { target_id, value } => {
                    NativeFormAction::SelectOption { target_id, value }
                }
                ClientFormAction::SetChecked { target_id, checked } => {
                    NativeFormAction::SetChecked { target_id, checked }
                }
            };
            native.push(action);
        }
        if serde_json::to_vec(&native)
            .map_err(|_| ResolveFailure::InvalidOutput)?
            .len()
            > MAX_VALUE
        {
            return Err(ResolveFailure::InvalidOutput);
        }
        Ok(native)
    }

    fn results_match_actions(
        actions: &[ClientFormAction],
        results: &[chrome_bridge::FormActionResult],
    ) -> bool {
        actions.len() == results.len()
            && actions.iter().zip(results).all(|(action, result)| {
                let (target_id, op) = match action {
                    ClientFormAction::SetFrom { target_id, .. }
                    | ClientFormAction::SetValue { target_id, .. } => {
                        (target_id.as_str(), "set_value")
                    }
                    ClientFormAction::SelectOption { target_id, .. } => {
                        (target_id.as_str(), "select_option")
                    }
                    ClientFormAction::SetChecked { target_id, .. } => {
                        (target_id.as_str(), "set_checked")
                    }
                };
                result.target_id == target_id && result.op == op
            })
    }

    fn resolve_value(reference: &str, timeout_ms: u64) -> Result<String, ResolveFailure> {
        let binary = resolve_binary()?;
        let mut command = Command::new(binary);
        command
            .args(["resolve", reference, "--timeout"])
            .arg(format!("{:.3}", timeout_ms as f64 / 1000.0))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // Provider diagnostics may contain values.  The host never
            // inherits or returns them; `resolve` has already classified the
            // failure by exit code.
            .stderr(Stdio::null());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let mut child = command.spawn().map_err(|_| ResolveFailure::Failed)?;
        let mut stdout = child.stdout.take().ok_or(ResolveFailure::Failed)?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = io::Read::by_ref(&mut stdout)
                .take((MAX_VALUE + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send(result);
        });

        let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_TIMEOUT_MS));
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
                Ok(None) => {
                    kill_child(&mut child);
                    let _ = child.wait();
                    return Err(ResolveFailure::TimedOut);
                }
                Err(_) => return Err(ResolveFailure::Failed),
            }
        };

        let bytes = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| ResolveFailure::TimedOut)?
            .map_err(|_| ResolveFailure::Failed)?;
        if !status.success() || bytes.is_empty() || bytes.len() > MAX_VALUE {
            return Err(if bytes.len() > MAX_VALUE {
                ResolveFailure::InvalidOutput
            } else {
                ResolveFailure::Failed
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| ResolveFailure::InvalidOutput)?;
        if text.is_empty() {
            return Err(ResolveFailure::InvalidOutput);
        }
        Ok(text)
    }

    fn kill_child(child: &mut Child) {
        #[cfg(unix)]
        {
            let group = -(child.id() as i32);
            if unsafe { libc::kill(group, libc::SIGKILL) } == 0 {
                return;
            }
        }
        let _ = child.kill();
    }
}

#[cfg(unix)]
fn main() {
    if let Err(message) = unix_host::run() {
        eprintln!("knapper Chrome bridge unavailable: {message}");
        std::process::exit(4);
    }
}

#[cfg(not(unix))]
fn main() {
    chrome_bridge::unsupported();
}
