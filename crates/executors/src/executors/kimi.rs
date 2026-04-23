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
    #[serde(other)]
    Unknown,
} // FORK-MOD-007: Added Unknown fallback for robust parsing

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
    #[serde(other)]
    Unknown,
}

/// Kimi tool call structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiToolCall {
    // FORK-MOD-011: Kimi 1.36.0+ uses "call_type" in stream-json output; older versions used "type"
    #[serde(rename = "type", alias = "call_type")]
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

    // FORK-MOD-007: Best-effort fallback for role-based messages that fail strict
    // KimiMessage parsing (e.g. unknown content block types or new roles in newer CLI)
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
        if let Some(role_str) = value.get("role").and_then(|v| v.as_str()) {
            let role = match role_str {
                "assistant" => KimiRole::Assistant,
                "user" => KimiRole::User,
                "tool" => KimiRole::Tool,
                "system" => KimiRole::System,
                _ => KimiRole::Unknown,
            };

            let mut blocks = Vec::new();
            if let Some(content) = value.get("content") {
                if let Some(text) = content.as_str() {
                    blocks.push(KimiContentBlock::Text { text: text.to_string() });
                } else if let Some(arr) = content.as_array() {
                    for item in arr {
                        if let Some(typ) = item.get("type").and_then(|v| v.as_str()) {
                            match typ {
                                "think" | "thinking" => {
                                    if let Some(text) = item
                                        .get("think")
                                        .and_then(|v| v.as_str())
                                        .or_else(|| item.get("thinking").and_then(|v| v.as_str()))
                                    {
                                        blocks.push(KimiContentBlock::Think {
                                            think: text.to_string(),
                                            encrypted: None,
                                        });
                                    }
                                }
                                "text" => {
                                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                        blocks.push(KimiContentBlock::Text {
                                            text: text.to_string(),
                                        });
                                    }
                                }
                                _ => {
                                    blocks.push(KimiContentBlock::Unknown);
                                }
                            }
                        }
                    }
                }
            }

            let mut tool_calls = Vec::new();
            if let Some(tcs) = value.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tcs {
                    if let (Some(id), Some(name), Some(arguments)) = (
                        tc.get("id").and_then(|v| v.as_str()),
                        tc.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str()),
                        tc.get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str()),
                    ) {
                        tool_calls.push(KimiToolCall {
                            call_type: "function".to_string(),
                            id: id.to_string(),
                            function: KimiFunctionCall {
                                name: name.to_string(),
                                arguments: arguments.to_string(),
                            },
                        });
                    }
                }
            }

            return Some(ParsedKimiEvent::Message(KimiMessage {
                role,
                content: KimiContent::Blocks(blocks),
                tool_calls,
                tool_call_id: value
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            }));
        }

        // FORK-MOD-006: Handle new event-based formats (Thought, Message, ToolCall, etc.)
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

// FORK-MOD-012: Truncate tool result display text without allocating full large strings.
// When Kimi returns very long tool outputs (e.g. large shell command results),
// we only need the first 500 chars for display. This avoids O(n) allocation of
// multi-megabyte strings that are immediately discarded.
fn truncate_tool_result(content: &KimiContent) -> String {
    const MAX_CHARS: usize = 500;
    match content {
        KimiContent::Text(text) => {
            let mut count = 0;
            let mut truncated = String::new();
            for ch in text.chars() {
                if count < MAX_CHARS {
                    truncated.push(ch);
                }
                count += 1;
            }
            if count > MAX_CHARS {
                format!("{}... [{} more chars]", truncated, count - MAX_CHARS)
            } else {
                truncated
            }
        }
        KimiContent::Blocks(blocks) => {
            let mut count = 0;
            let mut truncated = String::new();
            for block in blocks {
                let text = match block {
                    KimiContentBlock::Text { text } => text.as_str(),
                    KimiContentBlock::Think { think, .. } => think.as_str(),
                    KimiContentBlock::Unknown => continue,
                };
                for ch in text.chars() {
                    if count < MAX_CHARS {
                        truncated.push(ch);
                    }
                    count += 1;
                }
            }
            if count > MAX_CHARS {
                format!("{}... [{} more chars]", truncated, count - MAX_CHARS)
            } else {
                truncated
            }
        }
    }
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
                                            // FORK-MOD-007: Skip thinking blocks that just describe
                                        // tool calls if they are followed by actual tool_calls.
                                        // Previous logic was inverted, showing useless tool-call
                                        // planning thoughts in the UI.
                                        let should_skip = !msg.tool_calls.is_empty()
                                            && think.len() >= 100
                                            && think.contains("tool");
                                            if !should_skip {
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
                                        KimiContentBlock::Unknown => {
                                            // Silently ignore unknown block types
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
                                // FORK-MOD-012: Avoid O(n) allocation of full large strings.
                                // Tool results from Shell/Grep/etc. can be multi-MB; we only need
                                // first 500 chars for display.
                                let display_text = truncate_tool_result(&msg.content);

                                // Find matching tool call if any (using first pending as approximation)
                                // Kimi doesn't consistently link tool results to calls via ID in the stream
                                let tool_name = pending_tool_calls
                                    .values()
                                    .next()
                                    .map(|(name, _)| name.clone())
                                    .unwrap_or_else(|| "Tool".to_string());

                                // Add as a tool result entry with proper tool_use type
                                let idx = entry_index_provider.next();

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

        // Test tool_calls with "call_type" field (Kimi 1.36.0+ format)
        let json_with_call_type = r#"{"role":"assistant","content":[{"type":"think","think":"冲突已解决。","encrypted":null}],"tool_calls":[{"call_type":"function","id":"call_123","function":{"name":"Shell","arguments":"{}"}}],"tool_call_id":null}"#;
        let msg_call_type = serde_json::from_str::<KimiMessage>(json_with_call_type).unwrap();
        assert_eq!(msg_call_type.tool_calls.len(), 1);
        assert_eq!(msg_call_type.tool_calls[0].call_type, "function");
        assert_eq!(msg_call_type.tool_calls[0].function.name, "Shell");
    }

    #[tokio::test]
    async fn test_kimi_normalize_logs_produces_assistant_message() {
        use crate::executors::StandardCodingAgentExecutor;
        use tokio::time::{sleep, Duration};
        let msg_store = Arc::new(MsgStore::new());

        // Simulate the user's actual JSON sequence from Kimi 1.36.0
        let lines = vec![
            r#"{"role":"assistant","content":[{"type":"think","think":"冲突已解决。现在需要添加文件并完成 merge commit。","encrypted":null}],"tool_calls":[{"call_type":"function","id":"call_41c36c5e-aab2-4b30-81e5-1d62a5d3cab6","function":{"name":"Shell","arguments":"{\"command\":\"cd moyi-recommend && git add -A && git status\"}"}}],"tool_call_id":null}"#,
            r#"{"role":"tool","content":[{"type":"text","text":"<system>Command executed successfully.</system>\n<output>On branch vk/c106-moyi-recommend-m...nothing to commit, working tree clean</output>"}],"tool_call_id":"call_41c36c5e-aab2-4b30-81e5-1d62a5d3cab6"}"#,
            r#"{"role":"assistant","content":[{"type":"think","think":"merge 中有很多 origin/master 带来的新文件和修改，这是正常的。我需要完成 merge commit。\n\n冲突已经解决了（NydusTopics.java），现在只需要 `git commit` 完成 merge。","encrypted":null}],"tool_calls":[{"call_type":"function","id":"call_44177423-b62a-4653-b61e-4ca81f6c1665","function":{"name":"Shell","arguments":"{\"command\":\"cd moyi-recommend && git commit -m \\\"Merge origin/master into vk/c106-moyi-recommend-m\\\"\"}"}}],"tool_call_id":null}"#,
            r#"{"role":"tool","content":[{"type":"text","text":"<system>Command executed successfully.</system>\n<output>[vk/c106-moyi-recommend-m 64578ec] Merge origin/master into vk/c106-moyi-recommend-m</output>"}],"tool_call_id":"call_44177423-b62a-4653-b61e-4ca81f6c1665"}"#,
            r#"{"role":"assistant","content":[{"type":"think","think":"Merge 已成功完成。当前分支 `vk/c106-moyi-recommend-m` 现在已经基于最新的 `origin/master` 了。\n\n让我给用户一个简洁的总结。","encrypted":null}],"tool_calls":[],"tool_call_id":null}"#,
            r#"{"role":"assistant","content":[{"type":"text","text":"Merge 已完成。当前分支 `vk/c106-moyi-recommend-m` 现已基于最新的 `origin/master`（`c9750907f`）。\n\n**冲突情况**：仅 `NydusTopics.java` 一处冲突，位于 `moyi-recommend` 目录下，已按 origin/master 的版本自动解决（保留了你的修改）。\n\n**当前状态**：\n- 所有更改已暂存（`git add -A`）\n- 工作树干净（`nothing to commit, working tree clean`）\n- 分支 `vk/c106-moyi-recommend-m` 已基于 `origin/master`"}],"tool_calls":[],"tool_call_id":null}"#,
        ];

        for line in &lines {
            msg_store.push_stdout(format!("{line}\n"));
        }
        msg_store.push_finished();

        let executor = KimiCode {
            append_prompt: AppendPrompt(None),
            model: None,
            yolo: None,
            thinking: None,
            cmd: CmdOverrides::default(),
            approvals: None,
            session_id: Arc::new(Mutex::new(Some("test-session".to_string()))),
        };

        let handles = executor.normalize_logs(msg_store.clone(), Path::new("/tmp"));
        for handle in handles {
            handle.await.unwrap();
        }

        // Allow a brief moment for any final patches to be applied
        sleep(Duration::from_millis(100)).await;

        let history = msg_store.get_history();
        let patches: Vec<_> = history.iter().filter_map(|m| match m {
            workspace_utils::log_msg::LogMsg::JsonPatch(p) => Some(p),
            _ => None,
        }).collect();

        // Count entry types in patches
        let mut assistant_msg_count = 0;
        let mut thinking_count = 0;
        let mut tool_use_count = 0;
        let mut assistant_content = String::new();

        for patch in &patches {
            if let Some((_, entry)) = crate::logs::utils::patch::extract_normalized_entry_from_patch(patch) {
                match entry.entry_type {
                    NormalizedEntryType::AssistantMessage => {
                        assistant_msg_count += 1;
                        assistant_content = entry.content;
                    }
                    NormalizedEntryType::Thinking => thinking_count += 1,
                    NormalizedEntryType::ToolUse { .. } => tool_use_count += 1,
                    _ => {}
                }
            }
        }

        assert!(thinking_count >= 2, "Expected at least 2 thinking entries, got {}", thinking_count);
        assert!(tool_use_count >= 2, "Expected at least 2 tool_use entries, got {}", tool_use_count);
        assert_eq!(assistant_msg_count, 1, "Expected exactly 1 assistant_message, got {}", assistant_msg_count);
        assert!(assistant_content.contains("Merge 已完成"), "Assistant message should contain the final summary, got: {}", assistant_content);
    }

    #[tokio::test]
    async fn test_replay_from_actual_log_file() {
        let log_path = "/Users/lianghusile/dave/appData/daves-vibe-kanban/sessions/b5/b5553ef9-982e-4365-b1ac-15f5adabeac7/processes/55d38259-dded-41be-88d8-0f6fe9d154f8.jsonl";
        let content = tokio::fs::read_to_string(log_path).await.unwrap();

        let msg_store = Arc::new(MsgStore::new());
        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            let msg: workspace_utils::log_msg::LogMsg = serde_json::from_str(line).unwrap();
            msg_store.push(msg);
        }
        msg_store.push_finished();

        let executor = KimiCode {
            append_prompt: AppendPrompt(None),
            model: None,
            yolo: None,
            thinking: None,
            cmd: CmdOverrides::default(),
            approvals: None,
            session_id: Arc::new(Mutex::new(Some("test-session".to_string()))),
        };

        let handles = executor.normalize_logs(msg_store.clone(), Path::new("/tmp"));
        for handle in handles {
            handle.await.unwrap();
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let history = msg_store.get_history();
        let patches: Vec<_> = history.iter().filter_map(|m| match m {
            workspace_utils::log_msg::LogMsg::JsonPatch(p) => Some(p),
            _ => None,
        }).collect();

        let mut assistant_count = 0;
        let mut thinking_count = 0;
        let mut tool_use_count = 0;

        for patch in &patches {
            if let Some((_, entry)) = crate::logs::utils::patch::extract_normalized_entry_from_patch(patch) {
                match entry.entry_type {
                    NormalizedEntryType::AssistantMessage => assistant_count += 1,
                    NormalizedEntryType::Thinking => thinking_count += 1,
                    NormalizedEntryType::ToolUse { .. } => tool_use_count += 1,
                    _ => {}
                }
            }
        }

        println!("REPLAY RESULT: patches={} assistant={} thinking={} tool_use={}", patches.len(), assistant_count, thinking_count, tool_use_count);

        assert!(assistant_count >= 1, "Expected at least 1 assistant message, got {}", assistant_count);
    }

    #[test]
    fn test_truncate_tool_result_short() {
        let content = KimiContent::Text("Hello world".to_string());
        assert_eq!(truncate_tool_result(&content), "Hello world");
    }

    #[test]
    fn test_truncate_tool_result_exact_500() {
        let text = "a".repeat(500);
        let content = KimiContent::Text(text.clone());
        assert_eq!(truncate_tool_result(&content), text);
    }

    #[test]
    fn test_truncate_tool_result_long() {
        let text = "x".repeat(1000);
        let content = KimiContent::Text(text);
        let result = truncate_tool_result(&content);
        assert!(result.starts_with("x"), "Should start with x");
        assert!(result.ends_with("... [500 more chars]"), "Should indicate truncation: got {}", result);
        // Count visible chars before "..."
        let prefix_len = result.find("...").unwrap();
        assert_eq!(prefix_len, 500, "Should have 500 visible chars");
    }

    #[test]
    fn test_truncate_tool_result_blocks() {
        let blocks = vec![
            KimiContentBlock::Text { text: "Hello ".to_string() },
            KimiContentBlock::Think { think: "world".to_string(), encrypted: None },
        ];
        let content = KimiContent::Blocks(blocks);
        assert_eq!(truncate_tool_result(&content), "Hello world");
    }

    #[test]
    fn test_truncate_tool_result_blocks_long() {
        let blocks = vec![
            KimiContentBlock::Text { text: "a".repeat(300) },
            KimiContentBlock::Text { text: "b".repeat(300) },
            KimiContentBlock::Text { text: "c".repeat(300) },
        ];
        let content = KimiContent::Blocks(blocks);
        let result = truncate_tool_result(&content);
        assert!(result.starts_with("a"), "Should start with a");
        assert!(result.ends_with("... [400 more chars]"), "Should indicate 400 more chars: got {}", result);
    }
}
