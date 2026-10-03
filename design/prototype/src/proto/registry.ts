import { features } from '@/catalog/types'
import type { NoUi, Screen, ScreenGroup } from './types'

// Jede Gruppe liegt in src/screens/<gruppe>/index.tsx und exportiert `group`.
// Neue Gruppen werden automatisch eingesammelt; keine zentrale Datei muss geändert werden.
const modules = import.meta.glob<{ group: ScreenGroup }>('../screens/*/index.tsx', {
  eager: true,
})

export const groups: ScreenGroup[] = Object.values(modules)
  .map((m) => m.group)
  .sort((a, b) => a.order - b.order || a.title.localeCompare(b.title))

export const screens: Screen[] = groups.flatMap((g) => g.screens)
export const screenById = new Map(screens.map((s) => [s.id, s]))
export const groupOfScreen = new Map(groups.flatMap((g) => g.screens.map((s) => [s.id, g])))

export type Coverage = { screens: Screen[]; noUi?: NoUi }

/** Wo jedes Feature im Prototyp zu sehen ist. */
export const coverage: Map<string, Coverage> = (() => {
  const map = new Map<string, Coverage>(features.map((f) => [f.id, { screens: [] }]))
  for (const s of screens) {
    for (const id of s.features) map.get(id)?.screens.push(s)
  }
  for (const g of groups) {
    for (const [id, entry] of Object.entries(g.noUi ?? {})) {
      const c = map.get(id)
      if (c) c.noUi = entry
    }
  }
  return map
})()

export const coverageStats = () => {
  let withScreen = 0
  let noUi = 0
  let open = 0
  for (const c of coverage.values()) {
    if (c.screens.length) withScreen++
    else if (c.noUi) noUi++
    else open++
  }
  return { withScreen, noUi, open, total: coverage.size }
}
