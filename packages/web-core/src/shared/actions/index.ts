import { forwardRef, createElement } from 'react';
import type { Icon, IconProps } from '@phosphor-icons/react';
import type { ExecutorConfig, Merge, Workspace } from 'shared/types';
import type { QueryClient } from '@tanstack/react-query';
import {
  CopyIcon,
  XIcon,
  PushPinIcon,
  ArchiveIcon,
  TrashIcon,
  PlusIcon,
  GearIcon,
  ColumnsIcon,
  RowsIcon,
  TextAlignLeftIcon,
  EyeSlashIcon,
  SidebarSimpleIcon,
  ChatsTeardropIcon,
  GitDiffIcon,
  TerminalIcon,
  SignInIcon,
  SignOutIcon,
  CaretDoubleUpIcon,
  CaretDoubleDownIcon,
  PlayIcon,
  PauseIcon,
  SpinnerIcon,
  GitPullRequestIcon,
  GitForkIcon,
  DesktopIcon,
  PencilSimpleIcon,
  HighlighterIcon,
  ListIcon,
  MegaphoneIcon,
  QuestionIcon,
  ArrowsLeftRightIcon,
  ArrowFatLineUpIcon,
  UsersIcon,
  TreeStructureIcon,
  LinkIcon,
  ArrowBendUpRightIcon,
  ProhibitIcon,
} from '@phosphor-icons/react';
import { useDiffViewStore } from '@/shared/stores/useDiffViewStore';
import {
  useUiPreferencesStore,
  RIGHT_MAIN_PANEL_MODES,
} from '@/shared/stores/useUiPreferencesStore';

import { workspacesApi, relayApi, repoApi } from '@/shared/lib/api';
import { bulkUpdateIssues } from '@/shared/lib/remoteApi';
import { workspaceRecordKeys } from '@/shared/hooks/useWorkspaceRecord';
import { workspaceSummaryKeys } from '@/shared/hooks/workspaceSummaryKeys';
import { ConfirmDialog } from '@vibe/ui/components/ConfirmDialog';
import { DeleteWorkspaceDialog } from '@vibe/ui/components/DeleteWorkspaceDialog';
import { RenameWorkspaceDialog } from '@vibe/ui/components/RenameWorkspaceDialog';
import { ProjectsGuideDialog } from '@vibe/ui/components/ProjectsGuideDialog';
import { getIdeName } from '@/shared/lib/ideName';
import { EditorSelectionDialog } from '@/shared/dialogs/command-bar/EditorSelectionDialog';
import { StartReviewDialog } from '@/shared/dialogs/command-bar/StartReviewDialog';
import posthog from 'posthog-js';
import { WorkspacesGuideDialog } from '@/shared/dialogs/shared/WorkspacesGuideDialog';
import { SettingsDialog } from '@/shared/dialogs/settings/SettingsDialog';
import { CreateWorkspaceFromPrDialog } from '@/shared/dialogs/command-bar/CreateWorkspaceFromPrDialog';
import { buildWorkspaceCreateInitialState } from '@/shared/lib/workspaceCreateState';
import { setCreateModeSeedState } from '@/features/create-mode/model/createModeSeedStore';

// Mirrored sidebar icon for right sidebar toggle
const RightSidebarIcon: Icon = forwardRef<SVGSVGElement, IconProps>(
  (props, ref) =>
    createElement(SidebarSimpleIcon, {
      ref,
      ...props,
      style: { transform: 'scaleX(-1)', ...props.style },
    })
);
RightSidebarIcon.displayName = 'RightSidebarIcon';

import type {
  ActionExecutorContext,
  ActionDefinition,
  GlobalActionDefinition,
  WorkspaceActionDefinition,
  IssueActionDefinition,
  NavbarItem,
} from '@/shared/types/actions';
import { ActionTargetType, NavbarDivider } from '@/shared/types/actions';

async function resolveLinkedIssue(
  workspaceId: string,
  remoteWorkspaces: {
    local_workspace_id: string | null;
    issue_id: string | null;
    project_id: string;
  }[]
): Promise<{ issueId: string; remoteProjectId: string } | undefined> {
  const remoteWs = remoteWorkspaces.find(
    (w) => w.local_workspace_id === workspaceId
  );
  if (remoteWs?.issue_id) {
    return { issueId: remoteWs.issue_id, remoteProjectId: remoteWs.project_id };
  }
  return undefined;
}

async function getWorkspace(
  queryClient: QueryClient,
  workspaceId: string
): Promise<Workspace> {
  const cached = queryClient.getQueryData<Workspace>(
    workspaceRecordKeys.byId(workspaceId)
  );
  if (cached) {
    return cached;
  }
  // Fetch from API if not in cache
  return workspacesApi.get(workspaceId);
}

// Helper to invalidate workspace-related queries
function invalidateWorkspaceQueries(
  queryClient: QueryClient,
  workspaceId: string
) {
  queryClient.invalidateQueries({
    queryKey: workspaceRecordKeys.byId(workspaceId),
  });
  queryClient.invalidateQueries({ queryKey: workspaceSummaryKeys.all });
}

// Helper to find the next workspace to navigate to when removing current workspace
function getNextWorkspaceId(
  activeWorkspaces: { id: string; isRunning?: boolean }[],
  removingWorkspaceId: string
): string | null {
  const currentIndex = activeWorkspaces.findIndex(
    (ws) => ws.id === removingWorkspaceId
  );
  if (currentIndex >= 0 && activeWorkspaces.length > 1) {
    const nextWorkspace =
      activeWorkspaces[currentIndex + 1] || activeWorkspaces[currentIndex - 1];
    return nextWorkspace?.id ?? null;
  }
  return null;
}

// Helper to navigate to create-issue form for a sub-issue, carrying over parent assignees
function navigateToCreateSubIssue(
  ctx: ActionExecutorContext,
  parentIssueId: string
) {
  const assigneeIds = ctx.projectMutations
    ?.getAssigneesForIssue(parentIssueId)
    .map((a) => a.user_id);
  ctx.navigateToCreateIssue({
    statusId: ctx.defaultCreateStatusId,
    parentIssueId,
    assigneeIds: assigneeIds?.length ? assigneeIds : undefined,
  });
}

// All application actions
export const Actions = {
  // === Workspace Actions ===
  DuplicateWorkspace: {
    id: 'duplicate-workspace',
    label: 'Duplicate',
    icon: CopyIcon,
    shortcut: 'W D',
    requiresTarget: ActionTargetType.WORKSPACE,
    execute: async (ctx, workspaceId) => {
      try {
        const [firstMessage, repos, workspaceWithSession] = await Promise.all([
          workspacesApi.getFirstUserMessage(workspaceId),
          workspacesApi.getRepos(workspaceId),
          workspacesApi.getWithSession(workspaceId),
        ]);

        const linkedIssue = await resolveLinkedIssue(
          workspaceId,
          ctx.remoteWorkspaces
        );

        const executorConfig = workspaceWithSession.session?.executor
          ? {
              executor: workspaceWithSession.session
                .executor as ExecutorConfig['executor'],
            }
          : null;

        const createState = buildWorkspaceCreateInitialState({
          prompt: firstMessage,
          defaults: {
            preferredRepos: repos.map((r) => ({
              repo_id: r.id,
              target_branch: r.target_branch,
            })),
          },
          linkedIssue,
          executorConfig,
        });
        setCreateModeSeedState(createState);
        ctx.appNavigation.goToWorkspacesCreate();
      } catch {
        ctx.appNavigation.goToWorkspacesCreate();
      }
    },
  },

  RenameWorkspace: {
    id: 'rename-workspace',
    label: 'Rename',
    icon: PencilSimpleIcon,
    shortcut: 'W R',
    requiresTarget: ActionTargetType.WORKSPACE,
    execute: async (ctx, workspaceId) => {
      const workspace = await getWorkspace(ctx.queryClient, workspaceId);
      await RenameWorkspaceDialog.show({
        currentName: workspace.name || workspace.branch,
        onRename: async (newName) => {
          await workspacesApi.update(workspaceId, { name: newName });
          invalidateWorkspaceQueries(ctx.queryClient, workspaceId);
        },
      });
    },
  },

  PinWorkspace: {
    id: 'pin-workspace',
    label: (workspace?: Workspace) => (workspace?.pinned ? 'Unpin' : 'Pin'),
    icon: PushPinIcon,
    shortcut: 'W P',
    requiresTarget: ActionTargetType.WORKSPACE,
    execute: async (ctx, workspaceId) => {
      const workspace = await getWorkspace(ctx.queryClient, workspaceId);
      await workspacesApi.update(workspaceId, {
        pinned: !workspace.pinned,
      });
      invalidateWorkspaceQueries(ctx.queryClient, workspaceId);
    },
  },

  ArchiveWorkspace: {
    id: 'archive-workspace',
    label: (workspace?: Workspace) =>
      workspace?.archived ? 'Unarchive' : 'Archive',
    icon: ArchiveIcon,
    shortcut: 'W A',
    requiresTarget: ActionTargetType.WORKSPACE,
    isVisible: (ctx) => ctx.hasWorkspace && ctx.layoutMode === 'workspaces',
    isActive: (ctx) => ctx.workspaceArchived,
    execute: async (ctx, workspaceId) => {
      const workspace = await getWorkspace(ctx.queryClient, workspaceId);
      const wasArchived = workspace.archived;

      // Calculate next workspace before archiving
      const nextWorkspaceId = !wasArchived
        ? getNextWorkspaceId(ctx.activeWorkspaces, workspaceId)
        : null;

      // Perform the archive/unarchive
      await workspacesApi.update(workspaceId, { archived: !wasArchived });
      invalidateWorkspaceQueries(ctx.queryClient, workspaceId);

      // Select next workspace after successful archive
      if (!wasArchived && nextWorkspaceId) {
        ctx.selectWorkspace(nextWorkspaceId);
      }
    },
  },

  DeleteWorkspace: {
    id: 'delete-workspace',
    label: 'Delete',
    icon: TrashIcon,
    shortcut: 'W X',
    variant: 'destructive',
    requiresTarget: ActionTargetType.WORKSPACE,
    execute: async (ctx, workspaceId) => {
      const workspace = await getWorkspace(ctx.queryClient, workspaceId);

      // Check if workspace is linked to a remote issue
      const remoteWs = ctx.remoteWorkspaces.find(
        (w) => w.local_workspace_id === workspaceId
      );
      const linkedIssueSimpleId = remoteWs?.issue_id
        ? ctx.projectMutations?.getIssue(remoteWs.issue_id)?.simple_id
        : undefined;
      const branchStatus = await workspacesApi.getBranchStatus(workspaceId);
      const hasOpenPR = branchStatus.some((repoStatus) =>
        repoStatus.merges?.some(
          (m: Merge) => m.type === 'pr' && m.pr_info.status === 'open'
        )
      );

      const result = await DeleteWorkspaceDialog.show({
        branchName: workspace.branch,
        hasOpenPR,
        isLinkedToIssue: Boolean(remoteWs?.issue_id),
        linkedIssueSimpleId,
      });
      if (result.action === 'confirmed') {
        // Calculate next workspace before deleting (only if deleting current)
        const isCurrentWorkspace = ctx.currentWorkspaceId === workspaceId;
        const nextWorkspaceId = isCurrentWorkspace
          ? getNextWorkspaceId(ctx.activeWorkspaces, workspaceId)
          : null;

        await workspacesApi.delete(workspaceId, result.deleteBranches);

        // Unlink from remote issue after successful deletion
        if (result.unlinkFromIssue) {
          await workspacesApi.unlinkFromIssue(workspaceId);
        }
        ctx.queryClient.invalidateQueries({
          queryKey: workspaceSummaryKeys.all,
        });

        // Navigate away if we deleted the current workspace
        if (isCurrentWorkspace) {
          if (nextWorkspaceId) {
            ctx.selectWorkspace(nextWorkspaceId);
          } else {
            ctx.appNavigation.goToWorkspacesCreate();
          }
        }
      }
    },
  },

  StartReview: {
    id: 'start-review',
    label: 'Start Review',
    icon: HighlighterIcon,
    requiresTarget: ActionTargetType.WORKSPACE,
    isVisible: (ctx) => ctx.hasWorkspace,
    getTooltip: () => 'Review changes with agent',
    execute: async (_ctx, workspaceId) => {
      await StartReviewDialog.show({
        workspaceId,
      });
    },
  },

  SpinOffWorkspace: {
    id: 'spin-off-workspace',
    label: 'Spin off workspace',
    icon: GitForkIcon,
    requiresTarget: ActionTargetType.WORKSPACE,
    isVisible: (ctx) => ctx.hasWorkspace,
    execute: async (ctx, workspaceId) => {
      try {
        const [workspace, repos] = await Promise.all([
          getWorkspace(ctx.queryClient, workspaceId),
          workspacesApi.getRepos(workspaceId),
        ]);
        const linkedIssue = await resolveLinkedIssue(
          workspaceId,
          ctx.remoteWorkspaces
        );

        const createState = buildWorkspaceCreateInitialState({
          prompt: null,
          defaults: {
            preferredRepos: repos.map((r) => ({
              repo_id: r.id,
              target_branch: workspace.branch,
            })),
          },
          linkedIssue,
        });
        setCreateModeSeedState(createState);
        ctx.appNavigation.goToWorkspacesCreate();
      } catch {
        ctx.appNavigation.goToWorkspacesCreate();
      }
    },
  },

  // === Global/Navigation Actions ===
  NewWorkspace: {
    id: 'new-workspace',
    label: 'New Workspace',
    icon: PlusIcon,
    shortcut: 'G N',
    requiresTarget: ActionTargetType.NONE,
    execute: (ctx) => {
      ctx.appNavigation.goToWorkspacesCreate();
    },
  },

  CreateWorkspaceFromPR: {
    id: 'create-workspace-from-pr',
    label: 'Create Workspace from PR',
    icon: GitPullRequestIcon,
    keywords: ['pull request'],
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'workspaces',
    execute: async () => {
      await CreateWorkspaceFromPrDialog.show({});
    },
  } satisfies GlobalActionDefinition,

  Settings: {
    id: 'settings',
    label: 'Settings',
    icon: GearIcon,
    shortcut: 'G S',
    requiresTarget: ActionTargetType.NONE,
    execute: async () => {
      await SettingsDialog.show();
    },
  },

  ProjectSettings: {
    id: 'project-settings',
    label: 'Project Settings',
    icon: GearIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'kanban',
    execute: async (ctx) => {
      await SettingsDialog.show({
        initialSection: 'remote-projects',
        initialState: {
          organizationId: ctx.kanbanOrgId,
          projectId: ctx.kanbanProjectId,
        },
      });
    },
  } satisfies GlobalActionDefinition,

  SignIn: {
    id: 'sign-in',
    label: 'Sign In',
    icon: SignInIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => !ctx.isSignedIn,
    execute: async () => {
      const { OAuthDialog } = await import(
        '@/shared/dialogs/global/OAuthDialog'
      );
      await OAuthDialog.show({});
    },
  } satisfies GlobalActionDefinition,

  SignOut: {
    id: 'sign-out',
    label: 'Sign Out',
    icon: SignOutIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.isSignedIn,
    execute: async (ctx) => {
      const { oauthApi } = await import('@/shared/lib/api');
      const { useOrganizationStore } = await import(
        '@/shared/stores/useOrganizationStore'
      );
      const { organizationKeys } = await import(
        '@/shared/hooks/organizationKeys'
      );

      await oauthApi.logout();
      useOrganizationStore.getState().clearSelectedOrgId();
      ctx.queryClient.removeQueries({ queryKey: organizationKeys.all });
      // Invalidate user-system query to update loginStatus/useAuth state
      await ctx.queryClient.invalidateQueries({ queryKey: ['user-system'] });
      ctx.appNavigation.goToWorkspaces();
    },
  } satisfies GlobalActionDefinition,

  Feedback: {
    id: 'feedback',
    label: 'Give Feedback',
    icon: MegaphoneIcon,
    requiresTarget: ActionTargetType.NONE,
    execute: () => {
      posthog.displaySurvey('019bb6e8-3d36-0000-1806-7330cd3c727e');
    },
  },

  WorkspacesGuide: {
    id: 'workspaces-guide',
    label: 'Workspaces Guide',
    icon: QuestionIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'workspaces',
    execute: async () => {
      await WorkspacesGuideDialog.show();
    },
  },

  ProjectsGuide: {
    id: 'projects-guide',
    label: 'Projects Guide',
    icon: QuestionIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'kanban',
    execute: async () => {
      await ProjectsGuideDialog.show();
    },
  } satisfies GlobalActionDefinition,

  OpenCommandBar: {
    id: 'open-command-bar',
    label: 'Open Command Bar',
    icon: ListIcon,
    shortcut: '{mod} K',
    requiresTarget: ActionTargetType.NONE,
    execute: async () => {
      // Dynamic import to avoid circular dependency (pages.ts imports Actions)
      const { CommandBarDialog } = await import(
        '@/shared/dialogs/command-bar/CommandBarDialog'
      );
      CommandBarDialog.show();
    },
  },

  // === Diff View Actions ===
  ToggleDiffViewMode: {
    id: 'toggle-diff-view-mode',
    label: () =>
      useDiffViewStore.getState().mode === 'unified'
        ? 'Switch to Side-by-Side View'
        : 'Switch to Inline View',
    icon: ColumnsIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.CHANGES &&
      ctx.layoutMode === 'workspaces',
    isActive: (ctx) => ctx.diffViewMode === 'split',
    getIcon: (ctx) => (ctx.diffViewMode === 'split' ? ColumnsIcon : RowsIcon),
    getTooltip: (ctx) =>
      ctx.diffViewMode === 'split' ? 'Inline view' : 'Side-by-side view',
    execute: () => {
      useDiffViewStore.getState().toggle();
    },
  },

  ToggleIgnoreWhitespace: {
    id: 'toggle-ignore-whitespace',
    label: () =>
      useDiffViewStore.getState().ignoreWhitespace
        ? 'Show Whitespace Changes'
        : 'Ignore Whitespace Changes',
    icon: EyeSlashIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.CHANGES &&
      ctx.layoutMode === 'workspaces',
    execute: () => {
      const store = useDiffViewStore.getState();
      store.setIgnoreWhitespace(!store.ignoreWhitespace);
    },
  },

  ToggleWrapLines: {
    id: 'toggle-wrap-lines',
    label: () =>
      useDiffViewStore.getState().wrapText
        ? 'Disable Line Wrapping'
        : 'Enable Line Wrapping',
    icon: TextAlignLeftIcon,
    shortcut: 'T W',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.CHANGES &&
      ctx.layoutMode === 'workspaces',
    execute: () => {
      const store = useDiffViewStore.getState();
      store.setWrapText(!store.wrapText);
    },
  },

  // === Layout Panel Actions ===
  ToggleLeftSidebar: {
    id: 'toggle-left-sidebar',
    label: () =>
      useUiPreferencesStore.getState().isLeftSidebarVisible
        ? 'Hide Left Sidebar'
        : 'Show Left Sidebar',
    icon: SidebarSimpleIcon,
    shortcut: 'V S',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'workspaces',
    isActive: (ctx) => ctx.isLeftSidebarVisible,
    execute: () => {
      useUiPreferencesStore.getState().toggleLeftSidebar();
    },
  },

  ToggleLeftMainPanel: {
    id: 'toggle-left-main-panel',
    label: 'Toggle Chat Panel',
    icon: ChatsTeardropIcon,
    shortcut: 'V H',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'workspaces',
    isActive: (ctx) => ctx.isLeftMainPanelVisible,
    isEnabled: (ctx) =>
      !(ctx.isLeftMainPanelVisible && ctx.rightMainPanelMode === null),
    getLabel: (ctx) =>
      ctx.isLeftMainPanelVisible ? 'Hide Chat Panel' : 'Show Chat Panel',
    execute: (ctx) => {
      useUiPreferencesStore
        .getState()
        .toggleLeftMainPanel(ctx.currentWorkspaceId ?? undefined);
    },
  },

  ToggleRightSidebar: {
    id: 'toggle-right-sidebar',
    label: () =>
      useUiPreferencesStore.getState().isRightSidebarVisible
        ? 'Hide Right Sidebar'
        : 'Show Right Sidebar',
    icon: RightSidebarIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'workspaces',
    isActive: (ctx) => ctx.isRightSidebarVisible,
    execute: () => {
      useUiPreferencesStore.getState().toggleRightSidebar();
    },
  },

  ToggleChangesMode: {
    id: 'toggle-changes-mode',
    label: 'Toggle Changes Panel',
    icon: GitDiffIcon,
    shortcut: 'V C',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => !ctx.isCreateMode && ctx.layoutMode === 'workspaces',
    isActive: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.CHANGES,
    isEnabled: (ctx) => !ctx.isCreateMode,
    getLabel: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.CHANGES
        ? 'Hide Changes Panel'
        : 'Show Changes Panel',
    execute: (ctx) => {
      useUiPreferencesStore
        .getState()
        .toggleRightMainPanelMode(
          RIGHT_MAIN_PANEL_MODES.CHANGES,
          ctx.currentWorkspaceId ?? undefined
        );
    },
  },

  ToggleLogsMode: {
    id: 'toggle-logs-mode',
    label: 'Toggle Logs Panel',
    icon: TerminalIcon,
    shortcut: 'V L',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => !ctx.isCreateMode && ctx.layoutMode === 'workspaces',
    isActive: (ctx) => ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.LOGS,
    isEnabled: (ctx) => !ctx.isCreateMode,
    getLabel: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.LOGS
        ? 'Hide Logs Panel'
        : 'Show Logs Panel',
    execute: (ctx) => {
      useUiPreferencesStore
        .getState()
        .toggleRightMainPanelMode(
          RIGHT_MAIN_PANEL_MODES.LOGS,
          ctx.currentWorkspaceId ?? undefined
        );
    },
  },

  TogglePreviewMode: {
    id: 'toggle-preview-mode',
    label: 'Toggle Preview Panel',
    icon: DesktopIcon,
    shortcut: 'V P',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => !ctx.isCreateMode && ctx.layoutMode === 'workspaces',
    isActive: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.PREVIEW,
    isEnabled: (ctx) => !ctx.isCreateMode,
    getLabel: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.PREVIEW
        ? 'Hide Preview Panel'
        : 'Show Preview Panel',
    execute: (ctx) => {
      useUiPreferencesStore
        .getState()
        .toggleRightMainPanelMode(
          RIGHT_MAIN_PANEL_MODES.PREVIEW,
          ctx.currentWorkspaceId ?? undefined
        );
    },
  },

  // === Diff Actions for Navbar ===
  ToggleAllDiffs: {
    id: 'toggle-all-diffs',
    label: () => {
      const { expanded } = useUiPreferencesStore.getState();
      const keys = Array.from(new Set<string>()).map((p) => `diff:${p}`);
      const isAllExpanded =
        keys.length > 0 && keys.every((k) => expanded[k] !== false);
      return isAllExpanded ? 'Collapse All Diffs' : 'Expand All Diffs';
    },
    icon: CaretDoubleUpIcon,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.CHANGES &&
      ctx.layoutMode === 'workspaces',
    getIcon: (ctx) =>
      ctx.isAllDiffsExpanded ? CaretDoubleUpIcon : CaretDoubleDownIcon,
    getTooltip: (ctx) =>
      ctx.isAllDiffsExpanded ? 'Collapse all diffs' : 'Expand all diffs',
    execute: () => {
      const { expanded, setExpandedAll } = useUiPreferencesStore.getState();
      const keys = Array.from(new Set<string>()).map((p) => `diff:${p}`);
      const isAllExpanded =
        keys.length > 0 && keys.every((k) => expanded[k] !== false);
      setExpandedAll(keys, !isAllExpanded);
    },
  },

  // === ContextBar Actions ===
  OpenInIDE: {
    id: 'open-in-ide',
    label: 'Open in IDE',
    icon: 'ide-icon' as const,
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.hasWorkspace,
    getTooltip: (ctx) => `Open in ${getIdeName(ctx.editorType)}`,
    execute: async (ctx) => {
      if (!ctx.currentWorkspaceId) return;
      try {
        const response =
          ctx.appRuntime === 'local' && ctx.currentHostId
            ? await relayApi.openRemoteWorkspaceInEditor({
                host_id: ctx.currentHostId,
                workspace_id: ctx.currentWorkspaceId,
                editor_type: null,
                file_path: null,
              })
            : await workspacesApi.openEditor(ctx.currentWorkspaceId, {
                editor_type: null,
                file_path: null,
              });
        if (response.url) {
          window.open(response.url, '_blank');
        }
      } catch {
        // Show editor selection dialog on failure
        EditorSelectionDialog.show({
          selectedAttemptId: ctx.currentWorkspaceId,
        });
      }
    },
  },

  CopyWorkspacePath: {
    id: 'copy-workspace-path',
    label: 'Copy Workspace Path',
    icon: 'copy-icon' as const,
    shortcut: 'Y P',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.hasWorkspace,
    execute: async (ctx) => {
      if (!ctx.containerRef) return;
      await navigator.clipboard.writeText(ctx.containerRef);
    },
  },

  CopyRawLogs: {
    id: 'copy-raw-logs',
    label: 'Copy Raw Logs',
    icon: CopyIcon,
    shortcut: 'Y L',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) =>
      ctx.rightMainPanelMode === RIGHT_MAIN_PANEL_MODES.LOGS &&
      ctx.logsPanelContent?.type !== 'terminal',
    execute: async (ctx) => {
      if (!ctx.currentLogs || ctx.currentLogs.length === 0) return;
      const rawText = ctx.currentLogs.map((log) => log.content).join('\n');
      await navigator.clipboard.writeText(rawText);
    },
  },

  ToggleDevServer: {
    id: 'toggle-dev-server',
    label: 'Dev Server',
    icon: PlayIcon,
    shortcut: 'T D',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.hasWorkspace,
    isEnabled: (ctx) =>
      ctx.devServerState !== 'starting' && ctx.devServerState !== 'stopping',
    getIcon: (ctx) => {
      if (
        ctx.devServerState === 'starting' ||
        ctx.devServerState === 'stopping'
      ) {
        return SpinnerIcon;
      }
      if (ctx.devServerState === 'running') {
        return PauseIcon;
      }
      return PlayIcon;
    },
    getTooltip: (ctx) => {
      switch (ctx.devServerState) {
        case 'starting':
          return 'Starting dev server...';
        case 'stopping':
          return 'Stopping dev server...';
        case 'running':
          return 'Stop dev server';
        default:
          return 'Start dev server';
      }
    },
    getLabel: (ctx) =>
      ctx.devServerState === 'running' ? 'Stop Dev Server' : 'Start Dev Server',
    execute: (ctx) => {
      if (ctx.runningDevServers.length > 0) {
        ctx.stopDevServer();
      } else {
        ctx.startDevServer();
        // Auto-open preview mode when starting dev server
        useUiPreferencesStore
          .getState()
          .setRightMainPanelMode(
            RIGHT_MAIN_PANEL_MODES.PREVIEW,
            ctx.currentWorkspaceId ?? undefined
          );
      }
    },
  },

  // === Repo-specific Actions (for command bar when selecting a repo) ===
  RepoCopyPath: {
    id: 'repo-copy-path',
    label: 'Copy Repo Path',
    icon: CopyIcon,
    requiresTarget: ActionTargetType.GIT,
    isVisible: (ctx) => ctx.hasWorkspace && ctx.hasGitRepos,
    execute: async (_ctx, _workspaceId, repoId) => {
      try {
        const repo = await repoApi.getById(repoId);
        if (repo?.path) {
          await navigator.clipboard.writeText(repo.path);
        }
      } catch (err) {
        console.error('Failed to copy repo path:', err);
        throw new Error('Failed to copy repository path');
      }
    },
  },

  RepoOpenInIDE: {
    id: 'repo-open-in-ide',
    label: 'Open Repo in IDE',
    icon: DesktopIcon,
    requiresTarget: ActionTargetType.GIT,
    isVisible: (ctx) => ctx.hasWorkspace && ctx.hasGitRepos,
    execute: async (_ctx, _workspaceId, repoId) => {
      try {
        const response = await repoApi.openEditor(repoId, {
          editor_type: null,
          file_path: null,
        });
        if (response.url) {
          window.open(response.url, '_blank');
        }
      } catch (err) {
        console.error('Failed to open repo in editor:', err);
        throw new Error('Failed to open repository in IDE');
      }
    },
  },

  RepoSettings: {
    id: 'repo-settings',
    label: 'Repository Settings',
    icon: GearIcon,
    requiresTarget: ActionTargetType.GIT,
    isVisible: (ctx) => ctx.hasWorkspace && ctx.hasGitRepos,
    execute: async (_ctx, _workspaceId, repoId) => {
      await SettingsDialog.show({
        initialSection: 'repos',
        initialState: {
          repoId,
        },
      });
    },
  },

  // === Script Actions ===
  RunSetupScript: {
    id: 'run-setup-script',
    label: 'Run Setup Script',
    icon: TerminalIcon,
    shortcut: 'R S',
    requiresTarget: ActionTargetType.WORKSPACE,
    isVisible: (ctx) => ctx.hasWorkspace,
    isEnabled: (ctx) => !ctx.isAttemptRunning,
    execute: async (_ctx, workspaceId) => {
      const result = await workspacesApi.runSetupScript(workspaceId);
      if (!result.success) {
        if (result.error?.type === 'no_script_configured') {
          throw new Error('No setup script configured for this project');
        }
        if (result.error?.type === 'process_already_running') {
          throw new Error('Cannot run script while another process is running');
        }
        throw new Error('Failed to run setup script');
      }
    },
  },

  RunCleanupScript: {
    id: 'run-cleanup-script',
    label: 'Run Cleanup Script',
    icon: TerminalIcon,
    shortcut: 'R C',
    requiresTarget: ActionTargetType.WORKSPACE,
    isVisible: (ctx) => ctx.hasWorkspace,
    isEnabled: (ctx) => !ctx.isAttemptRunning,
    execute: async (_ctx, workspaceId) => {
      const result = await workspacesApi.runCleanupScript(workspaceId);
      if (!result.success) {
        if (result.error?.type === 'no_script_configured') {
          throw new Error('No cleanup script configured for this project');
        }
        if (result.error?.type === 'process_already_running') {
          throw new Error('Cannot run script while another process is running');
        }
        throw new Error('Failed to run cleanup script');
      }
    },
  },

  RunArchiveScript: {
    id: 'run-archive-script',
    label: 'Run Archive Script',
    icon: TerminalIcon,
    shortcut: 'R A',
    requiresTarget: ActionTargetType.WORKSPACE,
    isVisible: (ctx) => ctx.hasWorkspace,
    isEnabled: (ctx) => !ctx.isAttemptRunning,
    execute: async (_ctx, workspaceId) => {
      const result = await workspacesApi.runArchiveScript(workspaceId);
      if (!result.success) {
        if (result.error?.type === 'no_script_configured') {
          throw new Error('No archive script configured for this project');
        }
        if (result.error?.type === 'process_already_running') {
          throw new Error('Cannot run script while another process is running');
        }
        throw new Error('Failed to run archive script');
      }
    },
  } satisfies WorkspaceActionDefinition,

  // === Issue Actions ===
  CreateIssue: {
    id: 'create-issue',
    label: 'Create Issue',
    icon: PlusIcon,
    shortcut: 'I C',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'kanban' && !ctx.isCreatingIssue,
    execute: (ctx) => {
      ctx.navigateToCreateIssue({ statusId: ctx.defaultCreateStatusId });
    },
  } satisfies GlobalActionDefinition,

  ChangeIssueStatus: {
    id: 'change-issue-status',
    label: 'Change Status',
    icon: ArrowsLeftRightIcon,
    shortcut: 'I S',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      await ctx.openStatusSelection(projectId, issueIds);
    },
  } satisfies IssueActionDefinition,

  ChangeNewIssueStatus: {
    id: 'change-new-issue-status',
    label: 'Change Status',
    icon: ArrowsLeftRightIcon,
    shortcut: 'I S',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'kanban' && ctx.isCreatingIssue,
    execute: async (ctx) => {
      if (!ctx.kanbanProjectId) return;
      const { ProjectSelectionDialog } = await import(
        '@/shared/dialogs/command-bar/selections/ProjectSelectionDialog'
      );
      await ProjectSelectionDialog.show({
        projectId: ctx.kanbanProjectId,
        selection: { type: 'status', issueIds: [], isCreateMode: true },
      });
    },
  } satisfies GlobalActionDefinition,

  ChangePriority: {
    id: 'change-issue-priority',
    label: 'Change Priority',
    icon: ArrowFatLineUpIcon,
    shortcut: 'I P',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      await ctx.openPrioritySelection(projectId, issueIds);
    },
  } satisfies IssueActionDefinition,

  ChangeNewIssuePriority: {
    id: 'change-new-issue-priority',
    label: 'Change Priority',
    icon: ArrowFatLineUpIcon,
    shortcut: 'I P',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'kanban' && ctx.isCreatingIssue,
    execute: async (ctx) => {
      if (!ctx.kanbanProjectId) return;
      const { ProjectSelectionDialog } = await import(
        '@/shared/dialogs/command-bar/selections/ProjectSelectionDialog'
      );
      await ProjectSelectionDialog.show({
        projectId: ctx.kanbanProjectId,
        selection: { type: 'priority', issueIds: [], isCreateMode: true },
      });
    },
  } satisfies GlobalActionDefinition,

  ChangeAssignees: {
    id: 'change-assignees',
    label: 'Change Assignees',
    icon: UsersIcon,
    shortcut: 'I A',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      await ctx.openAssigneeSelection(projectId, issueIds, false);
    },
  } satisfies IssueActionDefinition,

  ChangeNewIssueAssignees: {
    id: 'change-new-issue-assignees',
    label: 'Change Assignees',
    icon: UsersIcon,
    shortcut: 'I A',
    requiresTarget: ActionTargetType.NONE,
    isVisible: (ctx) => ctx.layoutMode === 'kanban' && ctx.isCreatingIssue,
    execute: async (ctx) => {
      // Opens assignee selection for the issue being created
      // ProjectId will be resolved from route params inside the dialog
      await ctx.openAssigneeSelection('', [], true);
    },
  } satisfies GlobalActionDefinition,

  MakeSubIssueOf: {
    id: 'make-sub-issue-of',
    label: 'Make Sub-issue of',
    icon: TreeStructureIcon,
    shortcut: 'I M',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      if (issueIds.length === 1) {
        await ctx.openSubIssueSelection(projectId, issueIds[0], 'setParent');
      }
    },
  } satisfies IssueActionDefinition,

  AddSubIssue: {
    id: 'add-sub-issue',
    label: 'Add Sub-issue',
    icon: PlusIcon,
    shortcut: 'I B',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      if (issueIds.length !== 1) return;
      const parentIssueId = issueIds[0];
      const result = await ctx.openSubIssueSelection(
        projectId,
        parentIssueId,
        'addChild'
      );
      if (result?.type === 'createNew') {
        navigateToCreateSubIssue(ctx, parentIssueId);
      }
    },
  } satisfies IssueActionDefinition,

  CreateSubIssue: {
    id: 'create-sub-issue',
    label: 'Create Sub-issue',
    icon: PlusIcon,
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, _projectId, issueIds) => {
      if (issueIds.length !== 1) return;
      navigateToCreateSubIssue(ctx, issueIds[0]);
    },
  } satisfies IssueActionDefinition,

  RemoveParentIssue: {
    id: 'remove-parent-issue',
    label: 'Remove Parent',
    icon: XIcon,
    shortcut: 'I U',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' &&
      ctx.hasSelectedKanbanIssue &&
      ctx.hasSelectedKanbanIssueParent,
    execute: async (_ctx, _projectId, issueIds) => {
      await bulkUpdateIssues(
        issueIds.map((issueId) => ({
          id: issueId,
          changes: {
            parent_issue_id: null,
            parent_issue_sort_order: null,
          },
        }))
      );
    },
  } satisfies IssueActionDefinition,

  LinkWorkspace: {
    id: 'link-workspace',
    label: 'Link Workspace',
    icon: LinkIcon,
    shortcut: 'I W',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      if (issueIds.length === 1) {
        await ctx.openWorkspaceSelection(projectId, issueIds[0]);
      }
    },
  } satisfies IssueActionDefinition,

  DeleteIssue: {
    id: 'delete-issue',
    label: 'Delete Issue',
    icon: TrashIcon,
    shortcut: 'I X',
    variant: 'destructive',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, _projectId, issueIds) => {
      const count = issueIds.length;
      const result = await ConfirmDialog.show({
        title: count === 1 ? 'Delete Issue' : `Delete ${count} Issues`,
        message:
          count === 1
            ? 'Are you sure you want to delete this issue? This action cannot be undone.'
            : `Are you sure you want to delete these ${count} issues? This action cannot be undone.`,
        confirmText: 'Delete',
        cancelText: 'Cancel',
        variant: 'destructive',
      });
      if (result === 'confirmed' && ctx.projectMutations?.removeIssue) {
        for (const issueId of issueIds) {
          ctx.projectMutations.removeIssue(issueId);
        }
      }
    },
  } satisfies IssueActionDefinition,

  DuplicateIssue: {
    id: 'duplicate-issue',
    label: 'Duplicate Issue',
    icon: CopyIcon,
    shortcut: 'I D',
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, _projectId, issueIds) => {
      if (issueIds.length !== 1) {
        throw new Error('Can only duplicate one issue at a time');
      }
      ctx.projectMutations?.duplicateIssue(issueIds[0]);
    },
  } satisfies IssueActionDefinition,

  MarkBlocking: {
    id: 'mark-blocking',
    label: 'Mark Blocking',
    icon: ArrowBendUpRightIcon,
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      if (issueIds.length === 1) {
        await ctx.openRelationshipSelection(
          projectId,
          issueIds[0],
          'blocking',
          'forward'
        );
      }
    },
  } satisfies IssueActionDefinition,

  MarkBlockedBy: {
    id: 'mark-blocked-by',
    label: 'Mark Blocked By',
    icon: ProhibitIcon,
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      if (issueIds.length === 1) {
        await ctx.openRelationshipSelection(
          projectId,
          issueIds[0],
          'blocking',
          'reverse'
        );
      }
    },
  } satisfies IssueActionDefinition,

  MarkRelated: {
    id: 'mark-related',
    label: 'Mark Related',
    icon: ArrowsLeftRightIcon,
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      if (issueIds.length === 1) {
        await ctx.openRelationshipSelection(
          projectId,
          issueIds[0],
          'related',
          'forward'
        );
      }
    },
  } satisfies IssueActionDefinition,

  MarkDuplicateOf: {
    id: 'mark-duplicate-of',
    label: 'Mark Duplicate Of',
    icon: CopyIcon,
    requiresTarget: ActionTargetType.ISSUE,
    isVisible: (ctx) =>
      ctx.layoutMode === 'kanban' && ctx.hasSelectedKanbanIssue,
    execute: async (ctx, projectId, issueIds) => {
      if (issueIds.length === 1) {
        await ctx.openRelationshipSelection(
          projectId,
          issueIds[0],
          'has_duplicate',
          'forward'
        );
      }
    },
  } satisfies IssueActionDefinition,
} as const satisfies Record<string, ActionDefinition>;

// Navbar action groups define which actions appear in each section
export const NavbarActionGroups = {
  left: [Actions.ArchiveWorkspace] as NavbarItem[],
  right: [
    Actions.ToggleDiffViewMode,
    Actions.ToggleAllDiffs,
    NavbarDivider,
    Actions.ToggleLeftSidebar,
    Actions.ToggleLeftMainPanel,
    Actions.ToggleChangesMode,
    Actions.ToggleLogsMode,
    Actions.TogglePreviewMode,
    Actions.ToggleRightSidebar,
    NavbarDivider,
    Actions.OpenCommandBar,
    Actions.Feedback,
    Actions.WorkspacesGuide,
    Actions.ProjectsGuide,
    Actions.Settings,
  ] as NavbarItem[],
};

// ContextBar action groups define which actions appear in each section
export const ContextBarActionGroups = {
  primary: [Actions.OpenInIDE, Actions.CopyWorkspacePath] as ActionDefinition[],
  secondary: [
    Actions.ToggleDevServer,
    Actions.TogglePreviewMode,
    Actions.ToggleChangesMode,
  ] as ActionDefinition[],
};
