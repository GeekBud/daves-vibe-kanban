use axum::{
    Router,
    extract::{Query, State},
    response::Json as ResponseJson,
    routing::get,
};
use deployment::Deployment;
use serde::Deserialize;
use std::path::Path;
use ts_rs::TS;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Deserialize)]
pub struct SkillsQuery {
    pub workspace_id: Option<Uuid>,
}

#[derive(Debug, Clone, serde::Serialize, TS)]
#[ts(export)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Default built-in skills
fn builtin_skills() -> Vec<Skill> {
    vec![
        Skill {
            id: "ask".to_string(),
            name: "Ask".to_string(),
            description: "Ask the codebase questions".to_string(),
            source: Some("built-in".to_string()),
        },
        Skill {
            id: "git-ai-search".to_string(),
            name: "Git AI Search".to_string(),
            description: "Search git history with AI".to_string(),
            source: Some("built-in".to_string()),
        },
        Skill {
            id: "prompt-analysis".to_string(),
            name: "Prompt Analysis".to_string(),
            description: "Analyze prompt patterns".to_string(),
            source: Some("built-in".to_string()),
        },
        Skill {
            id: "daves-vibe-guide".to_string(),
            name: "Daves Vibe Guide".to_string(),
            description: "Fork-Track system guide".to_string(),
            source: Some("built-in".to_string()),
        },
        Skill {
            id: "daves-vibe-check".to_string(),
            name: "Daves Vibe Check".to_string(),
            description: "Check upstream updates".to_string(),
            source: Some("built-in".to_string()),
        },
        Skill {
            id: "daves-vibe-sync".to_string(),
            name: "Daves Vibe Sync".to_string(),
            description: "Sync with upstream".to_string(),
            source: Some("built-in".to_string()),
        },
        Skill {
            id: "daves-vibe-review".to_string(),
            name: "Daves Vibe Review".to_string(),
            description: "Review upstream commits".to_string(),
            source: Some("built-in".to_string()),
        },
        Skill {
            id: "daves-vibe-mod".to_string(),
            name: "Daves Vibe Mod".to_string(),
            description: "Record mod decisions".to_string(),
            source: Some("built-in".to_string()),
        },
    ]
}

/// Load local skills from .cursor/skills/ directory
async fn load_local_skills(
    deployment: &DeploymentImpl,
    workspace_id: Option<Uuid>,
) -> Vec<Skill> {
    let mut local_skills = Vec::new();

    tracing::info!("Loading local skills, workspace_id: {:?}", workspace_id);

    // Try to get the workspace path
    let workspace_path = if let Some(ws_id) = workspace_id {
        let pool = &deployment.db().pool;
        match db::models::workspace::Workspace::find_by_id(pool, ws_id).await {
            Ok(Some(workspace)) => {
                tracing::info!("Found workspace, container_ref: {:?}", workspace.container_ref);
                workspace.container_ref.clone()
            }
            Ok(None) => {
                tracing::warn!("Workspace not found: {}", ws_id);
                None
            }
            Err(e) => {
                tracing::error!("Error loading workspace: {}", e);
                None
            }
        }
    } else {
        tracing::info!("No workspace_id provided");
        None
    };

    // If we have a workspace path, look for .cursor/skills/
    if let Some(base_path) = workspace_path {
        let skills_dir = Path::new(&base_path).join(".cursor").join("skills");
        
        tracing::info!("Looking for skills at: {:?}", skills_dir);
        
        if skills_dir.exists() && skills_dir.is_dir() {
            tracing::info!("Skills directory exists");
            match tokio::fs::read_dir(&skills_dir).await {
                Ok(entries) => {
                    let mut entries = entries;
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let path = entry.path();
                        tracing::debug!("Found entry: {:?}", path);
                        if path.is_file() {
                            if let Some(ext) = path.extension() {
                                if ext == "md" || ext == "txt" {
                                    if let Some(stem) = path.file_stem() {
                                        let id = stem.to_string_lossy().to_string();
                                        
                                        tracing::info!("Loading skill file: {:?}", path);
                                        
                                        // Read first line as description
                                        let description = match tokio::fs::read_to_string(&path).await {
                                            Ok(content) => {
                                                let desc = content.lines().next()
                                                    .map(|line| line.trim().trim_start_matches("# ").to_string())
                                                    .unwrap_or_else(|| format!("Skill: {}", id));
                                                tracing::debug!("Read description: {}", desc);
                                                desc
                                            }
                                            Err(e) => {
                                                tracing::error!("Error reading file {:?}: {}", path, e);
                                                format!("Skill: {}", id)
                                            }
                                        };
                                        
                                        let name = id.chars().enumerate().map(|(i, c)| {
                                            if i == 0 { c.to_uppercase().next().unwrap_or(c) }
                                            else if c == '_' || c == '-' { ' ' }
                                            else { c }
                                        }).collect::<String>();
                                        
                                        local_skills.push(Skill {
                                            id: format!("local:{}", id),
                                            name,
                                            description,
                                            source: Some("local".to_string()),
                                        });
                                    }
                                } else {
                                    tracing::debug!("Skipping non-md/txt file: {:?}", path);
                                }
                            }
                        } else {
                            tracing::debug!("Skipping directory: {:?}", path);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Error reading skills directory: {}", e);
                }
            }
        } else {
            tracing::info!("Skills directory does not exist: {:?}", skills_dir);
        }
    }

    tracing::info!("Loaded {} local skills", local_skills.len());
    local_skills
}

pub async fn get_skills(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<SkillsQuery>,
) -> Result<ResponseJson<Vec<Skill>>, ApiError> {
    let mut skills = builtin_skills();
    
    // Load local skills
    let local_skills = load_local_skills(&deployment, query.workspace_id).await;
    skills.extend(local_skills);
    
    Ok(ResponseJson(skills))
}

pub fn router(deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route("/skills", get(get_skills))
        .with_state(deployment.clone())
}
