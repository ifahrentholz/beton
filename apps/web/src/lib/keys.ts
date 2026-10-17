/**
 * Tastenkürzel als Text (UX-004): `Mod+K`, `Mod+Shift+Enter`, `Escape Escape` (Folge zweier
 * Anschläge), `F2`. `Mod` ist ⌘ auf macOS und Ctrl sonst. Mehrere Kürzel einer Zeile im
 * Overlay stehen mit ` / ` getrennt (`J / K`).
 */

/** Läuft die UI auf macOS? Entscheidet über ⌘ oder Ctrl. */
export function isMac(): boolean {
  if (typeof navigator === 'undefined') return false
  const data = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData
  const platform = data?.platform || navigator.platform || ''
  return /mac|iphone|ipad/i.test(platform)
}

const MODIFIERS = ['Mod', 'Ctrl', 'Alt', 'Shift'] as const
type Modifier = (typeof MODIFIERS)[number]

/**
 * Taste aus dem Event: Buchstaben nach Position (⌥S liefert sonst „ß“), Ziffern nur mit ⌥ nach
 * Position, sonst das getippte Zeichen (auf deutschen Tastaturen ist `/` die Taste 7).
 */
function keyOf(e: Pick<KeyboardEvent, 'key' | 'code' | 'altKey'>): string {
  const code = e.code ?? ''
  if (/^Key[A-Z]$/.test(code)) return code.slice(3)
  if (/^Digit\d$/.test(code) && e.altKey) return code.slice(5)
  if (e.key === ' ') return 'Space'
  return e.key.length === 1 ? e.key.toUpperCase() : e.key
}

/** Nur eine Modifier-Taste gedrückt? */
export function isModifierKey(key: string): boolean {
  return key === 'Meta' || key === 'Control' || key === 'Alt' || key === 'Shift' || key === 'AltGraph' || key === 'OS'
}

/**
 * Ein Anschlag als Text, z. B. `Mod+Shift+M`. Shift zählt bei Satzzeichen nicht mit, weil es je
 * nach Tastaturbelegung nötig ist, um das Zeichen zu tippen (`/` ist auf deutschen Tastaturen ⇧7).
 */
export function strokeOf(e: Pick<KeyboardEvent, 'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>, mac = isMac()): string {
  const key = keyOf(e)
  const mods: Modifier[] = []
  // Außerhalb von macOS zählt auch Meta als Mod (so bleiben ⌘-Kürzel aus Tests und Fernsitzungen gültig).
  if (mac ? e.metaKey : e.ctrlKey || e.metaKey) mods.push('Mod')
  if (mac && e.ctrlKey) mods.push('Ctrl')
  if (e.altKey) mods.push('Alt')
  const punctuation = key.length === 1 && !/[A-Z0-9]/.test(key)
  if (e.shiftKey && !punctuation) mods.push('Shift')
  return [...mods, key].join('+')
}

/** Normalform eines Kürzels (Reihenfolge der Modifier, Großbuchstaben). */
export function normalize(chord: string): string {
  return chord
    .split(' / ')
    .map((alt) =>
      alt
        .split(' ')
        .filter(Boolean)
        .map((stroke) => {
          const parts = stroke.split('+')
          const key = parts.pop() ?? ''
          const mods = MODIFIERS.filter((m) => parts.includes(m))
          return [...mods, key.length === 1 ? key.toUpperCase() : key].join('+')
        })
        .join(' '),
    )
    .join(' / ')
}

/** Hat das Kürzel einen Modifier oder ist eine Funktionstaste? Nur solche lassen sich belegen. */
export function assignable(chord: string): boolean {
  const parts = (chord.split(' ')[0] ?? '').split('+')
  const key = parts.pop() ?? ''
  return parts.some((m) => m === 'Mod' || m === 'Ctrl' || m === 'Alt') || /^F\d{1,2}$/.test(key)
}

const MAC_SYMBOL: Record<string, string> = { Mod: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' }
const OTHER_NAME: Record<string, string> = { Mod: 'Ctrl', Ctrl: 'Ctrl', Alt: 'Alt', Shift: 'Shift' }
const KEY_SYMBOL: Record<string, string> = {
  Enter: '↵',
  Backspace: '⌫',
  Escape: 'Esc',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  Space: 'Space',
  Tab: 'Tab',
}

/** Ein Anschlag zur Anzeige: `⌘⇧M` auf macOS, `Ctrl+Shift+M` sonst; einzelne Buchstaben klein. */
export function formatStroke(stroke: string, mac = isMac()): string {
  const parts = stroke.split('+')
  const key = parts.pop() ?? ''
  const shown = KEY_SYMBOL[key] ?? (parts.length === 0 && key.length === 1 ? key.toLowerCase() : key)
  if (parts.length === 0) return shown
  if (mac) return parts.map((m) => MAC_SYMBOL[m] ?? m).join('') + shown
  return [...parts.map((m) => OTHER_NAME[m] ?? m), shown].join('+')
}

/** Ganzes Kürzel zur Anzeige als Liste von Anschlägen; `/` trennt Alternativen. */
export function formatChord(chord: string, mac = isMac()): string[] {
  return chord.split(' / ').flatMap((alt, i) => [...(i > 0 ? ['/'] : []), ...alt.split(' ').map((s) => formatStroke(s, mac))])
}

/** Kürzel als ein Text, z. B. für Titel und Hinweise. */
export function chordText(chord: string, mac = isMac()): string {
  return formatChord(chord, mac).join(' ')
}

/** Ist das Ziel ein Texteingabefeld (Input, Textarea, Select, contenteditable)? */
export function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  if (target.isContentEditable) return true
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true
  if (target instanceof HTMLInputElement) {
    return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file'].includes(target.type)
  }
  return false
}
