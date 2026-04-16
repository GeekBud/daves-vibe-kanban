import { useShape } from '@/shared/integrations/electric/hooks';
import { PROJECTS_SHAPE } from 'shared/remote-types';
import { useAuth } from '@/shared/hooks/auth/useAuth';

export function useOrganizationProjects(organizationId: string | null) {
  const { isSignedIn, isLocalMode } = useAuth();

  // Subscribe when signed in or in local mode, AND have an org
  const enabled = (isSignedIn || isLocalMode) && !!organizationId;

  const { data, isLoading, error } = useShape(
    PROJECTS_SHAPE,
    { organization_id: organizationId || '' },
    { enabled }
  );

  return {
    data,
    isLoading,
    isError: !!error,
    error,
  };
}
