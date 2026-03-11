use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use derivative::Derivative;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use workspace_utils::msg_store::MsgStore;

pub use super::acp::AcpAgentHarness;
use crate::{
    approvals::ExecutorApprovalService,
    command::{CmdOverrides, CommandBuildError, CommandBuilder, apply_overrides},
    env::ExecutionEnv,
    executor_discovery::ExecutorDiscoveredOptions,
    executors::{
        AppendPrompt, AvailabilityInfo, BaseCodingAgent, ExecutorError, SpawnedChild,
        StandardCodingAgentExecutor,
    },
    logs::utils::patch,
    model_selector::{ModelInfo, ModelSelectorConfig, PermissionPolicy},
    profile::ExecutorConfig,
};

const SUPPRESSED_STDERR_PATTERNS: &[&str] = &[
    "was started but never ended. Skipping metrics.",
    "YOLO mode is enabled. All tool calls will be automatically approved.",
];

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
}

impl KimiCode {
    fn build_command_builder(&self) -> Result<CommandBuilder, CommandBuildError> {
        // kimi is installed locally, not via npx
        let mut builder = CommandBuilder::new("kimi");

        // Add model if specified
        if let Some(model) = &self.model {
            builder = builder.extend_params(["--model", model.as_str()]);
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

        // Enable ACP (Agent Communication Protocol) for integration
        builder = builder.extend_params(["--acp"]);

        apply_overrides(builder, &self.cmd)
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
        let harness = AcpAgentHarness::with_session_namespace("kimi_sessions");
        let combined_prompt = self.append_prompt.combine_prompt(prompt);
        let kimi_command = self.build_command_builder()?.build_initial()?;
        let approvals = if self.yolo.unwrap_or(false) {
            None
        } else {
            self.approvals.clone()
        };
        harness
            .spawn_with_command(
                current_dir,
                combined_prompt,
                kimi_command,
                env,
                &self.cmd,
                approvals,
            )
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
        let harness = AcpAgentHarness::with_session_namespace("kimi_sessions");
        let combined_prompt = self.append_prompt.combine_prompt(prompt);
        let kimi_command = self.build_command_builder()?.build_follow_up(&[])?;
        let approvals = if self.yolo.unwrap_or(false) {
            None
        } else {
            self.approvals.clone()
        };
        harness
            .spawn_follow_up_with_command(
                current_dir,
                combined_prompt,
                session_id,
                kimi_command,
                env,
                &self.cmd,
                approvals,
            )
            .await
    }

    fn normalize_logs(
        &self,
        msg_store: Arc<MsgStore>,
        worktree_path: &Path,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        super::acp::normalize_logs_with_suppressed_stderr_patterns(
            msg_store,
            worktree_path,
            SUPPRESSED_STDERR_PATTERNS,
        )
    }

    fn default_mcp_config_path(&self) -> Option<std::path::PathBuf> {
        // Kimi stores config in ~/.kimi/config.toml
        dirs::home_dir().map(|home| home.join(".kimi").join("config.toml"))
    }

    fn get_availability_info(&self) -> AvailabilityInfo {
        // Check if kimi binary exists in PATH
        match which::which("kimi") {
            Ok(_) => AvailabilityInfo::InstallationFound,
            Err(_) => AvailabilityInfo::NotFound,
        }
    }

    fn get_preset_options(&self) -> ExecutorConfig {
        ExecutorConfig {
            executor: BaseCodingAgent::KimiCode,
            variant: None,
            model_id: self.model.clone(),
            agent_id: None,
            reasoning_id: None,
            permission_policy: Some(if self.yolo.unwrap_or(false) {
                PermissionPolicy::Auto
            } else {
                PermissionPolicy::Supervised
            }),
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
                permissions: vec![PermissionPolicy::Auto, PermissionPolicy::Supervised],
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
        };

        let builder = kimi.build_command_builder().unwrap();
        let cmd = builder.build_initial().unwrap();

        // Check that the command contains expected arguments
        let cmd_str = format!("{:?}", cmd);
        assert!(cmd_str.contains("kimi"));
        assert!(cmd_str.contains("--model"));
        assert!(cmd_str.contains("kimi-for-coding"));
        assert!(cmd_str.contains("--yolo"));
        assert!(cmd_str.contains("--no-thinking"));
        assert!(cmd_str.contains("--acp"));
    }
}
