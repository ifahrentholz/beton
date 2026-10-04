import { useQuery } from '@tanstack/react-query'
import { client } from './client'

/**
 * Aktive Feature-Flags des Servers aus `GET /v1/info` (UX-007). Solange sie nicht geladen
 * sind, gilt nichts als aktiv: Unfertiges bleibt ausgeblendet.
 */
export function useFeatures(): ReadonlySet<string> {
  const info = useQuery({ queryKey: ['info'], queryFn: () => client.info(), staleTime: Infinity })
  return new Set(info.data?.features ?? [])
}

/** Harnesses, die nur hinter einem Flag erreichbar sind. */
const HARNESS_FLAGS: Record<string, string> = { fake: 'fake_harness' }

/** Ist ein Harness laut Flags sichtbar? (UX-007 AC1: ohne Aktivierung ausgeblendet.) */
export function harnessVisible(id: string, features: ReadonlySet<string>): boolean {
  const flag = HARNESS_FLAGS[id]
  return flag === undefined || features.has(flag)
}
