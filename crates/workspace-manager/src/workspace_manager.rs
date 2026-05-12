use std::path::{Path, PathBuf};

use db::{
    DBService,
    models::{
        file::WorkspaceAttachment,
        repo::Repo,
        session::Session,
        workspace::Workspace as DbWorkspace,
        workspace_repo::WorkspaceRepo,
    },
};
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

/// Simplified input for workspace creation.
#[derive(Debug, Clone)]
pub struct RepoWorkspaceInput {
    pub repo: Repo,
    pub target_branch: String,
}

impl RepoWorkspaceInput {
    pub fn new(repo: Repo, target_branch: String) -> Self {
        Self {
            repo,
            target_branch,
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Workspace not found")]
    WorkspaceNotFound,
    #[error("No repositories provided")]
    NoRepositories,
}

/// Context needed for workspace deletion cleanup
#[derive(Debug, Clone)]
pub struct WorkspaceDeletionContext {
    pub workspace_id: Uuid,
    pub workspace_dir: Option<PathBuf>,
    pub session_ids: Vec<Uuid>,
}

#[derive(Clone)]
pub struct ManagedWorkspace {
    pub workspace: DbWorkspace,
    db: DBService,
}

impl ManagedWorkspace {
    fn new(db: DBService, workspace: DbWorkspace) -> Self {
        Self { workspace, db }
    }

    pub async fn associate_attachments(&self, attachment_ids: &[Uuid]) -> Result<(), sqlx::Error> {
        if attachment_ids.is_empty() {
            return Ok(());
        }
        WorkspaceAttachment::associate_many_dedup(&self.db.pool, self.workspace.id, attachment_ids)
            .await
    }

    pub async fn prepare_deletion_context(&self) -> Result<WorkspaceDeletionContext, sqlx::Error> {
        let session_ids = Session::find_by_workspace_id(&self.db.pool, self.workspace.id)
            .await?
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();

        Ok(WorkspaceDeletionContext {
            workspace_id: self.workspace.id,
            workspace_dir: self.workspace.container_ref.clone().map(PathBuf::from),
            session_ids,
        })
    }

    pub async fn delete_record(&self) -> Result<u64, sqlx::Error> {
        DbWorkspace::delete(&self.db.pool, self.workspace.id).await
    }
}

#[derive(Clone)]
pub struct WorkspaceManager {
    db: DBService,
}

impl WorkspaceManager {
    pub fn new(db: DBService) -> Self {
        Self { db }
    }

    pub async fn load_managed_workspace(
        &self,
        workspace: DbWorkspace,
    ) -> Result<ManagedWorkspace, sqlx::Error> {
        Ok(ManagedWorkspace::new(self.db.clone(), workspace))
    }

    /// Get the base directory for workspace working directories.
    pub fn get_workspace_base_dir() -> PathBuf {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("vibe-kanban")
            .join("workspaces");
        base
    }

    /// Spawn background cleanup for a deleted workspace.
    pub fn spawn_workspace_deletion_cleanup(context: WorkspaceDeletionContext) {
        tokio::spawn(async move {
            let WorkspaceDeletionContext {
                workspace_id,
                workspace_dir,
                session_ids,
            } = context;

            for session_id in session_ids {
                if let Err(e) = Self::remove_session_process_logs(session_id).await {
                    warn!(
                        "Failed to remove process logs for session {}: {}",
                        session_id, e
                    );
                }
            }

            if let Some(workspace_dir) = workspace_dir {
                if workspace_dir.exists() {
                    info!(
                        "Cleaning up workspace {} directory: {}",
                        workspace_id,
                        workspace_dir.display()
                    );
                    if let Err(e) = tokio::fs::remove_dir_all(&workspace_dir).await {
                        warn!(
                            "Failed to remove workspace dir {}: {}",
                            workspace_dir.display(),
                            e
                        );
                    }
                }
            }
        });
    }

    async fn remove_session_process_logs(session_id: Uuid) -> Result<(), std::io::Error> {
        let dir = utils::execution_logs::process_logs_session_dir(session_id);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn create_workspace(
        workspace_dir: &Path,
        _repos: &[RepoWorkspaceInput],
        _branch_name: &str,
    ) -> Result<PathBuf, WorkspaceError> {
        // Simply ensure the directory exists
        tokio::fs::create_dir_all(workspace_dir).await?;
        Ok(workspace_dir.to_path_buf())
    }

    /// Ensure workspace directory exists (cold restart recovery).
    pub async fn ensure_workspace_exists(
        workspace_dir: &Path,
        _repos: &[RepoWorkspaceInput],
        _branch_name: &str,
    ) -> Result<(), WorkspaceError> {
        if !workspace_dir.exists() {
            tokio::fs::create_dir_all(workspace_dir).await?;
        }
        Ok(())
    }

    /// Clean up workspace directory.
    pub async fn cleanup_workspace(
        workspace_dir: &Path,
        _repos: &[Repo],
    ) -> Result<(), WorkspaceError> {
        if workspace_dir.exists() {
            tokio::fs::remove_dir_all(workspace_dir).await?;
        }
        Ok(())
    }

    /// Clean up orphaned workspace directories that no longer have DB records.
    pub async fn cleanup_orphan_workspaces(&self) {
        let cfg = utils::env_config::load_config();
        let disabled = cfg.features.disable_worktree_cleanup.unwrap_or(false)
            || std::env::var("DISABLE_WORKTREE_CLEANUP").is_ok();
        if disabled {
            info!("Orphan workspace cleanup is disabled");
            return;
        }

        let base_dir = Self::get_workspace_base_dir();
        if !base_dir.exists() {
            return;
        }

        let active_workspace_dirs: std::collections::HashSet<String> =
            match DbWorkspace::find_all_with_status(&self.db.pool, Some(false), None).await {
                Ok(workspaces) => workspaces
                    .iter()
                    .filter_map(|w| w.workspace.container_ref.clone())
                    .collect(),
                Err(e) => {
                    warn!("Failed to query workspaces for orphan cleanup: {}", e);
                    return;
                }
            };

        let mut entries = match tokio::fs::read_dir(&base_dir).await {
            Ok(e) => e,
            Err(_) => return,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                let path_str = path.to_string_lossy().to_string();
                if !active_workspace_dirs.contains(&path_str) {
                    info!("Removing orphaned workspace directory: {}", path.display());
                    let _ = tokio::fs::remove_dir_all(&path).await;
                }
            }
        }
    }
}
