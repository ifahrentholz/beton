import { create } from 'zustand'

/** Sichtbarkeit der Session-Liste unter 1024 px (Drawer, WEB-001 AC3). */
export const useLayout = create<{ listOpen: boolean; toggleList: () => void; closeList: () => void }>()((set) => ({
  listOpen: false,
  toggleList: () => set((s) => ({ listOpen: !s.listOpen })),
  closeList: () => set({ listOpen: false }),
}))
