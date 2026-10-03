import type { ComponentType } from 'react'

/** Ein Zustand eines Screens, z. B. „Freigabe offen“ oder „leer“. */
export type ScreenState = { id: string; title: string }

export type Frame = 'desktop' | 'mobile' | 'terminal' | 'none'

export type Screen = {
  /** Eindeutig, kebab-case, z. B. `session-stream`. */
  id: string
  title: string
  /** Ein bis zwei Sätze: was man hier sieht und wozu. */
  description: string
  /** Feature-IDs, die dieser Screen zeigt. */
  features: string[]
  states?: ScreenState[]
  frame?: Frame
  component: ComponentType<{ state: string }>
}

/** Feature ohne eigene Oberfläche: warum, und wo man seine Wirkung sieht. */
export type NoUi = { reason: string; visibleIn?: string }

export type ScreenGroup = {
  id: string
  title: string
  /** Sortierung in der Navigation. */
  order: number
  screens: Screen[]
  noUi?: Record<string, NoUi>
}
