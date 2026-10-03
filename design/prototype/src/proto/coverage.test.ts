import { describe, expect, it } from 'vitest'
import { featureById, features } from '@/catalog/types'
import { coverage, groups, screenById, screens } from './registry'

// Optional einschränken: PREFIXES=HAR,AGT pnpm test
const only = process.env.PREFIXES?.split(',').map((p) => p.trim()).filter(Boolean)
const relevant = features.filter((f) => !only?.length || only.includes(f.prefix))

describe('Design-Prototyp deckt die Spec ab (ADR-0032)', () => {
  it('jedes Feature hat einen Screen oder einen „kein UI“-Eintrag', () => {
    const open = relevant.filter((f) => {
      const c = coverage.get(f.id)!
      return !c.screens.length && !c.noUi
    })
    expect(open.map((f) => `${f.id} ${f.title}`)).toEqual([])
  })

  it('Screens referenzieren nur existierende Feature-IDs', () => {
    const unknown = screens.flatMap((s) => s.features.filter((id) => !featureById.has(id)).map((id) => `${s.id}: ${id}`))
    expect(unknown).toEqual([])
  })

  it('„kein UI“-Einträge sind gültig und verweisen auf existierende Screens', () => {
    const problems: string[] = []
    for (const g of groups) {
      for (const [id, entry] of Object.entries(g.noUi ?? {})) {
        if (!featureById.has(id)) problems.push(`${g.id}: unbekannte ID ${id}`)
        if (!entry.reason.trim()) problems.push(`${g.id}: ${id} ohne Begründung`)
        if (entry.visibleIn && !screenById.has(entry.visibleIn)) problems.push(`${g.id}: ${id} → unbekannter Screen ${entry.visibleIn}`)
      }
    }
    expect(problems).toEqual([])
  })

  it('Screen-IDs sind eindeutig', () => {
    const ids = screens.map((s) => s.id)
    expect(ids.filter((id, i) => ids.indexOf(id) !== i)).toEqual([])
  })
})
