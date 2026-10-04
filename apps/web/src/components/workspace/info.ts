import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'

/** Überblick über den Workspace (ist er ein Git-Repository?), einmal je Session. */
export function useWorkspaceInfo(sessionId: string) {
  return useQuery({
    queryKey: ['ws-info', sessionId],
    queryFn: () => client.session(sessionId).workspace.info(),
    staleTime: Infinity,
    retry: false,
  })
}
