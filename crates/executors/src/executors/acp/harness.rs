use std::{
    path::{Path, PathBuf},
    process::Stdio,
    rc::Rc,
    sync::Arc,
};

use agent_client_protocol as proto;
use agent_client_protocol::Agent as _;
use command_group::AsyncGroupChild;
use futures::StreamExt;
use tokio::{io::AsyncWriteExt, process::Command, sync::mpsc};
use tokio_util::{
    compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt},
    io::ReaderStream,
    sync::CancellationToken,
};
use tracing::error;
use workspace_utils::{
    approvals::ApprovalStatus, command_ext::GroupSpawnNoWindowExt, stream_lines::LinesStreamExt,
};

use super::{AcpClient, SessionManager};
use crate::{
    approvals::ExecutorApprovalService,
    command::{CmdOverrides, CommandParts},
    env::ExecutionEnv,
    executors::{ExecutorError, ExecutorExitResult, SpawnedChild, acp::AcpEvent},
};

/// Complete ACP initialization handshake with Kimi (or other agents that require string protocolVersion).
/// This is needed because the agent-client-protocol crate sends protocolVersion as integer (1),
/// but Kimi expects string format ("1.0").
async fn complete_kimi_handshake<
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
>(
    stdout: &mut R,
    stdin: &mut W,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::AsyncReadExt;

    // Send initialize request with string protocolVersion
    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "1.0",
            "clientCapabilities": {},
            "clientInfo": {
                "name": "Vibe Kanban",
                "version": "0.1.28"
            }
        }
    });
    let init_json = init_request.to_string() + "\n";
    tracing::debug!("Sending Kimi initialize request: {}", init_json.trim());
    stdin.write_all(init_json.as_bytes()).await?;
    stdin.flush().await?;

    // Read response
    let mut buf = vec![0u8; 4096];
    let mut response_lines = Vec::new();
    let timeout = tokio::time::Duration::from_secs(10);

    let read_response = async {
        loop {
            let n = stdout.read(&mut buf).await?;
            if n == 0 {
                return Err::<String, std::io::Error>(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF while reading init response",
                ));
            }
            response_lines.extend_from_slice(&buf[..n]);
            // Check if we have a complete JSON object
            let response_str = String::from_utf8_lossy(&response_lines);
            if let Some(line) = response_str.lines().next() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                    if json.get("id").is_some()
                        && (json.get("result").is_some() || json.get("error").is_some())
                    {
                        tracing::debug!("Received Kimi initialize response: {}", line);
                        return Ok(line.to_string());
                    }
                }
            }
        }
    };

    match tokio::time::timeout(timeout, read_response).await {
        Ok(Ok(response)) => {
            // Check if response contains error
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                if json.get("error").is_some() {
                    tracing::warn!("Kimi initialize returned error: {}", response);
                } else {
                    tracing::debug!("Kimi initialize succeeded");
                }
            }
        }
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => {
            tracing::warn!("Timeout waiting for Kimi initialize response");
            return Err("Initialize timeout".into());
        }
    }

    // Send initialized notification (no response expected)
    let initialized_notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let notif_json = initialized_notification.to_string() + "\n";
    tracing::debug!(
        "Sending Kimi initialized notification: {}",
        notif_json.trim()
    );
    stdin.write_all(notif_json.as_bytes()).await?;
    stdin.flush().await?;

    Ok(())
}

/// Reusable harness for ACP-based conns (Gemini, Qwen, etc.)
pub struct AcpAgentHarness {
    session_namespace: String,
    model: Option<String>,
    mode: Option<String>,
    /// Whether to use string format for protocolVersion in initialize request.
    /// Some agents (like Kimi) expect "1.0" string instead of integer 1.
    use_string_protocol_version: bool,
}

impl Default for AcpAgentHarness {
    fn default() -> Self {
        // Keep existing behavior for Gemini
        Self::new()
    }
}

impl AcpAgentHarness {
    /// Create a harness with the default Gemini namespace
    pub fn new() -> Self {
        Self {
            session_namespace: "gemini_sessions".to_string(),
            model: None,
            mode: None,
            use_string_protocol_version: false,
        }
    }

    /// Create a harness with a custom session namespace (e.g. for Qwen)
    pub fn with_session_namespace(namespace: impl Into<String>) -> Self {
        Self {
            session_namespace: namespace.into(),
            model: None,
            mode: None,
            use_string_protocol_version: false,
        }
    }

    /// Create a harness for agents that require string format protocolVersion (e.g. Kimi)
    pub fn with_string_protocol_version(namespace: impl Into<String>) -> Self {
        Self {
            session_namespace: namespace.into(),
            model: None,
            mode: None,
            use_string_protocol_version: true,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = Some(mode.into());
        self
    }

    /// Enable string format for protocolVersion (for Kimi compatibility)
    pub fn with_string_protocol_version_flag(mut self) -> Self {
        self.use_string_protocol_version = true;
        self
    }

    pub fn apply_overrides(&mut self, executor_config: &crate::profile::ExecutorConfig) {
        if let Some(model_id) = &executor_config.model_id {
            self.model = Some(model_id.clone());
        }

        if let Some(agent_id) = &executor_config.agent_id {
            self.mode = Some(agent_id.clone());
        }
    }

    pub async fn spawn_with_command(
        &self,
        current_dir: &Path,
        prompt: String,
        command_parts: CommandParts,
        env: &ExecutionEnv,
        cmd_overrides: &CmdOverrides,
        approvals: Option<std::sync::Arc<dyn ExecutorApprovalService>>,
    ) -> Result<SpawnedChild, ExecutorError> {
        let (program_path, args) = command_parts.into_resolved().await?;
        let mut command = Command::new(program_path);
        command
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(current_dir)
            .env("NPM_CONFIG_LOGLEVEL", "error")
            .env("NODE_NO_WARNINGS", "1")
            .args(&args);

        env.clone()
            .with_profile(cmd_overrides)
            .apply_to_command(&mut command);

        let mut child = command.group_spawn_no_window()?;

        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<ExecutorExitResult>();
        let cancel = CancellationToken::new();

        Self::bootstrap_acp_connection(
            &mut child,
            current_dir.to_path_buf(),
            None,
            prompt,
            Some(exit_tx),
            self.session_namespace.clone(),
            self.model.clone(),
            self.mode.clone(),
            approvals,
            cancel.clone(),
            self.use_string_protocol_version,
        )
        .await?;

        Ok(SpawnedChild {
            child,
            exit_signal: Some(exit_rx),
            cancel: Some(cancel),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_follow_up_with_command(
        &self,
        current_dir: &Path,
        prompt: String,
        session_id: &str,
        command_parts: CommandParts,
        env: &ExecutionEnv,
        cmd_overrides: &CmdOverrides,
        approvals: Option<std::sync::Arc<dyn ExecutorApprovalService>>,
    ) -> Result<SpawnedChild, ExecutorError> {
        let (program_path, args) = command_parts.into_resolved().await?;
        let mut command = Command::new(program_path);
        command
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(current_dir)
            .env("NPM_CONFIG_LOGLEVEL", "error")
            .env("NODE_NO_WARNINGS", "1")
            .args(&args);

        env.clone()
            .with_profile(cmd_overrides)
            .apply_to_command(&mut command);

        let mut child = command.group_spawn_no_window()?;

        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<ExecutorExitResult>();
        let cancel = CancellationToken::new();

        Self::bootstrap_acp_connection(
            &mut child,
            current_dir.to_path_buf(),
            Some(session_id.to_string()),
            prompt,
            Some(exit_tx),
            self.session_namespace.clone(),
            self.model.clone(),
            self.mode.clone(),
            approvals,
            cancel.clone(),
            self.use_string_protocol_version,
        )
        .await?;

        Ok(SpawnedChild {
            child,
            exit_signal: Some(exit_rx),
            cancel: Some(cancel),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn bootstrap_acp_connection(
        child: &mut AsyncGroupChild,
        cwd: PathBuf,
        existing_session: Option<String>,
        prompt: String,
        exit_signal: Option<tokio::sync::oneshot::Sender<ExecutorExitResult>>,
        session_namespace: String,
        model: Option<String>,
        mode: Option<String>,
        approvals: Option<std::sync::Arc<dyn ExecutorApprovalService>>,
        cancel: CancellationToken,
        use_string_protocol_version: bool,
    ) -> Result<(), ExecutorError> {
        // Take child's stdio for ACP wiring
        let orig_stdout = child.inner().stdout.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Child process has no stdout",
            ))
        })?;
        let orig_stdin = child.inner().stdin.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Child process has no stdin",
            ))
        })?;

        // Create a fresh stdout pipe for logs
        let writer = crate::stdout_dup::create_stdout_pipe_writer(child)?;
        let shared_writer = Arc::new(tokio::sync::Mutex::new(writer));
        let (log_tx, mut log_rx) = mpsc::unbounded_channel::<String>();

        // Spawn log -> stdout writer task
        tokio::spawn(async move {
            while let Some(line) = log_rx.recv().await {
                let mut data = line.into_bytes();
                data.push(b'\n');
                let mut w = shared_writer.lock().await;
                let _ = w.write_all(&data).await;
            }
        });

        // For agents requiring string protocolVersion (e.g., Kimi), complete handshake first
        let (orig_stdout, orig_stdin) = if use_string_protocol_version {
            // Complete the initialization handshake directly with the child process
            // before setting up the ACP connection
            let mut stdout = orig_stdout;
            let mut stdin = orig_stdin;
            match complete_kimi_handshake(&mut stdout, &mut stdin).await {
                Ok(()) => {
                    tracing::debug!("Kimi ACP handshake completed successfully");
                }
                Err(e) => {
                    tracing::warn!("Kimi ACP handshake failed: {}", e);
                    // Continue anyway, let the standard flow handle errors
                }
            }
            (stdout, stdin)
        } else {
            (orig_stdout, orig_stdin)
        };

        // ACP client STDIO
        let (mut to_acp_writer, acp_incoming_reader) = tokio::io::duplex(64 * 1024);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Process stdout -> ACP
        let stdout_shutdown_rx = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut stdout_stream = ReaderStream::new(orig_stdout);
            while let Some(res) = stdout_stream.next().await {
                if *stdout_shutdown_rx.borrow() {
                    break;
                }
                match res {
                    Ok(data) => {
                        let _ = to_acp_writer.write_all(&data).await;
                    }
                    Err(_) => break,
                }
            }
        });

        // ACP crate expects futures::AsyncRead + AsyncWrite, use tokio compat to adapt tokio::io::AsyncRead + Write
        let (acp_out_writer, acp_out_reader) = tokio::io::duplex(64 * 1024);
        let outgoing = acp_out_writer.compat_write();
        let incoming = acp_incoming_reader.compat();

        // Process ACP -> stdin
        let stdin_shutdown_rx = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut child_stdin = orig_stdin;
            let mut lines = ReaderStream::new(acp_out_reader)
                .map(|res| res.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
                .lines();
            while let Some(result) = lines.next().await {
                if *stdin_shutdown_rx.borrow() {
                    break;
                }
                match result {
                    Ok(line) => {
                        // Use \r\n on Windows for compatibility with buggy ACP implementations
                        const LINE_ENDING: &str = if cfg!(windows) { "\r\n" } else { "\n" };
                        let line = line + LINE_ENDING;
                        if let Err(err) = child_stdin.write_all(line.as_bytes()).await {
                            tracing::debug!("Failed to write to child stdin {err}");
                            break;
                        }
                        let _ = child_stdin.flush().await;
                    }
                    Err(err) => {
                        tracing::debug!("ACP stdin line error {err}");
                        break;
                    }
                }
            }
        });

        let mut exit_signal_tx = exit_signal;

        // Run ACP client in a LocalSet
        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime");

            rt.block_on(async move {
                let local = tokio::task::LocalSet::new();
                local
                    .run_until(async move {
                        // Create event and raw channels
                        // Typed events available for future use; raw lines forwarded and persisted
                        let (event_tx, mut event_rx) =
                            mpsc::unbounded_channel::<crate::executors::acp::AcpEvent>();

                        // Create session manager
                        let session_manager = match SessionManager::new(session_namespace) {
                            Ok(sm) => sm,
                            Err(e) => {
                                error!("Failed to create session manager: {}", e);
                                return;
                            }
                        };
                        let session_manager = std::sync::Arc::new(session_manager);

                        // Create ACP client with approvals support
                        let client =
                            AcpClient::new(event_tx.clone(), approvals.clone(), cancel.clone());
                        let client_feedback_handle = client.clone();

                        client.record_user_prompt_event(&prompt);

                        // Set up connection
                        let (conn, io_fut) =
                            proto::ClientSideConnection::new(client, outgoing, incoming, |fut| {
                                tokio::task::spawn_local(fut);
                            });
                        let conn = Rc::new(conn);

                        // Drive I/O
                        let io_handle = tokio::task::spawn_local(async move {
                            let _ = io_fut.await;
                        });

                        // Initialize
                        let init_result = if use_string_protocol_version {
                            // For Kimi and similar agents: we already sent a custom initialization
                            // request with string protocolVersion before creating the connection.
                            // Skip the standard library initialization to avoid duplicate requests.
                            tracing::debug!("Skipping standard ACP initialization for string protocolVersion agent");
                            Ok(proto::InitializeResponse::new(proto::ProtocolVersion::V1))
                        } else {
                            conn.initialize(proto::InitializeRequest::new(proto::ProtocolVersion::V1)).await
                        };
                        if let Err(ref e) = init_result {
                            tracing::warn!("ACP initialization failed: {:?}", e);
                        }

                        // Handle session creation/forking
                        let (acp_session_id, display_session_id, prompt_to_send) =
                            if let Some(existing) = existing_session {
                                // Fork existing session
                                let new_ui_id = uuid::Uuid::new_v4().to_string();
                                let _ = session_manager.fork_session(&existing, &new_ui_id);

                                let history = session_manager.read_session_raw(&new_ui_id).ok();
                                let meta =
                                    history.map(|h| serde_json::json!({ "history_jsonl": h }));

                                let mut req = proto::NewSessionRequest::new(cwd.clone());
                                if let Some(m) = meta
                                    && let Some(obj) = m.as_object()
                                {
                                    req = req.meta(obj.clone());
                                }
                                match conn.new_session(req).await {
                                    Ok(resp) => {
                                        let resume_prompt = session_manager
                                            .generate_resume_prompt(&new_ui_id, &prompt)
                                            .unwrap_or_else(|_| prompt.clone());
                                        (resp.session_id.0.to_string(), new_ui_id, resume_prompt)
                                    }
                                    Err(e) => {
                                        error!("Failed to create session: {}", e);
                                        return;
                                    }
                                }
                            } else {
                                // New session
                                match conn
                                    .new_session(proto::NewSessionRequest::new(cwd.clone()))
                                    .await
                                {
                                    Ok(resp) => {
                                        let sid = resp.session_id.0.to_string();
                                        (sid.clone(), sid, prompt)
                                    }
                                    Err(e) => {
                                        error!("Failed to create session: {}", e);
                                        return;
                                    }
                                }
                            };

                        // Emit session ID
                        let _ = log_tx
                            .send(AcpEvent::SessionStart(display_session_id.clone()).to_string());

                        if let Some(model) = model.clone() {
                            match conn
                                .set_session_model(proto::SetSessionModelRequest::new(
                                    proto::SessionId::new(acp_session_id.clone()),
                                    model,
                                ))
                                .await
                            {
                                Ok(_) => {}
                                Err(e) => error!("Failed to set session mode: {}", e),
                            }
                        }

                        if let Some(mode) = mode.clone() {
                            match conn
                                .set_session_mode(proto::SetSessionModeRequest::new(
                                    proto::SessionId::new(acp_session_id.clone()),
                                    mode,
                                ))
                                .await
                            {
                                Ok(_) => {}
                                Err(e) => error!("Failed to set session mode: {}", e),
                            }
                        }

                        // Start raw event forwarder and persistence
                        let app_tx_clone = log_tx.clone();
                        let sess_id_for_writer = display_session_id.clone();
                        let sm_for_writer = session_manager.clone();
                        let conn_for_cancel = conn.clone();
                        let acp_session_id_for_cancel = acp_session_id.clone();
                        tokio::task::spawn_local(async move {
                            while let Some(event) = event_rx.recv().await {
                                if let AcpEvent::ApprovalResponse(resp) = &event
                                    && let ApprovalStatus::Denied {
                                        reason: Some(reason),
                                    } = &resp.status
                                    && !reason.trim().is_empty()
                                {
                                    let _ = conn_for_cancel
                                        .cancel(proto::CancelNotification::new(
                                            proto::SessionId::new(
                                                acp_session_id_for_cancel.clone(),
                                            ),
                                        ))
                                        .await;
                                }

                                let line = event.to_string();
                                // Forward to stdout
                                let _ = app_tx_clone.send(line.clone());
                                // Persist to session file
                                let _ = sm_for_writer.append_raw_line(&sess_id_for_writer, &line);
                            }
                        });

                        // Save prompt to session
                        let _ = session_manager.append_raw_line(
                            &display_session_id,
                            &serde_json::to_string(&serde_json::json!({ "user": prompt_to_send }))
                                .unwrap_or_default(),
                        );

                        // Build prompt request
                        let initial_req = proto::PromptRequest::new(
                            proto::SessionId::new(acp_session_id.clone()),
                            vec![proto::ContentBlock::Text(proto::TextContent::new(
                                prompt_to_send,
                            ))],
                        );

                        let mut current_req = Some(initial_req);

                        while let Some(req) = current_req.take() {
                            if cancel.is_cancelled() {
                                tracing::debug!("ACP executor cancelled, stopping prompt loop");
                                break;
                            }

                            tracing::trace!(?req, "sending ACP prompt request");
                            // Send the prompt and await completion to obtain stop_reason
                            let prompt_result = tokio::select! {
                                _ = cancel.cancelled() => {
                                    tracing::debug!("ACP executor cancelled during prompt");
                                    break;
                                }
                                result = conn.prompt(req) => result,
                            };

                            match prompt_result {
                                Ok(resp) => {
                                    // Emit done with stop_reason
                                    let stop_reason = serde_json::to_string(&resp.stop_reason)
                                        .unwrap_or_default();
                                    let _ = log_tx.send(AcpEvent::Done(stop_reason).to_string());
                                }
                                Err(e) => {
                                    tracing::debug!("error {} {e} {:?}", e.code, e.data);
                                    if e.code
                                        == agent_client_protocol::ErrorCode::INTERNAL_ERROR.code
                                        && e.data
                                            .as_ref()
                                            .is_some_and(|d| d == "server shut down unexpectedly")
                                    {
                                        tracing::debug!("ACP server killed");
                                    } else {
                                        let _ = log_tx
                                            .send(AcpEvent::Error(format!("{e}")).to_string());
                                    }
                                }
                            }

                            // Flush any pending user feedback after finish
                            let feedback = client_feedback_handle
                                .drain_feedback()
                                .await
                                .join("\n")
                                .trim()
                                .to_string();
                            if !feedback.is_empty() {
                                tracing::trace!(?feedback, "sending ACP follow-up feedback");
                                let session_id = proto::SessionId::new(acp_session_id.clone());
                                let feedback_req = proto::PromptRequest::new(
                                    session_id.clone(),
                                    vec![proto::ContentBlock::Text(proto::TextContent::new(
                                        feedback,
                                    ))],
                                );
                                current_req = Some(feedback_req);
                            }
                        }

                        // Notify container of completion
                        if let Some(tx) = exit_signal_tx.take() {
                            let _ = tx.send(ExecutorExitResult::Success);
                        }

                        // Cancel session work
                        let _ = conn
                            .cancel(proto::CancelNotification::new(proto::SessionId::new(
                                acp_session_id,
                            )))
                            .await;

                        // Cleanup
                        drop(conn);
                        let _ = shutdown_tx.send(true);
                        let _ = io_handle.await;
                        drop(log_tx);
                    })
                    .await;
            });
        });

        Ok(())
    }
}
