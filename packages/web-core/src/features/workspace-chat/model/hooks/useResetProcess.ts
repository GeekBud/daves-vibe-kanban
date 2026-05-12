import { useCallback, useMemo } from 'react';
import { useExecutionProcessesContext } from '@/shared/hooks/useExecutionProcessesContext';
import { isCodingAgent } from '@/shared/constants/processes';
import { useResetProcessMutation } from './useResetProcessMutation';

export interface UseResetProcessResult {
  resetProcess: (executionProcessId: string) => void;
  canResetProcess: (executionProcessId: string) => boolean;
  isResetPending: boolean;
}

/**
 * @param workspaceId - passed explicitly to avoid subscribing to WorkspaceContext
 * @param selectedSessionId - passed explicitly to avoid subscribing to WorkspaceContext
 */
export function useResetProcess(
  _workspaceId: string | undefined,
  selectedSessionId: string | undefined
): UseResetProcessResult {
  const { executionProcessesAll: processes } = useExecutionProcessesContext();

  const resetMutation = useResetProcessMutation(selectedSessionId ?? '', undefined);
  const isResetPending = resetMutation.isPending;

  const hasCodingProcess = useMemo(
    () =>
      processes.some(
        (process) => !process.dropped && isCodingAgent(process.run_reason)
      ),
    [processes]
  );

  const canResetProcess = useCallback(
    (executionProcessId: string) => hasCodingProcess && !!executionProcessId,
    [hasCodingProcess]
  );

  const resetProcess = useCallback(
    (executionProcessId: string) => {
      if (!selectedSessionId) return;
      resetMutation.mutate({
        executionProcessId,
        branchStatus: undefined,
        processes,
      });
    },
    [selectedSessionId, resetMutation, processes]
  );

  return useMemo(
    () => ({
      resetProcess,
      canResetProcess,
      isResetPending,
    }),
    [resetProcess, canResetProcess, isResetPending]
  );
}
