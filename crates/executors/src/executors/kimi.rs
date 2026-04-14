use std::{path::Path, process::Stdio, sync::Arc, time::Duration};

use async_trait::async_trait;
// AsyncGroupChild is used via Into<SpawnedChild>
use derivative::Derivative;
use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, sync::Mutex};
use ts_rs::TS;
use uuid::Uuid;
use workspace_utils::{command_ext::GroupSpawnNoWindowExt, msg_store::MsgStore};

use crate::{
    approvals::ExecutorApprovalService,
    command::{CmdOverrides, CommandBuildError, CommandBuilder, apply_overrides},
    env::ExecutionEnv,
    executor_discovery::ExecutorDiscoveredOptions,
    executors::{
        AppendPrompt, AvailabilityInfo, BaseCodingAgent, ExecutorError, SpawnedChild,
        StandardCodingAgentExecutor,
    },
    logs::{
        NormalizedEntry, NormalizedEntryError, NormalizedEntryType,
        plain_text_processor::PlainTextLogProcessor,
        utils::{ConversationPatch, EntryIndexProvider, patch},
    },
    model_selector::{ModelInfo, ModelSelectorConfig, PermissionPolicy},
    profile::ExecutorConfig,
};

const KIMI_AUTH_REQUIRED_MSG: &str = "Please login first";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KimiRole {
    Assistant,
    User,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KimiContentBlock {
    Think {
        think: String,
        #[serde(default)]
        encrypted: Option<String>,
    },
    Text {
        text: String,
    },
}

/// Kimi tool call structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiToolCall {
    #[serde(rename = "type")]
    pub call_type: String,
    pub id: String,
    pub function: KimiFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Helper enum to handle content that can be either a string or an array of blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KimiContent {
    Text(String),
    Blocks(Vec<KimiContentBlock>),
}

impl KimiContent {
    fn into_blocks(self) -> Vec<KimiContentBlock> {
        match self {
            KimiContent::Text(text) => vec![KimiContentBlock::Text { text }],
            KimiContent::Blocks(blocks) => blocks,
        }
    }
}

/// Kimi's stream-json output format (role-based)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiMessage {
    pub role: KimiRole,
    pub content: KimiContent,
    #[serde(default)]
    pub tool_calls: Vec<KimiToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

/// Parse Kimi JSON output lines
fn parse_kimi_line(line: &str) -> Option<ParsedKimiEvent> {
    // Try to parse as role-based message (assistant/tool) - legacy format
    if let Ok(msg) = serde_json::from_str::<KimiMessage>(line) {
        return Some(ParsedKimiEvent::Message(msg));
    }

    // Try to parse new format (Thought, Message, ToolCall, etc.)
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
        // Handle Thought {"Thought": {"type": "text", "text": "..."}}
        if let Some(thought) = value.get("Thought") {
            if let Some(text) = thought.get("text").and_then(|v| v.as_str()) {
                return Some(ParsedKimiEvent::Message(KimiMessage {
                    role: KimiRole::Assistant,
                    content: KimiContent::Blocks(vec![KimiContentBlock::Think {
                        think: text.to_string(),
                        encrypted: None,
                    }]),
                    tool_calls: vec![],
                    tool_call_id: None,
                }));
            }
        }

        // Handle Message {"Message": {"type": "text", "text": "..."}}
        if let Some(message) = value.get("Message") {
            if let Some(text) = message.get("text").and_then(|v| v.as_str()) {
                return Some(ParsedKimiEvent::Message(KimiMessage {
                    role: KimiRole::Assistant,
                    content: KimiContent::Blocks(vec![KimiContentBlock::Text {
                        text: text.to_string(),
                    }]),
                    tool_calls: vec![],
                    tool_call_id: None,
                }));
            }
        }

        // Handle ToolCall {"ToolCall": {...}}
        if let Some(tool_call) = value.get("ToolCall") {
            if let (Some(tool_call_id), Some(title)) = (
                tool_call.get("toolCallId").and_then(|v| v.as_str()),
                tool_call.get("title").and_then(|v| v.as_str()),
            ) {
                // Extract arguments from content if available
                let arguments = tool_call
                    .get("content")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.get(0))
                    .and_then(|c| c.get("content"))
                    .and_then(|c| c.get("text"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                return Some(ParsedKimiEvent::Message(KimiMessage {
                    role: KimiRole::Assistant,
                    content: KimiContent::Text(String::new()),
                    tool_calls: vec![KimiToolCall {
                        call_type: "function".to_string(),
                        id: tool_call_id.to_string(),
                        function: KimiFunctionCall {
                            name: title.to_string(),
                            arguments,
                        },
                    }],
                    tool_call_id: None,
                }));
            }
        }

        // Handle Done {"Done": "..."} - treat as turn end
        if value.get("Done").is_some() {
            return Some(ParsedKimiEvent::RawLog("TurnEnd".to_string()));
        }

        // Handle SessionStart {"SessionStart": "..."} - ignore
        if value.get("SessionStart").is_some() {
            return None;
        }

        // Handle User {"User": "..."} - ignore (echo of user input)
        if value.get("User").is_some() {
            return None;
        }

        // Handle Other {"Other": {...}} - log but don't display
        if value.get("Other").is_some() {
            return None;
        }

        // Handle ToolUpdate {"ToolUpdate": {...}} - ignore updates for now
        if value.get("ToolUpdate").is_some() {
            return None;
        }

        // Handle status update with context_usage
        if value.get("context_usage").is_some() {
            return Some(ParsedKimiEvent::Status(KimiStatusUpdate {
                context_usage: value["context_usage"].as_f64().unwrap_or(0.0),
                context_tokens: value["context_tokens"].as_i64().unwrap_or(0),
                max_context_tokens: value["max_context_tokens"].as_i64().unwrap_or(262144),
                message_id: value["message_id"].as_str().unwrap_or("").to_string(),
                plan_mode: value["plan_mode"].as_bool().unwrap_or(false),
            }));
        }
    }

    // Try to parse as status update (text format)
    if line.starts_with("StatusUpdate(") {
        return Some(ParsedKimiEvent::RawLog(line.to_string()));
    }

    // Treat as raw output if not recognized
    Some(ParsedKimiEvent::RawLog(line.to_string()))
}

/// Parse tool arguments JSON into action type
fn parse_tool_arguments(
    tool_name: &str,
    arguments: &str,
) -> (Option<String>, Option<serde_json::Value>) {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(args) => {
            let display_name = match tool_name {
                "ReadFile" => args.get("path").and_then(|v| v.as_str()).map(|p| format!("Read {}", p)),
                "WriteFile" => args.get("path").and_then(|v| v.as_str()).map(|p| format!("Write {}", p)),
                "StrReplaceFile" => args.get("path").and_then(|v| v.as_str()).map(|p| format!("Edit {}", p)),
                "Shell" => args.get("command").and_then(|v| v.as_str()).map(|c| format!("Shell: {}", c)),
                "Grep" => args.get("pattern").and_then(|v| v.as_str()).map(|p| format!("Search: {}", p)),
                "Glob" => args.get("pattern").and_then(|v| v.as_str()).map(|p| format!("Glob: {}", p)),
                _ => Some(format!("{} tool", tool_name)),
            };
            (display_name, Some(args))
        }
        Err(_) => (Some(format!("{} tool", tool_name)), None),
    }
}

/// Parse tool arguments into ActionType
fn parse_action_type(
    tool_name: &str,
    args: Option<&serde_json::Value>,
) -> crate::logs::ActionType {
    use crate::logs::ActionType;
    use crate::logs::utils::shell_command_parsing::CommandCategory;
    
    match tool_name {
        "ReadFile" => ActionType::FileRead {
            path: args.and_then(|a| a.get("path")).and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
        },
        "WriteFile" => ActionType::FileEdit {
            path: args.and_then(|a| a.get("path")).and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
            changes: vec![],
        },
        "StrReplaceFile" => ActionType::FileEdit {
            path: args.and_then(|a| a.get("path")).and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
            changes: vec![],
        },
        "Shell" => ActionType::CommandRun {
            command: args.and_then(|a| a.get("command")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            result: None,
            category: CommandCategory::Other,
        },
        "Grep" => ActionType::Search {
            query: args.and_then(|a| a.get("pattern")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
        },
        "Glob" => ActionType::Search {
            query: args.and_then(|a| a.get("pattern")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
        },
        _ => ActionType::Tool {
            tool_name: tool_name.to_string(),
            arguments: args.cloned(),
            result: None,
        },
    }
}

#[derive(Debug, Clone)]
pub enum ParsedKimiEvent {
    Message(KimiMessage),
    Status(KimiStatusUpdate),
    RawLog(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiStatusUpdate {
    pub context_usage: f64,
    pub context_tokens: i64,
    pub max_context_tokens: i64,
    pub message_id: String,
    pub plan_mode: bool,
}

#[derive(Derivative, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[derivative(Debug, PartialEq)]
pub struct KimiCode {
    #[serde(default)]
    pub append_prompt: AppendPrompt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yolo: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
    #[serde(flatten)]
    pub cmd: CmdOverrides,
    #[serde(skip)]
    #[ts(skip)]
    #[derivative(Debug = "ignore", PartialEq = "ignore")]
    pub approvals: Option<Arc<dyn ExecutorApprovalService>>,
    /// Session ID for context management - generated internally and shared between spawn/normalize_logs
    #[serde(skip)]
    #[ts(skip)]
    #[derivative(Debug = "ignore", PartialEq = "ignore")]
    session_id: Arc<Mutex<Option<String>>>,
}

impl KimiCode {
    fn build_command_builder(&self) -> Result<CommandBuilder, CommandBuildError> {
        // Use print mode with JSON output for programmatic integration
        let mut builder =
            CommandBuilder::new("kimi").params(["--print", "--output-format=stream-json"]);

        // Add model if specified
        if let Some(model) = &self.model {
            builder = builder.extend_params(["--model", model]);
        }

        // YOLO mode - auto approve all actions
        if self.yolo.unwrap_or(false) {
            builder = builder.extend_params(["--yolo"]);
        }

        // Thinking mode (default to true for better code quality)
        let thinking = self.thinking.unwrap_or(true);
        if thinking {
            builder = builder.extend_params(["--thinking"]);
        } else {
            builder = builder.extend_params(["--no-thinking"]);
        }

        // Add work-dir to ensure session management works correctly
        builder = builder.extend_params(["--work-dir", "."]);

        apply_overrides(builder, &self.cmd)
    }

    /// Shared implementation for spawning with a session ID
    async fn spawn_with_session(
        &self,
        current_dir: &Path,
        prompt: &str,
        session_id: &str,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        // Always use --session for consistent context management
        let command_parts = self
            .build_command_builder()?
            .build_follow_up(&["--session".to_string(), session_id.to_string()])?;
        let (executable_path, args) = command_parts.into_resolved().await?;

        let combined_prompt = self.append_prompt.combine_prompt(prompt);

        let mut command = tokio::process::Command::new(executable_path);
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
            .with_profile(&self.cmd)
            .apply_to_command(&mut command);

        let mut child = command.group_spawn_no_window()?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.inner().stdin.take() {
            stdin.write_all(combined_prompt.as_bytes()).await?;
            stdin.shutdown().await?;
        }

        Ok(child.into())
    }
}

#[async_trait]
impl StandardCodingAgentExecutor for KimiCode {
    fn apply_overrides(&mut self, executor_config: &ExecutorConfig) {
        if let Some(model_id) = &executor_config.model_id {
            self.model = Some(model_id.clone());
        }
        if let Some(permission_policy) = executor_config.permission_policy.clone() {
            self.yolo = Some(matches!(
                permission_policy,
                crate::model_selector::PermissionPolicy::Auto
            ));
        }
    }

    fn use_approvals(&mut self, approvals: Arc<dyn ExecutorApprovalService>) {
        self.approvals = Some(approvals);
    }

    async fn spawn(
        &self,
        current_dir: &Path,
        prompt: &str,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        // Generate a new session ID for this conversation
        let session_id = format!("vk-{}", Uuid::new_v4().to_string().replace('-', ""));

        // Store the session ID for later use in normalize_logs
        let mut stored = self.session_id.lock().await;
        *stored = Some(session_id.clone());
        drop(stored);

        // Use the shared implementation with the generated session ID
        self.spawn_with_session(current_dir, prompt, &session_id, env)
            .await
    }

    async fn spawn_follow_up(
        &self,
        current_dir: &Path,
        prompt: &str,
        session_id: &str,
        _reset_to_message_id: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        // Store the session ID for later use in normalize_logs
        let mut stored = self.session_id.lock().await;
        *stored = Some(session_id.to_string());
        drop(stored);

        // Use the shared implementation with the provided session ID
        self.spawn_with_session(current_dir, prompt, session_id, env)
            .await
    }

    fn normalize_logs(
        &self,
        msg_store: Arc<MsgStore>,
        _worktree_path: &Path,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let entry_index_provider = EntryIndexProvider::start_from(&msg_store);

        // Process stderr for auth errors
        let msg_store_stderr = msg_store.clone();
        let entry_index_stderr = EntryIndexProvider::start_from(&msg_store);
        let h1 = tokio::spawn(async move {
            let mut stderr = msg_store_stderr.stderr_chunked_stream();

            let mut processor = PlainTextLogProcessor::builder()
                .normalized_entry_producer(Box::new(|content: String| NormalizedEntry {
                    timestamp: None,
                    entry_type: NormalizedEntryType::ErrorMessage {
                        error_type: NormalizedEntryError::Other,
                    },
                    content: strip_ansi_escapes::strip_str(&content),
                    metadata: None,
                }))
                .time_gap(Duration::from_secs(2))
                .index_provider(entry_index_stderr.clone())
                .build();

            while let Some(Ok(chunk)) = stderr.next().await {
                let content = strip_ansi_escapes::strip_str(&chunk);
                if content.contains(KIMI_AUTH_REQUIRED_MSG) {
                    let error_message = NormalizedEntry {
                        timestamp: None,
                        entry_type: NormalizedEntryType::ErrorMessage {
                            error_type: NormalizedEntryError::SetupRequired,
                        },
                        content: content.to_string(),
                        metadata: None,
                    };
                    let id = entry_index_stderr.next();
                    msg_store_stderr
                        .push_patch(ConversationPatch::add_normalized_entry(id, error_message));
                } else {
                    for patch in processor.process(chunk) {
                        msg_store_stderr.push_patch(patch);
                    }
                }
            }
        });

        // Get the session ID that was set during spawn
        let session_id_for_logs = self.session_id.clone();

        // Process stdout JSON lines
        let h2 = tokio::spawn(async move {
            // Wait for session_id to be set by spawn() using a short retry loop
            // instead of fixed sleep for faster response
            let session_id = {
                let mut id = None;
                for _ in 0..50 {
                    // Max 500ms wait
                    if let Some(sid) = session_id_for_logs.lock().await.as_ref() {
                        id = Some(sid.clone());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                id
            };
            if let Some(sid) = session_id {
                msg_store.push_session_id(sid);
            }

            let mut lines = msg_store.stdout_lines_stream();

            let mut current_thinking: Option<String> = None;
            let mut current_message: Option<String> = None;
            let mut thinking_index: Option<usize> = None;
            let mut message_index: Option<usize> = None;
            // Track tool calls to match with tool results
            let mut pending_tool_calls: std::collections::HashMap<String, (String, usize)> =
                std::collections::HashMap::new();

            while let Some(Ok(line)) = lines.next().await {
                if line.trim().is_empty() {
                    continue;
                }

                // Parse Kimi JSON output
                match parse_kimi_line(&line) {
                    Some(ParsedKimiEvent::Message(msg)) => {
                        match msg.role {
                            KimiRole::Assistant => {
                                // Convert content to blocks (handles both string and array formats)
                                let content_blocks = msg.content.into_blocks();
                                for block in content_blocks {
                                    match block {
                                        KimiContentBlock::Think { think, .. } => {
                                            // Skip thinking blocks that just describe tool calls
                                            // if they are followed by actual tool_calls
                                            if msg.tool_calls.is_empty()
                                                || think.len() < 100
                                                || !think.contains("tool")
                                            {
                                                current_message = None;
                                                message_index = None;

                                                if current_thinking.is_none() {
                                                    current_thinking = Some(String::new());
                                                    thinking_index =
                                                        Some(entry_index_provider.next());
                                                }
                                                if let Some(ref mut t) = current_thinking {
                                                    t.push_str(&think);
                                                    let entry = NormalizedEntry {
                                                        timestamp: None,
                                                        entry_type: NormalizedEntryType::Thinking,
                                                        content: t.clone(),
                                                        metadata: None,
                                                    };
                                                    if let Some(idx) = thinking_index {
                                                        if t.len() == think.len() {
                                                            msg_store.push_patch(
                                                                ConversationPatch::add_normalized_entry(
                                                                    idx, entry,
                                                                ),
                                                            );
                                                        } else {
                                                            msg_store.push_patch(
                                                                ConversationPatch::replace(idx, entry),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        KimiContentBlock::Text { text } => {
                                            current_thinking = None;
                                            thinking_index = None;

                                            if current_message.is_none() {
                                                current_message = Some(String::new());
                                                message_index = Some(entry_index_provider.next());
                                            }
                                            if let Some(ref mut m) = current_message {
                                                m.push_str(&text);
                                                let entry = NormalizedEntry {
                                                    timestamp: None,
                                                    entry_type:
                                                        NormalizedEntryType::AssistantMessage,
                                                    content: m.clone(),
                                                    metadata: None,
                                                };
                                                if let Some(idx) = message_index {
                                                    if m.len() == text.len() {
                                                        msg_store.push_patch(
                                                            ConversationPatch::add_normalized_entry(
                                                                idx, entry,
                                                            ),
                                                        );
                                                    } else {
                                                        msg_store.push_patch(
                                                            ConversationPatch::replace(idx, entry),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Handle tool_calls - display as ToolUse entries
                                for tool_call in &msg.tool_calls {
                                    let tool_name = tool_call.function.name.clone();
                                    let tool_id = tool_call.id.clone();
                                    let arguments = tool_call.function.arguments.clone();

                                    let (display_name, parsed_args) =
                                        parse_tool_arguments(&tool_name, &arguments);

                                    let idx = entry_index_provider.next();
                                    let summary = display_name.unwrap_or_else(|| {
                                        format!("{} tool", tool_name)
                                    });

                                    let entry = NormalizedEntry {
                                        timestamp: None,
                                        entry_type: NormalizedEntryType::ToolUse {
                                            tool_name: summary.clone(),
                                            action_type: parse_action_type(&tool_name, parsed_args.as_ref()),
                                            status: crate::logs::ToolStatus::Success,
                                        },
                                        content: summary.clone(),
                                        metadata: Some(serde_json::json!({
                                            "tool_id": tool_id.clone(),
                                            "tool_name": tool_name,
                                            "arguments": arguments,
                                        })),
                                    };
                                    msg_store.push_patch(
                                        ConversationPatch::add_normalized_entry(idx, entry),
                                    );

                                    // Store for matching with tool result
                                    pending_tool_calls.insert(tool_id, (summary, idx));
                                }
                            }
                            KimiRole::Tool => {
                                // Tool result - extract from content (handles string or array)
                                let content_text: String = match &msg.content {
                                    KimiContent::Text(text) => text.clone(),
                                    KimiContent::Blocks(blocks) => blocks
                                        .iter()
                                        .map(|c| match c {
                                            KimiContentBlock::Text { text } => text.clone(),
                                            KimiContentBlock::Think { think, .. } => think.clone(),
                                        })
                                        .collect(),
                                };

                                // Find matching tool call if any (using first pending as approximation)
                                // Kimi doesn't consistently link tool results to calls via ID in the stream
                                let tool_name = pending_tool_calls
                                    .values()
                                    .next()
                                    .map(|(name, _)| name.clone())
                                    .unwrap_or_else(|| "Tool".to_string());

                                // Add as a tool result entry with proper tool_use type
                                let idx = entry_index_provider.next();
                                let display_text = if content_text.len() > 500 {
                                    format!(
                                        "{}... [{} more chars]",
                                        &content_text[..500],
                                        content_text.len() - 500
                                    )
                                } else {
                                    content_text.clone()
                                };

                                let entry = NormalizedEntry {
                                    timestamp: None,
                                    entry_type: NormalizedEntryType::ToolUse {
                                        tool_name: tool_name.clone(),
                                        action_type: crate::logs::ActionType::Other {
                                            description: "Tool execution result".to_string(),
                                        },
                                        status: crate::logs::ToolStatus::Success,
                                    },
                                    content: display_text,
                                    metadata: Some(serde_json::json!({
                                        "is_result": true,
                                    })),
                                };
                                msg_store.push_patch(
                                    ConversationPatch::add_normalized_entry(idx, entry),
                                );
                            }
                            _ => {}
                        }
                    }
                    Some(ParsedKimiEvent::Status(update)) => {
                        tracing::debug!(
                            "Kimi context usage: {:.1}% ({}/{} tokens)",
                            update.context_usage * 100.0,
                            update.context_tokens,
                            update.max_context_tokens
                        );
                        // Also push to UI as a system message so users can see context usage
                        let idx = entry_index_provider.next();
                        let entry = NormalizedEntry {
                            timestamp: None,
                            entry_type: NormalizedEntryType::SystemMessage,
                            content: format!(
                                "📊 Context: {:.0}% ({}K / {}K tokens)",
                                update.context_usage * 100.0,
                                update.context_tokens / 1000,
                                update.max_context_tokens / 1000
                            ),
                            metadata: Some(serde_json::json!({
                                "context_usage": update.context_usage,
                                "context_tokens": update.context_tokens,
                                "max_context_tokens": update.max_context_tokens,
                                "message_id": update.message_id,
                            })),
                        };
                        msg_store.push_patch(ConversationPatch::add_normalized_entry(idx, entry));
                    }
                    Some(ParsedKimiEvent::RawLog(text)) => {
                        // Check if it looks like a tool call or other structured output
                        if text.starts_with("TurnBegin") || text.starts_with("TurnEnd") {
                            // Reset state on turn boundaries
                            if text.starts_with("TurnEnd") {
                                current_thinking = None;
                                current_message = None;
                                thinking_index = None;
                                message_index = None;
                            }
                        } else if !text.is_empty() {
                            // Display as system message
                            let idx = entry_index_provider.next();
                            let entry = NormalizedEntry {
                                timestamp: None,
                                entry_type: NormalizedEntryType::SystemMessage,
                                content: text,
                                metadata: None,
                            };
                            msg_store
                                .push_patch(ConversationPatch::add_normalized_entry(idx, entry));
                        }
                    }
                    None => {
                        tracing::debug!("Failed to parse Kimi line: {}", line);
                    }
                }
            }
        });

        vec![h1, h2]
    }

    fn default_mcp_config_path(&self) -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|home| home.join(".kimi").join("config.toml"))
    }

    fn get_availability_info(&self) -> AvailabilityInfo {
        match which::which("kimi") {
            Ok(_) => AvailabilityInfo::InstallationFound,
            Err(_) => AvailabilityInfo::NotFound,
        }
    }

    fn get_preset_options(&self) -> ExecutorConfig {
        // NOTE: Kimi --print mode does not support interactive approvals,
        // so we always use Auto permission policy regardless of yolo setting.
        // The yolo field is kept for API compatibility but has no practical effect.
        ExecutorConfig {
            executor: BaseCodingAgent::KimiCode,
            variant: None,
            model_id: self.model.clone(),
            agent_id: None,
            reasoning_id: None,
            permission_policy: Some(PermissionPolicy::Auto),
        }
    }

    async fn discover_options(
        &self,
        _workdir: Option<&Path>,
        _repo_path: Option<&Path>,
    ) -> Result<futures::stream::BoxStream<'static, json_patch::Patch>, ExecutorError> {
        let options = ExecutorDiscoveredOptions {
            model_selector: ModelSelectorConfig {
                models: vec![ModelInfo {
                    id: "kimi-code/kimi-for-coding".to_string(),
                    name: "Kimi for Coding".to_string(),
                    provider_id: None,
                    reasoning_options: vec![],
                }],
                default_model: Some("kimi-code/kimi-for-coding".to_string()),
                // NOTE: Only Auto is supported since Kimi --print mode does not support
                // interactive approvals. Supervised would be misleading as it has no effect.
                permissions: vec![PermissionPolicy::Auto],
                ..Default::default()
            },
            ..Default::default()
        };
        Ok(Box::pin(futures::stream::once(async move {
            patch::executor_discovered_options(options)
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kimi_code_default() {
        let kimi = KimiCode {
            append_prompt: AppendPrompt(None),
            model: None,
            yolo: None,
            thinking: None,
            cmd: CmdOverrides::default(),
            approvals: None,
            session_id: Arc::new(Mutex::new(None)),
        };

        assert!(kimi.thinking.is_none());
        assert!(kimi.yolo.is_none());
    }

    #[test]
    fn test_kimi_code_command_builder() {
        let kimi = KimiCode {
            append_prompt: AppendPrompt(None),
            model: Some("kimi-for-coding".to_string()),
            yolo: Some(true),
            thinking: Some(false),
            cmd: CmdOverrides::default(),
            approvals: None,
            session_id: Arc::new(Mutex::new(None)),
        };

        let builder = kimi.build_command_builder().unwrap();
        let cmd = builder.build_initial().unwrap();

        let cmd_str = format!("{:?}", cmd);
        assert!(cmd_str.contains("kimi"));
        assert!(cmd_str.contains("--print"));
        assert!(cmd_str.contains("--output-format=stream-json"));
        assert!(cmd_str.contains("--model"));
        assert!(cmd_str.contains("kimi-for-coding"));
        assert!(cmd_str.contains("--yolo"));
        assert!(cmd_str.contains("--no-thinking"));
        assert!(!cmd_str.contains("acp")); // No longer using ACP mode
    }

    #[test]
    fn test_kimi_json_parsing() {
        // Test new role-based format for assistant messages
        let json_line = r#"{"role":"assistant","content":[{"type":"text","text":"Hello world"}]}"#;
        let msg = serde_json::from_str::<KimiMessage>(json_line).unwrap();
        assert!(matches!(msg.role, KimiRole::Assistant));
        let blocks = msg.content.into_blocks();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            KimiContentBlock::Text { text } => assert_eq!(text, "Hello world"),
            _ => panic!("Expected Text block"),
        }

        // Test thinking format
        let json_think = r#"{"role":"assistant","content":[{"type":"think","think":"Let me think about this"}]}"#;
        let msg_think = serde_json::from_str::<KimiMessage>(json_think).unwrap();
        assert!(matches!(msg_think.role, KimiRole::Assistant));
        let blocks_think = msg_think.content.into_blocks();
        match &blocks_think[0] {
            KimiContentBlock::Think { think, .. } => assert_eq!(think, "Let me think about this"),
            _ => panic!("Expected Think block"),
        }

        // Test tool result format with string content (the problematic format)
        let json_tool_str = r#"{"role":"tool","content":"Tool output as string","tool_call_id":"tool_123"}"#;
        let msg_tool_str = serde_json::from_str::<KimiMessage>(json_tool_str).unwrap();
        assert!(matches!(msg_tool_str.role, KimiRole::Tool));
        match msg_tool_str.content {
            KimiContent::Text(text) => assert_eq!(text, "Tool output as string"),
            _ => panic!("Expected Text content for tool result"),
        }
        assert_eq!(msg_tool_str.tool_call_id, Some("tool_123".to_string()));

        // Test tool result format with array content
        let json_tool_arr = r#"{"role":"tool","content":[{"type":"text","text":"Tool output here"}]}"#;
        let msg_tool_arr = serde_json::from_str::<KimiMessage>(json_tool_arr).unwrap();
        assert!(matches!(msg_tool_arr.role, KimiRole::Tool));

        // Test parse_kimi_line
        let event = parse_kimi_line(json_line).unwrap();
        match event {
            ParsedKimiEvent::Message(msg) => assert!(matches!(msg.role, KimiRole::Assistant)),
            _ => panic!("Expected Message event"),
        }
    }
}
