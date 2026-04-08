import {
  ExecutionProcess,
  ExecutionProcessStatus,
  PatchType,
} from 'shared/types';
import { useExecutionProcessesContext } from '@/shared/hooks/useExecutionProcessesContext';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { streamJsonPatchEntries } from '@/shared/lib/streamJsonPatchEntries';
import type {
  AddEntryType,
  ConversationTimelineSource,
  ExecutionProcessStateStore,
  PatchTypeWithKey,
  UseConversationHistoryParams,
} from '@/shared/hooks/useConversationHistory/types';

// Result type for the new UI's conversation history hook
export interface UseConversationHistoryResult {
  /** Whether the conversation only has a single coding agent turn (no follow-ups) */
  isFirstTurn: boolean;
  /** Whether background batches are still loading older history entries */
  isLoadingHistory: boolean;
}
import {
  MIN_INITIAL_ENTRIES,
  REMAINING_BATCH_SIZE,
} from '@/shared/hooks/useConversationHistory/constants';

export const useConversationHistory = ({
  onTimelineUpdated,
  scopeKey,
}: UseConversationHistoryParams): UseConversationHistoryResult => {
  const {
    executionProcessesVisible: executionProcessesRaw,
    isLoading,
    isConnected,
  } = useExecutionProcessesContext();
  const executionProcesses = useRef<ExecutionProcess[]>(executionProcessesRaw);
  const displayedExecutionProcesses = useRef<ExecutionProcessStateStore>({});
  const loadedInitialEntries = useRef(false);
  const emittedEmptyInitialRef = useRef(false);
  const streamingProcessIdsRef = useRef<Set<string>>(new Set());
  const onTimelineUpdatedRef = useRef<
    UseConversationHistoryParams['onTimelineUpdated'] | null
  >(null);
  const previousStatusMapRef = useRef<Map<string, ExecutionProcessStatus>>(
    new Map()
  );
  const [isLoadingHistoryState, setIsLoadingHistory] = useState(false);

  // Derive whether this is the first turn (no follow-up processes exist)
  const isFirstTurn = useMemo(() => {
    const codingAgentProcessCount = executionProcessesRaw.filter(
      (ep) =>
        ep.executor_action.typ.type === 'CodingAgentInitialRequest' ||
        ep.executor_action.typ.type === 'CodingAgentFollowUpRequest'
    ).length;
    return codingAgentProcessCount <= 1;
  }, [executionProcessesRaw]);

  const mergeIntoDisplayed = (
    mutator: (state: ExecutionProcessStateStore) => void
  ) => {
    const state = displayedExecutionProcesses.current;
    mutator(state);
  };

  // The hook owns transport, loading, and reconciliation.
  // It emits a source model that later derivation layers can transform further.

  const buildTimelineSource = useCallback(
    (
      executionProcessState: ExecutionProcessStateStore
    ): ConversationTimelineSource => ({
      executionProcessState,
      liveExecutionProcesses: executionProcesses.current,
    }),
    []
  );

  useEffect(() => {
    onTimelineUpdatedRef.current = onTimelineUpdated;
  }, [onTimelineUpdated]);

  // Keep executionProcesses up to date
  useEffect(() => {
    executionProcesses.current = executionProcessesRaw.filter(
      (ep) =>
        ep.run_reason === 'setupscript' ||
        ep.run_reason === 'cleanupscript' ||
        ep.run_reason === 'archivescript' ||
        ep.run_reason === 'codingagent'
    );
  }, [executionProcessesRaw]);

  const loadEntriesForHistoricExecutionProcess = (
    executionProcess: ExecutionProcess
  ) => {
    const startTime = performance.now();
    const processId = executionProcess.id.slice(0, 8);
    
    let url = '';
    if (executionProcess.executor_action.typ.type === 'ScriptRequest') {
      url = `/api/execution-processes/${executionProcess.id}/raw-logs/ws`;
    } else {
      url = `/api/execution-processes/${executionProcess.id}/normalized-logs/ws`;
    }

    console.log(`[History Load] Process ${processId} - Starting load, type: ${executionProcess.executor_action.typ.type}, status: ${executionProcess.status}`);

    return new Promise<PatchType[]>((resolve) => {
      const controller = streamJsonPatchEntries<PatchType>(url, {
        onFinished: (allEntries) => {
          const duration = performance.now() - startTime;
          console.log(`[History Load] Process ${processId} - Finished, entries: ${allEntries.length}, time: ${duration.toFixed(1)}ms`);
          controller.close();
          resolve(allEntries);
        },
        onError: (err) => {
          const duration = performance.now() - startTime;
          console.warn(
            `[History Load] Process ${processId} - Error after ${duration.toFixed(1)}ms:`,
            err
          );
          controller.close();
          resolve([]);
        },
      });
    });
  };

  const patchWithKey = (
    patch: PatchType,
    executionProcessId: string,
    index: number
  ) => {
    return {
      ...patch,
      patchKey: `${executionProcessId}:${index}`,
      executionProcessId,
    };
  };

  const flattenEntries = (
    executionProcessState: ExecutionProcessStateStore
  ): PatchTypeWithKey[] => {
    return Object.values(executionProcessState)
      .filter(
        (p) =>
          p.executionProcess.executor_action.typ.type ===
            'CodingAgentFollowUpRequest' ||
          p.executionProcess.executor_action.typ.type ===
            'CodingAgentInitialRequest' ||
          p.executionProcess.executor_action.typ.type === 'ReviewRequest'
      )
      .sort(
        (a, b) =>
          new Date(
            a.executionProcess.created_at as unknown as string
          ).getTime() -
          new Date(b.executionProcess.created_at as unknown as string).getTime()
      )
      .flatMap((p) => p.entries);
  };

  const getActiveAgentProcesses = (): ExecutionProcess[] => {
    return (
      executionProcesses?.current.filter(
        (p) =>
          p.status === ExecutionProcessStatus.running &&
          p.run_reason !== 'devserver'
      ) ?? []
    );
  };

  const emitEntries = useCallback(
    (
      executionProcessState: ExecutionProcessStateStore,
      addEntryType: AddEntryType,
      loading: boolean
    ) => {
      const timelineSource = buildTimelineSource(executionProcessState);
      let modifiedAddEntryType = addEntryType;

      const latestEntry = Object.values(executionProcessState)
        .sort(
          (a, b) =>
            new Date(
              a.executionProcess.created_at as unknown as string
            ).getTime() -
            new Date(
              b.executionProcess.created_at as unknown as string
            ).getTime()
        )
        .flatMap((processState) => processState.entries)
        .at(-1);

      if (
        latestEntry?.type === 'NORMALIZED_ENTRY' &&
        latestEntry.content.entry_type.type === 'tool_use' &&
        latestEntry.content.entry_type.tool_name === 'ExitPlanMode'
      ) {
        modifiedAddEntryType = 'plan';
      }

      onTimelineUpdatedRef.current?.(
        timelineSource,
        modifiedAddEntryType,
        loading
      );
    },
    [buildTimelineSource]
  );

  // This emits its own events as they are streamed
  const loadRunningAndEmit = useCallback(
    (executionProcess: ExecutionProcess): Promise<void> => {
      return new Promise((resolve, reject) => {
        let url = '';
        if (executionProcess.executor_action.typ.type === 'ScriptRequest') {
          url = `/api/execution-processes/${executionProcess.id}/raw-logs/ws`;
        } else {
          url = `/api/execution-processes/${executionProcess.id}/normalized-logs/ws`;
        }
        const controller = streamJsonPatchEntries<PatchType>(url, {
          onEntries(entries) {
            const patchesWithKey = entries.map((entry, index) =>
              patchWithKey(entry, executionProcess.id, index)
            );
            mergeIntoDisplayed((state) => {
              state[executionProcess.id] = {
                executionProcess,
                entries: patchesWithKey,
              };
            });
            emitEntries(displayedExecutionProcesses.current, 'running', false);
          },
          onFinished: () => {
            emitEntries(displayedExecutionProcesses.current, 'running', false);
            controller.close();
            resolve();
          },
          onError: () => {
            controller.close();
            reject();
          },
        });
      });
    },
    [emitEntries]
  );

  // Sometimes it can take a few seconds for the stream to start, wrap the loadRunningAndEmit method
  const loadRunningAndEmitWithBackoff = useCallback(
    async (executionProcess: ExecutionProcess) => {
      for (let i = 0; i < 20; i++) {
        try {
          await loadRunningAndEmit(executionProcess);
          break;
        } catch (_) {
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      }
    },
    [loadRunningAndEmit]
  );

  // 并发限制：同时最多 20 个 process 加载
  const MAX_CONCURRENT_LOADS = 20;

  // 带并发限制的并行加载工具函数
  const loadProcessesWithLimit = async (
    processes: ExecutionProcess[],
    onProcessLoaded: (result: {
      process: ExecutionProcess;
      entries: PatchTypeWithKey[];
    }) => boolean // 返回 true 表示继续加载，false 表示停止
  ): Promise<void> => {
    const totalStartTime = performance.now();
    const totalProcesses = processes.length;
    
    console.log(`[History Load] Starting batch load: ${totalProcesses} processes, batch size: ${MAX_CONCURRENT_LOADS}`);
    
    // 将 processes 分成多个 batch，每个 batch 最多 MAX_CONCURRENT_LOADS 个
    for (let i = 0; i < processes.length; i += MAX_CONCURRENT_LOADS) {
      const batch = processes.slice(i, i + MAX_CONCURRENT_LOADS);
      const batchNum = Math.floor(i / MAX_CONCURRENT_LOADS) + 1;
      const totalBatches = Math.ceil(processes.length / MAX_CONCURRENT_LOADS);
      
      console.log(`[History Load] Batch ${batchNum}/${totalBatches}: loading ${batch.length} processes concurrently`);
      const batchStartTime = performance.now();

      // 并行加载当前 batch
      const batchResults = await Promise.all(
        batch.map(async (executionProcess) => {
          const entries = await loadEntriesForHistoricExecutionProcess(executionProcess);
          const entriesWithKey = entries.map((e, idx) =>
            patchWithKey(e, executionProcess.id, idx)
          );
          return { process: executionProcess, entries: entriesWithKey };
        })
      );
      
      const batchDuration = performance.now() - batchStartTime;
      console.log(`[History Load] Batch ${batchNum}/${totalBatches}: completed in ${batchDuration.toFixed(1)}ms`);

      // 按原始顺序处理结果（保持时间线顺序）
      for (const result of batchResults) {
        const shouldContinue = onProcessLoaded(result);
        if (!shouldContinue) {
          const totalDuration = performance.now() - totalStartTime;
          console.log(`[History Load] Early stop after ${totalDuration.toFixed(1)}ms`);
          return;
        }
      }
    }
    
    const totalDuration = performance.now() - totalStartTime;
    console.log(`[History Load] All batches completed: ${totalProcesses} processes in ${totalDuration.toFixed(1)}ms`);
  };

  const loadHistoricEntries = useCallback(
    async (maxEntries?: number): Promise<ExecutionProcessStateStore> => {
      const startTime = performance.now();
      const localDisplayedExecutionProcesses: ExecutionProcessStateStore = {};

      if (!executionProcesses?.current) {
        console.log('[History Load] loadHistoricEntries: no processes to load');
        return localDisplayedExecutionProcesses;
      }

      // 过滤掉 running 状态的 process，并按时间倒序（最新的在前）
      const historicProcesses = [...executionProcesses.current]
        .reverse()
        .filter((p) => p.status !== ExecutionProcessStatus.running);
      
      const runningCount = executionProcesses.current.length - historicProcesses.length;
      console.log(`[History Load] loadHistoricEntries: ${historicProcesses.length} historic (skipped ${runningCount} running), maxEntries: ${maxEntries ?? 'none'}`);

      await loadProcessesWithLimit(
        historicProcesses,
        ({ process, entries }) => {
          localDisplayedExecutionProcesses[process.id] = {
            executionProcess: process,
            entries,
          };
          
          const currentCount = flattenEntries(localDisplayedExecutionProcesses).length;

          // 如果已达到 maxEntries，停止加载
          if (
            maxEntries != null &&
            currentCount > maxEntries
          ) {
            console.log(`[History Load] loadHistoricEntries: reached maxEntries ${maxEntries}, stopping`);
            return false; // 停止加载
          }
          return true; // 继续加载
        }
      );
      
      const totalDuration = performance.now() - startTime;
      const loadedCount = Object.keys(localDisplayedExecutionProcesses).length;
      const entryCount = flattenEntries(localDisplayedExecutionProcesses).length;
      console.log(`[History Load] loadHistoricEntries: completed - ${loadedCount} processes, ${entryCount} entries in ${totalDuration.toFixed(1)}ms`);

      return localDisplayedExecutionProcesses;
    },
    [executionProcesses]
  );

  const loadRemainingEntriesInBatches = useCallback(
    async (batchSize: number): Promise<boolean> => {
      const startTime = performance.now();
      
      if (!executionProcesses?.current) {
        console.log('[History Load] loadRemainingEntriesInBatches: no processes');
        return false;
      }

      // 获取尚未加载的 historic processes
      const current = displayedExecutionProcesses.current;
      const remainingProcesses = [...executionProcesses.current]
        .reverse()
        .filter(
          (p) =>
            !current[p.id] && p.status !== ExecutionProcessStatus.running
        );

      if (remainingProcesses.length === 0) {
        console.log('[History Load] loadRemainingEntriesInBatches: no remaining processes');
        return false;
      }
      
      console.log(`[History Load] loadRemainingEntriesInBatches: ${remainingProcesses.length} remaining, batchSize: ${batchSize}`);

      let anyUpdated = false;

      await loadProcessesWithLimit(
        remainingProcesses,
        ({ process, entries }) => {
          mergeIntoDisplayed((state) => {
            state[process.id] = {
              executionProcess: process,
              entries,
            };
          });

          anyUpdated = true;

          // 如果已达到 batchSize，停止加载
          const currentCount = flattenEntries(displayedExecutionProcesses.current).length;
          if (currentCount > batchSize) {
            console.log(`[History Load] loadRemainingEntriesInBatches: reached batchSize ${batchSize} (${currentCount} entries), stopping`);
            return false; // 停止加载
          }
          return true; // 继续加载
        }
      );
      
      const totalDuration = performance.now() - startTime;
      console.log(`[History Load] loadRemainingEntriesInBatches: completed - updated: ${anyUpdated} in ${totalDuration.toFixed(1)}ms`);

      return anyUpdated;
    },
    [executionProcesses]
  );

  const ensureProcessVisible = useCallback((p: ExecutionProcess) => {
    mergeIntoDisplayed((state) => {
      if (!state[p.id]) {
        state[p.id] = {
          executionProcess: {
            id: p.id,
            created_at: p.created_at,
            updated_at: p.updated_at,
            executor_action: p.executor_action,
          },
          entries: [],
        };
      }
    });
  }, []);

  const idListKey = useMemo(
    () => executionProcessesRaw?.map((p) => p.id).join(','),
    [executionProcessesRaw]
  );

  const idStatusKey = useMemo(
    () => executionProcessesRaw?.map((p) => `${p.id}:${p.status}`).join(','),
    [executionProcessesRaw]
  );

  // Clean up entries for processes that have been removed (e.g., after reset)
  useEffect(() => {
    if (isLoading || !isConnected) return;
    const visibleProcessIds = new Set(executionProcessesRaw.map((p) => p.id));
    const displayedIds = Object.keys(displayedExecutionProcesses.current);
    let changed = false;

    for (const id of displayedIds) {
      if (!visibleProcessIds.has(id)) {
        delete displayedExecutionProcesses.current[id];
        changed = true;
      }
    }

    if (changed) {
      emitEntries(displayedExecutionProcesses.current, 'historic', false);
    }
  }, [idListKey, executionProcessesRaw, emitEntries, isLoading, isConnected]);

  useEffect(() => {
    displayedExecutionProcesses.current = {};
    loadedInitialEntries.current = false;
    emittedEmptyInitialRef.current = false;
    streamingProcessIdsRef.current.clear();
    previousStatusMapRef.current.clear();
    emitEntries(displayedExecutionProcesses.current, 'initial', true);
  }, [scopeKey, emitEntries]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (loadedInitialEntries.current) return;

      if (isLoading) return;

      if (executionProcesses.current.length === 0) {
        if (emittedEmptyInitialRef.current) return;
        emittedEmptyInitialRef.current = true;
        emitEntries(displayedExecutionProcesses.current, 'initial', false);
        return;
      }

      emittedEmptyInitialRef.current = false;

      const allInitialEntries = await loadHistoricEntries(MIN_INITIAL_ENTRIES);
      if (cancelled) return;
      loadedInitialEntries.current = true;
      mergeIntoDisplayed((state) => {
        Object.assign(state, allInitialEntries);
      });
      emitEntries(displayedExecutionProcesses.current, 'initial', false);

      setIsLoadingHistory(true);
      while (
        !cancelled &&
        (await loadRemainingEntriesInBatches(REMAINING_BATCH_SIZE))
      ) {
        if (cancelled) return;
        emitEntries(displayedExecutionProcesses.current, 'historic', false);
      }
      if (!cancelled) setIsLoadingHistory(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [
    scopeKey,
    idListKey,
    isLoading,
    loadHistoricEntries,
    loadRemainingEntriesInBatches,
    emitEntries,
  ]); // include idListKey so new processes trigger reload

  useEffect(() => {
    const activeProcesses = getActiveAgentProcesses();
    if (activeProcesses.length === 0) return;

    for (const activeProcess of activeProcesses) {
      if (!displayedExecutionProcesses.current[activeProcess.id]) {
        const runningOrInitial =
          Object.keys(displayedExecutionProcesses.current).length > 1
            ? 'running'
            : 'initial';
        ensureProcessVisible(activeProcess);
        emitEntries(
          displayedExecutionProcesses.current,
          runningOrInitial,
          false
        );
      }

      if (
        activeProcess.status === ExecutionProcessStatus.running &&
        !streamingProcessIdsRef.current.has(activeProcess.id)
      ) {
        streamingProcessIdsRef.current.add(activeProcess.id);
        loadRunningAndEmitWithBackoff(activeProcess).finally(() => {
          streamingProcessIdsRef.current.delete(activeProcess.id);
        });
      }
    }
  }, [
    scopeKey,
    idStatusKey,
    emitEntries,
    ensureProcessVisible,
    loadRunningAndEmitWithBackoff,
  ]);

  useEffect(() => {
    if (!executionProcessesRaw) return;

    const processesToReload: ExecutionProcess[] = [];

    for (const process of executionProcessesRaw) {
      const previousStatus = previousStatusMapRef.current.get(process.id);
      const currentStatus = process.status;

      if (
        previousStatus === ExecutionProcessStatus.running &&
        currentStatus !== ExecutionProcessStatus.running &&
        displayedExecutionProcesses.current[process.id]
      ) {
        processesToReload.push(process);
      }

      previousStatusMapRef.current.set(process.id, currentStatus);
    }

    if (processesToReload.length === 0) return;

    (async () => {
      let anyUpdated = false;

      for (const process of processesToReload) {
        const entries = await loadEntriesForHistoricExecutionProcess(process);
        if (entries.length === 0) continue;

        const entriesWithKey = entries.map((e, idx) =>
          patchWithKey(e, process.id, idx)
        );

        mergeIntoDisplayed((state) => {
          state[process.id] = {
            executionProcess: process,
            entries: entriesWithKey,
          };
        });
        anyUpdated = true;
      }

      if (anyUpdated) {
        emitEntries(displayedExecutionProcesses.current, 'running', false);
      }
    })();
  }, [idStatusKey, executionProcessesRaw, emitEntries]);

  // If an execution process is removed, remove it from the state
  useEffect(() => {
    if (!executionProcessesRaw) return;

    const removedProcessIds = Object.keys(
      displayedExecutionProcesses.current
    ).filter((id) => !executionProcessesRaw.some((p) => p.id === id));

    if (removedProcessIds.length > 0) {
      mergeIntoDisplayed((state) => {
        removedProcessIds.forEach((id) => {
          delete state[id];
        });
      });
    }
  }, [scopeKey, idListKey, executionProcessesRaw]);

  return { isFirstTurn, isLoadingHistory: isLoadingHistoryState };
};
