import { create } from 'zustand'

/**
 * Themes und Darstellung (UX-005, WEB-016, Entwurf `settings-appearance`). Ein Theme ist eine
 * Familie von Token-Sätzen (CSS-Variablen in `index.css`) in hell und dunkel: Sichtbeton
 * (Markenthema), Nord und Hoher Kontrast. „System“ ist Sichtbeton und folgt dem Betriebssystem
 * live, Nord und Hoher Kontrast ebenso. Die Auswahl gilt je Gerät (localStorage). Monaco, Shiki
 * und xterm leiten ihre Farben aus Familie und Modus ab, damit Editor, Code-Blöcke und Terminal
 * zum UI passen.
 */

export type ThemeId = 'system' | 'concrete-light' | 'concrete-dark' | 'nord' | 'hc'
export type Family = 'concrete' | 'nord' | 'hc'
export type Mode = 'light' | 'dark'
export type Motion = 'system' | 'on' | 'off'
export type FontSize = 13 | 14 | 16

export const THEMES: ThemeId[] = ['system', 'concrete-light', 'concrete-dark', 'nord', 'hc']

export interface Appearance {
  theme: ThemeId
  /** Bewegung reduzieren: wie System, an oder aus. */
  motion: Motion
  /** Fokusrahmen auch bei Mausbedienung. */
  focusAlways: boolean
  /** Streaming für Screenreader ansagen (abgeschlossene Absätze, gedrosselt). */
  announce: boolean
  fontSize: FontSize
  /** Theme vor „Hoher Kontrast“, für den Schalter in Barrierefreiheit. */
  beforeContrast?: ThemeId | undefined
}

export const DEFAULTS: Appearance = { theme: 'system', motion: 'system', focusAlways: true, announce: true, fontSize: 14 }

const STORAGE_KEY = 'beton.appearance.v1'

export function parseAppearance(raw: unknown): Appearance {
  const v = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
  return {
    theme: THEMES.includes(v.theme as ThemeId) ? (v.theme as ThemeId) : DEFAULTS.theme,
    motion: v.motion === 'on' || v.motion === 'off' ? v.motion : 'system',
    focusAlways: typeof v.focusAlways === 'boolean' ? v.focusAlways : DEFAULTS.focusAlways,
    announce: typeof v.announce === 'boolean' ? v.announce : DEFAULTS.announce,
    fontSize: v.fontSize === 13 || v.fontSize === 16 ? v.fontSize : 14,
    beforeContrast: THEMES.includes(v.beforeContrast as ThemeId) ? (v.beforeContrast as ThemeId) : undefined,
  }
}

function read(): Appearance {
  try {
    return parseAppearance(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null'))
  } catch {
    return { ...DEFAULTS }
  }
}

/** Familie und Modus eines Themes bei gegebener Systemeinstellung. */
export function resolveTheme(theme: ThemeId, systemDark: boolean): { family: Family; mode: Mode } {
  const sys: Mode = systemDark ? 'dark' : 'light'
  switch (theme) {
    case 'concrete-light':
      return { family: 'concrete', mode: 'light' }
    case 'concrete-dark':
      return { family: 'concrete', mode: 'dark' }
    case 'nord':
      return { family: 'nord', mode: sys }
    case 'hc':
      return { family: 'hc', mode: sys }
    default:
      return { family: 'concrete', mode: sys }
  }
}

/** Shiki-Theme zu Familie und Modus (Code-Blöcke im Chat, Diff). */
export function shikiTheme(family: Family, mode: Mode): string {
  if (family === 'hc') return mode === 'dark' ? 'github-dark-high-contrast' : 'github-light-high-contrast'
  if (family === 'nord') return mode === 'dark' ? 'nord' : 'github-light'
  return mode === 'dark' ? 'github-dark' : 'github-light'
}

/** Basis-Theme von Monaco; Farben kommen aus den Tokens. */
export function monacoBase(family: Family, mode: Mode): 'vs' | 'vs-dark' | 'hc-light' | 'hc-black' {
  if (family === 'hc') return mode === 'dark' ? 'hc-black' : 'hc-light'
  return mode === 'dark' ? 'vs-dark' : 'vs'
}

/** Terminal-Farben (xterm.js `ITheme`) aus den Tokens eines Elements. */
export function terminalTheme(read: (name: string) => string): Record<string, string> {
  const v = (name: string, fallback: string) => read(name).trim() || fallback
  return {
    background: v('--sunken', '#161819'),
    foreground: v('--foreground', '#dcddd8'),
    cursor: v('--foreground', '#dcddd8'),
    cursorAccent: v('--sunken', '#161819'),
    selectionBackground: v('--accent', '#30343a'),
    black: v('--sunken', '#161819'),
    red: v('--deny', '#e0675b'),
    green: v('--ok', '#5bab78'),
    yellow: v('--signal', '#e2b33b'),
    blue: v('--voice-direct', '#93a2bf'),
    magenta: v('--voice-claude', '#a690d4'),
    cyan: v('--voice-codex', '#56b0b7'),
    white: v('--foreground', '#dcddd8'),
    brightBlack: v('--muted-foreground', '#9a9ea4'),
    brightRed: v('--deny', '#e0675b'),
    brightGreen: v('--ok', '#5bab78'),
    brightYellow: v('--warn', '#e8803c'),
    brightBlue: v('--voice-direct', '#93a2bf'),
    brightMagenta: v('--voice-claude', '#a690d4'),
    brightCyan: v('--voice-codex', '#56b0b7'),
    brightWhite: v('--foreground', '#dcddd8'),
  }
}

interface ThemeState extends Appearance {
  systemDark: boolean
  family: Family
  mode: Mode
  update: (patch: Partial<Appearance>) => void
}

const prefersDark = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-color-scheme: dark)').matches

export const useTheme = create<ThemeState>()((set, get) => {
  const initial = read()
  const systemDark = prefersDark()
  return {
    ...initial,
    systemDark,
    ...resolveTheme(initial.theme, systemDark),
    update: (patch) => {
      const next = { ...pick(get()), ...patch }
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
      } catch {
        // Gilt bis zum Neuladen.
      }
      set({ ...next, ...resolveTheme(next.theme, get().systemDark) })
    },
  }
})

function pick(s: ThemeState): Appearance {
  return { theme: s.theme, motion: s.motion, focusAlways: s.focusAlways, announce: s.announce, fontSize: s.fontSize, beforeContrast: s.beforeContrast }
}

/** Klassen und Attribute am Wurzelelement setzen (Theme, Bewegung, Fokus, Schriftgröße). */
export function applyToDocument(s: Pick<ThemeState, 'family' | 'mode' | 'motion' | 'focusAlways' | 'fontSize'>, root: HTMLElement = document.documentElement): void {
  root.classList.toggle('theme-nord', s.family === 'nord')
  root.classList.toggle('theme-hc', s.family === 'hc')
  root.classList.toggle('dark', s.mode === 'dark')
  root.classList.toggle('light', s.mode === 'light')
  root.style.colorScheme = s.mode
  if (s.motion === 'system') delete root.dataset.motion
  else root.dataset.motion = s.motion === 'on' ? 'reduce' : 'full'
  if (s.focusAlways) root.dataset.focus = 'always'
  else delete root.dataset.focus
  root.style.fontSize = `${s.fontSize}px`
}

/** Theme anwenden und der Systemeinstellung live folgen (UX-005 AC3). */
export function startTheme(): () => void {
  const mq = window.matchMedia('(prefers-color-scheme: dark)')
  const onSystem = () =>
    useTheme.setState((s) => ({ systemDark: mq.matches, ...resolveTheme(s.theme, mq.matches) }))
  onSystem()
  applyToDocument(useTheme.getState())
  const unsub = useTheme.subscribe((s) => applyToDocument(s))
  mq.addEventListener('change', onSystem)
  return () => {
    unsub()
    mq.removeEventListener('change', onSystem)
  }
}

/** Hell/dunkel des aktuellen Themes umschalten (Befehl „Theme wechseln“). */
export function toggleMode(): void {
  const s = useTheme.getState()
  const dark = s.mode === 'dark'
  if (s.family === 'concrete') s.update({ theme: dark ? 'concrete-light' : 'concrete-dark' })
  // Nord und Hoher Kontrast folgen dem System; der Wechsel geht auf Sichtbeton im Gegenmodus.
  else s.update({ theme: dark ? 'concrete-light' : 'concrete-dark' })
}

/** Schalter „Hoher Kontrast“: wechselt auf das Theme und zurück. */
export function setHighContrast(on: boolean): void {
  const s = useTheme.getState()
  if (on && s.theme !== 'hc') s.update({ theme: 'hc', beforeContrast: s.theme })
  else if (!on && s.theme === 'hc') s.update({ theme: s.beforeContrast ?? 'system', beforeContrast: undefined })
}
