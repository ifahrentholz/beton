import { create } from 'zustand'

/**
 * Zuletzt genutzte Sessions (UX-003): Reihenfolge je Gerät im localStorage, überlebt Neustarts
 * (AC3). Die zuletzt geöffnete Session steht vorn.
 */

const STORAGE_KEY = 'beton.mru.v1'
export const MRU_MAX = 50

function read(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]') as unknown
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string').slice(0, MRU_MAX) : []
  } catch {
    return []
  }
}

interface MruState {
  ids: string[]
  touch: (id: string) => void
}

export const useMru = create<MruState>()((set, get) => ({
  ids: read(),
  touch: (id) => {
    const ids = [id, ...get().ids.filter((x) => x !== id)].slice(0, MRU_MAX)
    if (ids.join() === get().ids.join()) return
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(ids))
    } catch {
      // Ohne Speicher gilt die Reihenfolge bis zum Neuladen.
    }
    set({ ids })
  },
}))

/** Für Tests: Stand aus dem Speicher neu lesen. */
export function reloadMru(): void {
  useMru.setState({ ids: read() })
}

/**
 * Reihenfolge des Switchers: zuletzt genutzte zuerst, danach alle übrigen nach jüngster
 * Aktivität.
 */
export function mruOrder<T extends { id: string; last_activity_at: string }>(sessions: T[], mru: string[]): T[] {
  const rank = new Map(mru.map((id, i) => [id, i]))
  return [...sessions].sort((a, b) => {
    const ra = rank.get(a.id)
    const rb = rank.get(b.id)
    if (ra !== undefined && rb !== undefined) return ra - rb
    if (ra !== undefined) return -1
    if (rb !== undefined) return 1
    return a.last_activity_at < b.last_activity_at ? 1 : a.last_activity_at > b.last_activity_at ? -1 : 0
  })
}
