import { ReactNode, useMemo, useCallback, useEffect, useRef } from 'react';
import { useParams } from '@tanstack/react-router';
import { useQueryClient } from '@tanstack/react-query';
import { useWorkspaces } from '@/shared/hooks/useWorkspaces';
import { workspaceSummaryKeys } from '@/shared/hooks/workspaceSummaryKeys';
import { useWorkspaceRecord } from '@/shared/hooks/useWorkspaceRecord';
import { useWorkspaceRepo } from '@/shared/hooks/useWorkspaceRepo';
import { useWorkspaceSessions } from '@/shared/hooks/useWorkspaceSessions';
import { workspacesApi } from '@/shared/lib/api';
import type { DiffStats } from 'shared/types';
import { useAppNavigation } from '@/shared/hooks/useAppNavigation';
import { useCurrentAppDestination } from '@/shared/hooks/useCurrentAppDestination';

import { WorkspaceContext } from '@/shared/hooks/useWorkspaceContext';

interface WorkspaceProviderProps {
  children: ReactNode;
}

export function WorkspaceProvider({ children }: WorkspaceProviderProps) {
  const { workspaceId } = useParams({ strict: false });
  const appNavigation = useAppNavigation();
  const currentDestination = useCurrentAppDestination();
  const queryClient = useQueryClient();

  const isCreateMode = currentDestination?.kind === 'workspaces-create';

  const {
    workspaces: activeWorkspaces,
    archivedWorkspaces,
    isLoading: isLoadingList,
  } = useWorkspaces();

  const { data: workspace, isLoading: isLoadingWorkspace } = useWorkspaceRecord(
    workspaceId,
    { enabled: !!workspaceId && !isCreateMode }
  );

  const {
    sessions,
    selectedSession,
    selectedSessionId,
    selectSession,
    selectLatestSession,
    isLoading: isSessionsLoading,
    isNewSessionMode,
    startNewSession,
  } = useWorkspaceSessions(workspaceId, { enabled: !isCreateMode });

  const { repos, isLoading: isReposLoading } = useWorkspaceRepo(workspaceId, {
    enabled: !isCreateMode,
  });

  const gitHubComments: never[] = [];
  const isGitHubCommentsLoading = false;
  const showGitHubComments = false;
  const setShowGitHubComments = (_v: boolean) => {};
  const getGitHubCommentsForFile = (_f: string) => [] as never[];
  const getGitHubCommentCountForFile = (_f: string) => 0;
  const getFilesWithGitHubComments = () => new Set<string>();
  const getFirstCommentLineForFile = (_f: string) => undefined;

  const diffs: never[] = [];

  const diffPaths = useMemo(
    () =>
      new Set(diffs.map((d: any) => d.newPath || d.oldPath || '').filter(Boolean)),
    [diffs]
  );

  const diffStats: DiffStats = useMemo(
    () => ({
      files_changed: 0,
      lines_added: 0,
      lines_removed: 0,
    }),
    []
  );

  const rafRef = useRef<number | null>(null);
  const batchCountRef = useRef(0);

  const latestDiffDataRef = useRef({
    diffs,
    diffPaths,
    diffStats,
    gitHubComments,
    isGitHubCommentsLoading,
    showGitHubComments,
    setShowGitHubComments,
    getGitHubCommentsForFile,
    getGitHubCommentCountForFile,
    getFilesWithGitHubComments,
    getFirstCommentLineForFile,
  });
  latestDiffDataRef.current = {
    diffs,
    diffPaths,
    diffStats,
    gitHubComments,
    isGitHubCommentsLoading,
    showGitHubComments,
    setShowGitHubComments,
    getGitHubCommentsForFile,
    getGitHubCommentCountForFile,
    getFilesWithGitHubComments,
    getFirstCommentLineForFile,
  };

  useEffect(() => {
    batchCountRef.current++;
    if (rafRef.current === null) {
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        batchCountRef.current = 0;
      });
    }
    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [
    diffs,
    diffPaths,
    diffStats,
    gitHubComments,
    isGitHubCommentsLoading,
    showGitHubComments,
    setShowGitHubComments,
    getGitHubCommentsForFile,
    getGitHubCommentCountForFile,
    getFilesWithGitHubComments,
    getFirstCommentLineForFile,
  ]);

  useEffect(() => {
    return () => {
    };
  }, []);

  const isLoading = isLoadingList || isLoadingWorkspace;

  useEffect(() => {
    if (!workspaceId || isCreateMode) return;

    workspacesApi
      .markSeen(workspaceId)
      .then(() => {
        queryClient.invalidateQueries({ queryKey: workspaceSummaryKeys.all });
      })
      .catch((error) => {
        console.warn('Failed to mark workspace as seen:', error);
      });
  }, [workspaceId, isCreateMode, queryClient]);

  const selectWorkspace = useCallback(
    (id: string) => {
      appNavigation.goToWorkspace(id);
    },
    [appNavigation]
  );

  const navigateToCreate = useMemo(
    () => () => {
      appNavigation.goToWorkspacesCreate();
    },
    [appNavigation]
  );

  const coreValue = useMemo(
    () => ({
      workspaceId,
      workspace,
      activeWorkspaces,
      archivedWorkspaces,
      isWorkspacesListLoading: isLoadingList,
      isLoading,
      isCreateMode,
      selectWorkspace,
      navigateToCreate,
      sessions,
      selectedSession,
      selectedSessionId,
      selectSession,
      selectLatestSession,
      isSessionsLoading,
      isNewSessionMode,
      startNewSession,
      repos,
      isReposLoading,
    }),
    [
      workspaceId,
      workspace,
      activeWorkspaces,
      archivedWorkspaces,
      isLoadingList,
      isLoading,
      isCreateMode,
      selectWorkspace,
      navigateToCreate,
      sessions,
      selectedSession,
      selectedSessionId,
      selectSession,
      selectLatestSession,
      isSessionsLoading,
      isNewSessionMode,
      startNewSession,
      repos,
      isReposLoading,
    ]
  );

  return (
    <WorkspaceContext.Provider value={coreValue}>
      {children}
    </WorkspaceContext.Provider>
  );
}
