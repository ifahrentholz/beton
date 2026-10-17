import { create } from 'zustand'

/** Offene Overlays der App: Befehlspalette, Session-Switcher, Tastenkürzel, Neue Session. */
export interface UiState {
  palette: boolean
  /** Session-Switcher; `cycle` heißt: per Ctrl+Tab geöffnet, Loslassen von Ctrl wechselt. */
  switcher: false | 'open' | 'cycle'
  shortcuts: boolean
  newSession: boolean
  set: (patch: Partial<Omit<UiState, 'set'>>) => void
}

export const useUi = create<UiState>()((set) => ({
  palette: false,
  switcher: false,
  shortcuts: false,
  newSession: false,
  set: (patch) => set(patch),
}))

/** Nur ein Overlay zur Zeit. */
export function openOnly(which: 'palette' | 'switcher' | 'shortcuts', switcher: 'open' | 'cycle' = 'open'): void {
  useUi.getState().set({ palette: which === 'palette', shortcuts: which === 'shortcuts', switcher: which === 'switcher' ? switcher : false })
}
