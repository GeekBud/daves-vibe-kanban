import { useState, useMemo, useCallback, useEffect, useRef } from 'react';
import { useLiveQuery } from '@tanstack/react-db';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createShapeCollection } from '@/shared/lib/electric/collections';
import { useSyncErrorContext } from '@/shared/hooks/useSyncErrorContext';
import { useAuth } from '@/shared/hooks/auth/useAuth';
import { makeRequest } from '@/shared/lib/remoteApi';
import type { MutationDefinition, ShapeDefinition } from 'shared/remote-types';
import type { SyncError } from '@/shared/lib/electric/types';
import type { MutationResult, InsertResult } from '@/shared/lib/electric/types';

// Type helpers for extracting types from MutationDefinition
type MutationCreateType<M> =
  M extends MutationDefinition<unknown, infer C, unknown> ? C : never;
type MutationUpdateType<M> =
  M extends MutationDefinition<unknown, unknown, infer U> ? U : never;

/**
 * Base result type returned by useShape (read-only).
 */
export interface UseShapeResult<TRow> {
  /** The synced data array */
  data: TRow[];
  /** Whether the initial sync is still loading */
  isLoading: boolean;
  /** Sync error if one occurred */
  error: SyncError | null;
  /** Function to retry after an error */
  retry: () => void;
}

/**
 * Extended result when mutation is provided — adds insert/update/remove.
 */
export interface UseShapeMutationResult<TRow, TCreate, TUpdate>
  extends UseShapeResult<TRow> {
  /** Insert a new row (optimistic), returns row and persistence promise */
  insert: (data: TCreate) => InsertResult<TRow>;
  /** Update a row by ID (optimistic), returns persistence promise */
  update: (id: string, changes: Partial<TUpdate>) => MutationResult;
  /** Update multiple rows in a single optimistic transaction */
  updateMany: (
    updates: Array<{ id: string; changes: Partial<TUpdate> }>
  ) => MutationResult;
  /** Delete a row by ID (optimistic), returns persistence promise */
  remove: (id: string) => MutationResult;
}

/**
 * Options for the useShape hook.
 */
export interface UseShapeOptions<
  M extends
    | MutationDefinition<unknown, unknown, unknown>
    | undefined = undefined,
> {
  /**
   * Whether to enable the Electric sync subscription.
   * When false, returns empty data and no-op mutation functions.
   * @default true
   */
  enabled?: boolean;
  /**
   * Optional mutation definition. When provided, the hook returns
   * insert/update/remove functions for optimistic mutations.
   */
  mutation?: M;
}

// =============================================================================
// Local-mode helpers
// =============================================================================

function buildFallbackUrl(
  fallbackUrl: string,
  params: Record<string, string>
): string {
  let url = fallbackUrl;
  for (const [key, value] of Object.entries(params)) {
    url = url.replace(`{${key}}`, encodeURIComponent(value));
  }
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (!value) continue;
    query.set(key, value);
  }
  const queryString = query.toString();
  return queryString ? `${url}?${queryString}` : url;
}

async function fetchLocalShape<T>(
  shape: ShapeDefinition<T>,
  params: Record<string, string>
): Promise<T[]> {
  const path = buildFallbackUrl(shape.fallbackUrl, params);
  const response = await makeRequest(path, { method: 'GET', cache: 'no-store' });
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.message || body.error || `Failed to fetch ${shape.table}`);
  }
  const payload = (await response.json()) as Record<string, unknown>;
  // Local backend wraps responses in ApiResponse { success, data }
  const data = (payload.data ?? payload) as Record<string, unknown>;
  const rows = data[shape.table];
  if (!Array.isArray(rows)) {
    throw new Error(`Fallback response missing "${shape.table}" array`);
  }
  return rows as T[];
}

async function parseError(response: Response, fallback: string): Promise<never> {
  const body = await response.json().catch(() => ({}));
  throw new Error(body.message || body.error || fallback);
}

// =============================================================================
// useShape hook
// =============================================================================

/**
 * Hook for subscribing to a shape's data via Electric sync,
 * with optional optimistic mutation support.
 *
 * In local mode, falls back to polling the local backend API.
 */
export function useShape<
  T extends Record<string, unknown>,
  M extends
    | MutationDefinition<unknown, unknown, unknown>
    | undefined = undefined,
>(
  shape: ShapeDefinition<T>,
  params: Record<string, string>,
  options: UseShapeOptions<M> = {} as UseShapeOptions<M>
): M extends MutationDefinition<unknown, unknown, unknown>
  ? UseShapeMutationResult<T, MutationCreateType<M>, MutationUpdateType<M>>
  : UseShapeResult<T> {
  const { enabled = true, mutation } = options;
  const { isLocalMode } = useAuth();
  const queryClient = useQueryClient();

  const [error, setError] = useState<SyncError | null>(null);
  const [retryKey, setRetryKey] = useState(0);

  const syncErrorContext = useSyncErrorContext();
  const registerErrorFn = syncErrorContext?.registerError;
  const clearErrorFn = syncErrorContext?.clearError;

  const handleError = useCallback((err: SyncError) => setError(err), []);

  const retry = useCallback(() => {
    setError(null);
    setRetryKey((k) => k + 1);
    if (isLocalMode) {
      queryClient.invalidateQueries({ queryKey: ['local-shape', shape.table, params] });
    }
  }, [isLocalMode, queryClient, shape.table, params]);

  const paramsKey = JSON.stringify(params);
  const stableParams = useMemo(
    () => JSON.parse(paramsKey) as Record<string, string>,
    [paramsKey]
  );

  const streamId = useMemo(
    () => `${shape.table}:${paramsKey}`,
    [shape.table, paramsKey]
  );

  useEffect(() => {
    if (error && registerErrorFn) {
      registerErrorFn(streamId, shape.table, error, retry);
    } else if (!error && clearErrorFn) {
      clearErrorFn(streamId);
    }

    return () => {
      clearErrorFn?.(streamId);
    };
  }, [error, streamId, shape.table, retry, registerErrorFn, clearErrorFn]);

  // ---------------------------------------------------------------------------
  // Local mode: use TanStack Query polling
  // ---------------------------------------------------------------------------
  const localQuery = useQuery({
    queryKey: ['local-shape', shape.table, stableParams, retryKey],
    queryFn: () => fetchLocalShape(shape, stableParams),
    enabled: enabled && isLocalMode,
    refetchInterval: 3000,
    staleTime: 2000,
  });

  // ---------------------------------------------------------------------------
  // Electric mode: use Electric sync
  // ---------------------------------------------------------------------------
  const collection = useMemo(() => {
    if (!enabled || isLocalMode) return null;
    const config = { onError: handleError };
    void retryKey;
    return createShapeCollection(shape, stableParams, config, mutation);
  }, [enabled, isLocalMode, shape, mutation, handleError, retryKey, stableParams]);

  const { data: liveData, isLoading: queryLoading } = useLiveQuery(
    (query) => (collection ? query.from({ item: collection }) : undefined),
    [collection]
  );

  const items = useMemo(() => {
    if (!enabled) return [];
    if (isLocalMode) {
      return (localQuery.data ?? []) as T[];
    }
    if (!collection || !liveData || queryLoading) return [];
    return liveData as unknown as T[];
  }, [enabled, isLocalMode, localQuery.data, collection, liveData, queryLoading]);

  const isLoading = enabled
    ? isLocalMode
      ? localQuery.isLoading
      : queryLoading
    : false;

  const localError = localQuery.error
    ? { message: (localQuery.error as Error).message }
    : null;
  const effectiveError = isLocalMode ? localError : error;

  // --- Mutation support ---

  const itemsRef = useRef<T[]>([]);
  useEffect(() => {
    itemsRef.current = items;
  }, [items]);

  type TransactionResult = { isPersisted: { promise: Promise<void> } };
  type CollectionWithMutations = {
    insert: (data: unknown) => TransactionResult;
    update: {
      (
        id: string,
        updater: (draft: Record<string, unknown>) => void
      ): TransactionResult;
      (
        ids: string[],
        updater: (drafts: Array<Record<string, unknown>>) => void
      ): TransactionResult;
    };
    delete: (id: string) => TransactionResult;
  };
  const typedCollection =
    collection as unknown as CollectionWithMutations | null;

  const insert = useCallback(
    (insertData: unknown): InsertResult<T> => {
      const dataWithId = {
        id: crypto.randomUUID(),
        ...(insertData as Record<string, unknown>),
      };

      if (isLocalMode && mutation) {
        const promise = makeRequest(mutation.url, {
          method: 'POST',
          body: JSON.stringify(dataWithId),
        })
          .then(async (res) => {
            if (!res.ok) await parseError(res, `Failed to create ${mutation.name}`);
            // invalidate after mutation
            queryClient.invalidateQueries({ queryKey: ['local-shape', shape.table, stableParams] });
          })
          .then(() => {
            const synced = itemsRef.current.find(
              (item) => (item as unknown as { id: string }).id === dataWithId.id
            );
            return (synced ?? dataWithId) as unknown as T;
          });
        return { data: dataWithId as unknown as T, persisted: promise };
      }

      if (!typedCollection) {
        return {
          data: dataWithId as unknown as T,
          persisted: Promise.resolve(dataWithId as unknown as T),
        };
      }
      const tx = typedCollection.insert(dataWithId);
      return {
        data: dataWithId as unknown as T,
        persisted: tx.isPersisted.promise.then(() => {
          const synced = itemsRef.current.find(
            (item) => (item as unknown as { id: string }).id === dataWithId.id
          );
          return (synced ?? dataWithId) as unknown as T;
        }),
      };
    },
    [typedCollection, isLocalMode, mutation, queryClient, shape.table, stableParams]
  );

  const update = useCallback(
    (id: string, changes: unknown): MutationResult => {
      if (isLocalMode && mutation) {
        const promise = makeRequest(`${mutation.url}/${id}`, {
          method: 'POST',
          body: JSON.stringify(changes),
        })
          .then(async (res) => {
            if (!res.ok) await parseError(res, `Failed to update ${mutation.name}`);
            queryClient.invalidateQueries({ queryKey: ['local-shape', shape.table, stableParams] });
          });
        return { persisted: promise };
      }

      if (!typedCollection) {
        return { persisted: Promise.resolve() };
      }
      const tx = typedCollection.update(id, (draft: Record<string, unknown>) =>
        Object.assign(draft, changes)
      );
      return { persisted: tx.isPersisted.promise };
    },
    [typedCollection, isLocalMode, mutation, queryClient, shape.table, stableParams]
  );

  const updateMany = useCallback(
    (updates: Array<{ id: string; changes: unknown }>): MutationResult => {
      if (isLocalMode && mutation && updates.length > 0) {
        const promise = makeRequest(`${mutation.url}/bulk`, {
          method: 'POST',
          body: JSON.stringify({ updates: updates.map((u) => ({ id: u.id, ...(u.changes as Record<string, unknown>) })) }),
        })
          .then(async (res) => {
            if (!res.ok) await parseError(res, `Failed to bulk update ${mutation.name}`);
            queryClient.invalidateQueries({ queryKey: ['local-shape', shape.table, stableParams] });
          });
        return { persisted: promise };
      }

      if (!typedCollection || updates.length === 0) {
        return { persisted: Promise.resolve() };
      }

      const ids = updates.map((update) => update.id);
      const changesById = new Map(
        updates.map((update) => [update.id, update.changes])
      );

      const tx = typedCollection.update(
        ids,
        (drafts: Array<Record<string, unknown>>) => {
          for (const draft of drafts) {
            const draftId = String(draft.id ?? '');
            const changes = changesById.get(draftId);
            if (changes) {
              Object.assign(draft, changes);
            }
          }
        }
      );

      return { persisted: tx.isPersisted.promise };
    },
    [typedCollection, isLocalMode, mutation, queryClient, shape.table, stableParams]
  );

  const remove = useCallback(
    (id: string): MutationResult => {
      if (isLocalMode && mutation) {
        const promise = makeRequest(`${mutation.url}/${id}`, {
          method: 'DELETE',
        })
          .then(async (res) => {
            if (!res.ok) await parseError(res, `Failed to delete ${mutation.name}`);
            queryClient.invalidateQueries({ queryKey: ['local-shape', shape.table, stableParams] });
          });
        return { persisted: promise };
      }

      if (!typedCollection) {
        return { persisted: Promise.resolve() };
      }
      const tx = typedCollection.delete(id);
      return { persisted: tx.isPersisted.promise };
    },
    [typedCollection, isLocalMode, mutation, queryClient, shape.table, stableParams]
  );

  const base: UseShapeResult<T> = {
    data: items,
    isLoading,
    error: effectiveError,
    retry,
  };

  if (mutation) {
    return {
      ...base,
      insert,
      update,
      updateMany,
      remove,
    } as M extends MutationDefinition<unknown, unknown, unknown>
      ? UseShapeMutationResult<T, MutationCreateType<M>, MutationUpdateType<M>>
      : UseShapeResult<T>;
  }

  return base as M extends MutationDefinition<unknown, unknown, unknown>
    ? UseShapeMutationResult<T, MutationCreateType<M>, MutationUpdateType<M>>
    : UseShapeResult<T>;
}
