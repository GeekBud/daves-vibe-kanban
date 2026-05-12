use serde::Deserialize;
use std::{collections::HashMap, path::PathBuf, sync::OnceLock};

static CONFIG: OnceLock<DaveEnvConfig> = OnceLock::new();

const DEFAULT_CONFIG_PATH: &str =
    "/Users/lianghusile/dave/appData/daves-vibe-kanban/daves_env_config.json";

fn config_path() -> PathBuf {
    std::env::var("DAVES_ENV_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CONFIG_PATH))
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DaveEnvConfig {
    #[serde(default)]
    pub paths: PathConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub remote: RemoteConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    #[serde(default)]
    pub frontend: FrontendConfig,
    #[serde(default)]
    pub features: FeatureConfig,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PathConfig {
    pub asset_dir: Option<String>,
    pub cache_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServerConfig {
    pub host: Option<String>,
    pub port: Option<String>,
    pub preview_proxy_port: Option<String>,
    pub allowed_origins: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RemoteConfig {
    pub vk_shared_api_base: Option<String>,
    pub vk_shared_relay_api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TelemetryConfig {
    pub posthog_api_key: Option<String>,
    pub posthog_api_endpoint: Option<String>,
    pub sentry_dsn: Option<String>,
    pub sentry_dsn_remote: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FrontendConfig {
    pub frontend_port: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FeatureConfig {
    pub disable_worktree_cleanup: Option<bool>,
}

pub fn load_config() -> &'static DaveEnvConfig {
    CONFIG.get_or_init(|| {
        let path = config_path();
        if !path.exists() {
            return DaveEnvConfig::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("Warning: failed to parse daves_env_config.json: {e}");
                    DaveEnvConfig::default()
                }
            },
            Err(e) => {
                eprintln!("Warning: failed to read daves_env_config.json: {e}");
                DaveEnvConfig::default()
            }
        }
    })
}

/// Get a string value from config, falling back to an environment variable.
pub fn resolve_string(config_value: Option<&str>, env_name: &str) -> Option<String> {
    config_value
        .map(|s| {
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        })
        .flatten()
        .or_else(|| std::env::var(env_name).ok().filter(|s| !s.is_empty()))
}

/// FORK-MOD-014: 全局唯一日志级别入口。
///
/// 仅识别 `VK_LOG_LEVEL`（由 npx-cli 在 `--debug` 时设置为 "debug"），
/// 其它任何来源（RUST_LOG、config 文件等）一律忽略，缺省一律 `"info"`。
///
/// 这样保证从 `node bin/cli.js [--debug]` 启动时行为完全可预测，
/// 不会被环境/配置文件意外覆盖。
pub fn resolve_log_level() -> String {
    std::env::var("VK_LOG_LEVEL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "info".to_string())
}
