import { useEffect, useRef, type ComponentType } from 'react'
import { create } from 'zustand'
import { t as currentT, type T } from './i18n'
import { DECLARED } from './keymap'
import { assignable, isModifierKey, isTyping, normalize, strokeOf } from './keys'

/**
 * Zentrale Befehls-Registry (UX-002, UX-004): Befehlspalette, Tastenkürzel und Shortcuts-Overlay
 * lesen aus ihr. Ein Befehl wird einmal deklariert (Titel, Kürzel, Gruppe) und dort, wo er wirkt,
 * mit `useCommand` an Ausführung und Bedingung (`when`) gebunden. Die Palette zeigt nur Befehle mit
 * Ausführung, deren Bedingung erfüllt ist; das Overlay zeigt alle Befehle mit Kürzel.
 *
 * Kürzel greifen nicht, solange ein Texteingabefeld Fokus hat, außer bei `globalInInput`
 * (z. B. `⌘K`, UX-004 AC3). `local` heißt: Die Komponente wertet die Taste selbst aus (Senden mit
 * `↵`, `j`/`k` in der Inbox); solche Kürzel stehen im Overlay, lassen sich aber nicht umbelegen.
 */

/** Abschnitt im Shortcuts-Overlay. */
export type Section = 'general' | 'session' | 'approvals' | 'inbox'
/** Gruppe in der Palette. */
export type PaletteGroup = 'session' | 'general' | 'goto'

export interface RunContext {
  /** Auslösendes Tastenereignis, fehlt beim Aufruf aus der Palette. */
  event?: KeyboardEvent
}

export interface Command {
  id: string
  /** Titel als Text oder Übersetzungsschlüssel. */
  title: string | ((t: T) => string)
  /** Zusatz in der Palette, gedämpft (z. B. „2 offen“). */
  hint?: string | ((t: T) => string | undefined) | undefined
  icon?: ComponentType<{ className?: string }> | undefined
  /** Weitere Suchwörter für die Palette (beide Sprachen). */
  keywords?: string[] | undefined
  /** Standard-Kürzel, z. B. `Mod+K` (siehe `lib/keys`). */
  shortcut?: string | undefined
  section?: Section | undefined
  group?: PaletteGroup | undefined
  /** Reihenfolge innerhalb von Gruppe bzw. Abschnitt. */
  order?: number | undefined
  /** Kürzel greift auch im Eingabefeld. */
  globalInInput?: boolean | undefined
  /** Vor allen anderen Handlern auswerten (Capture-Phase), z. B. damit `⌘↵` nicht sendet. */
  capture?: boolean | undefined
  /** Kürzel wertet die Komponente selbst aus; fest belegt. */
  local?: boolean | undefined
  /** In der Palette zeigen (Standard: ja, sobald ausführbar). */
  palette?: boolean | undefined
  when?: (() => boolean) | undefined
  /** `false` heißt: nicht behandelt, das Tastenereignis läuft normal weiter. */
  run?: ((ctx: RunContext) => void | boolean) | undefined
}

const titleOf = (c: Command, t: T) => (typeof c.title === 'function' ? c.title(t) : c.title)
export const commandTitle = titleOf
export const commandHint = (c: Command, t: T) => (typeof c.hint === 'function' ? c.hint(t) : c.hint)

export { tk } from './i18n'

// --- Registry ----------------------------------------------------------------------------

interface Layer {
  token: number
  def: Partial<Command>
}

interface RegistryState {
  /** Ebenen je Befehl: Deklaration unten, Bindungen der Komponenten darüber. */
  layers: Record<string, Layer[]>
  /** Wird bei jeder Änderung erhöht (Palette und Overlay rendern neu). */
  version: number
}

export const useRegistry = create<RegistryState>()(() => ({ layers: {}, version: 0 }))

let nextToken = 0

/** Befehl registrieren bzw. eine bestehende Deklaration ergänzen; liefert die Abmeldung. */
export function registerCommand(def: Partial<Command> & { id: string }): () => void {
  ensureDispatcher()
  const token = (nextToken += 1)
  useRegistry.setState((s) => ({
    layers: { ...s.layers, [def.id]: [...(s.layers[def.id] ?? []), { token, def }] },
    version: s.version + 1,
  }))
  return () =>
    useRegistry.setState((s) => {
      const rest = (s.layers[def.id] ?? []).filter((l) => l.token !== token)
      const layers = { ...s.layers }
      if (rest.length) layers[def.id] = rest
      else delete layers[def.id]
      return { layers, version: s.version + 1 }
    })
}

/** Wirksamer Befehl aus allen Ebenen (obere gewinnen). */
function merged(layers: Layer[]): Command {
  return Object.assign({}, ...layers.map((l) => l.def)) as Command
}

/** Alle registrierten Befehle. */
export function commands(state: RegistryState = useRegistry.getState()): Command[] {
  return Object.values(state.layers).map(merged)
}

export function command(id: string): Command | undefined {
  const layers = useRegistry.getState().layers[id]
  return layers ? merged(layers) : undefined
}

/** Ausführbar: hat eine Ausführung, und die Bedingung ist erfüllt (UX-002 AC2). */
export function available(c: Command): boolean {
  if (!c.run) return false
  try {
    return c.when ? c.when() : true
  } catch {
    return false
  }
}

/** Befehl ausführen, wenn er verfügbar ist. */
export function runCommand(id: string, ctx: RunContext = {}): boolean {
  const c = command(id)
  if (!c || !available(c)) return false
  return c.run!(ctx) !== false
}

/**
 * Bindet einen Befehl an die Komponente: registriert beim Einhängen, meldet beim Aushängen ab.
 * `run` und `when` lesen immer den aktuellen Stand der Komponente.
 */
export function useCommand(def: Partial<Command> & { id: string }, enabled = true): void {
  const ref = useRef(def)
  ref.current = def
  const visible = enabled ? JSON.stringify([text(def.title), text(def.hint), def.keywords, def.shortcut, def.group, def.order, def.when ? safe(def.when) : null]) : ''
  useEffect(() => {
    if (!enabled) return
    const d = ref.current
    return registerCommand({
      ...d,
      ...(typeof d.title === 'function' ? { title: (t: T) => resolveText(ref.current.title, t) ?? '' } : {}),
      ...(typeof d.hint === 'function' ? { hint: (t: T) => resolveText(ref.current.hint, t) } : {}),
      ...(d.when ? { when: () => ref.current.when?.() ?? true } : {}),
      ...(d.run ? { run: (ctx: RunContext) => ref.current.run?.(ctx) } : {}),
    })
    // `visible` fasst alles zusammen, was Palette und Overlay zeigen.
  }, [def.id, enabled, visible])
}

function resolveText(v: string | ((t: T) => string | undefined) | undefined, tr: T): string | undefined {
  return typeof v === 'function' ? v(tr) : v
}

/** Angezeigter Text in der aktuellen Sprache (für den Vergleich, ob neu registriert werden muss). */
function text(v: string | ((t: T) => string | undefined) | undefined): string | undefined {
  return resolveText(v, currentT)
}

function safe(fn: () => boolean): boolean {
  try {
    return fn()
  } catch {
    return false
  }
}

/** Mehrere dynamische Befehle (z. B. „Weiter mit <Harness>“). */
export function useCommands(defs: (Partial<Command> & { id: string })[]): void {
  const ref = useRef(defs)
  ref.current = defs
  const key = JSON.stringify(defs.map((d) => [d.id, typeof d.title === 'string' ? d.title : '', typeof d.hint === 'string' ? d.hint : '', d.when ? safe(d.when) : null]))
  useEffect(() => {
    const offs = ref.current.map((d, i) =>
      registerCommand({
        ...d,
        ...(d.run ? { run: (ctx: RunContext) => ref.current[i]?.run?.(ctx) } : {}),
      }),
    )
    return () => offs.forEach((off) => off())
  }, [key])
}

// --- Belegung (Umbelegen mit Konfliktprüfung, UX-004 AC2) ----------------------------------

const KEYMAP_STORAGE = 'beton.shortcuts.v1'

function readOverrides(): Record<string, string> {
  try {
    const raw = JSON.parse(localStorage.getItem(KEYMAP_STORAGE) ?? '{}') as unknown
    if (raw && typeof raw === 'object') {
      return Object.fromEntries(Object.entries(raw as Record<string, unknown>).filter((e): e is [string, string] => typeof e[1] === 'string'))
    }
  } catch {
    // Kaputter oder fehlender Speicher: Standardbelegung.
  }
  return {}
}

interface KeymapState {
  overrides: Record<string, string>
  set: (overrides: Record<string, string>) => void
}

export const useKeymap = create<KeymapState>()((set) => ({
  overrides: readOverrides(),
  set: (overrides) => {
    try {
      localStorage.setItem(KEYMAP_STORAGE, JSON.stringify(overrides))
    } catch {
      // Gilt dann bis zum Neuladen.
    }
    set({ overrides })
  },
}))

/** Wirksames Kürzel eines Befehls (eigene Belegung vor Standard). */
export function shortcutOf(c: Command, overrides: Record<string, string> = useKeymap.getState().overrides): string | undefined {
  if (c.local) return c.shortcut
  const own = overrides[c.id]
  return own !== undefined ? own : c.shortcut
}

export type RebindResult = { ok: true } | { ok: false; reason: 'not_assignable' } | { ok: false; reason: 'conflict'; with: Command }

/** Wer belegt dieses Kürzel schon? */
export function conflictOf(id: string, chord: string): Command | undefined {
  const n = normalize(chord)
  return commands().find((c) => c.id !== id && c.shortcut !== undefined && shortcutOf(c)?.split(' / ').some((alt) => normalize(alt) === n))
}

/**
 * Kürzel umbelegen. Kollidiert es mit einem bestehenden, wird abgelehnt und der Konflikt
 * gemeldet; tauschen geht dann mit `swapShortcut`.
 */
export function rebind(id: string, chord: string): RebindResult {
  const c = command(id)
  if (!c || c.local || !assignable(chord)) return { ok: false, reason: 'not_assignable' }
  const other = conflictOf(id, chord)
  if (other) return { ok: false, reason: 'conflict', with: other }
  setOverride(id, normalize(chord))
  return { ok: true }
}

/** Kürzel übernehmen und dem bisherigen Besitzer das alte Kürzel geben (nach Bestätigung). */
export function swapShortcut(id: string, chord: string): boolean {
  const c = command(id)
  const other = conflictOf(id, chord)
  if (!c || c.local || !other || other.local) return false
  const previous = shortcutOf(c)
  const next = { ...useKeymap.getState().overrides, [id]: normalize(chord) }
  if (previous !== undefined) next[other.id] = previous
  else delete next[other.id]
  useKeymap.getState().set(clean(next))
  return true
}

function setOverride(id: string, chord: string) {
  useKeymap.getState().set(clean({ ...useKeymap.getState().overrides, [id]: chord }))
}

/** Belegungen, die dem Standard entsprechen, nicht speichern. */
function clean(overrides: Record<string, string>): Record<string, string> {
  return Object.fromEntries(Object.entries(overrides).filter(([id, chord]) => command(id)?.shortcut !== chord))
}

/** Alle eigenen Belegungen verwerfen. */
export function resetShortcuts(): void {
  useKeymap.getState().set({})
}

// --- Auswertung der Tasten ------------------------------------------------------------------

/** Zeit zwischen den Anschlägen einer Folge wie `Esc Esc` (WEB-005). */
export const SEQUENCE_MS = 600

let installed = false
let pending: { stroke: string; at: number } | undefined
let suspended = 0

/** Kürzel aussetzen, z. B. während das Overlay eine neue Belegung aufnimmt; liefert das Fortsetzen. */
export function suspendShortcuts(): () => void {
  suspended += 1
  let done = false
  return () => {
    if (done) return
    done = true
    suspended -= 1
  }
}

/** Für Tests: Zustand der Tastenfolgen zurücksetzen. */
export function resetSequence(): void {
  pending = undefined
}

function handle(e: KeyboardEvent, capture: boolean) {
  if (suspended > 0 || e.isComposing || isModifierKey(e.key)) return
  if (!capture && e.defaultPrevented) {
    pending = undefined
    return
  }
  const stroke = strokeOf(e)
  const now = Date.now()
  const prefix = pending && now - pending.at < SEQUENCE_MS ? pending.stroke : undefined
  const typing = isTyping(e.target)
  const overrides = useKeymap.getState().overrides
  let sequenceStart = false
  for (const c of commands()) {
    if (c.local || !!c.capture !== capture) continue
    const chord = shortcutOf(c, overrides)
    if (!chord) continue
    for (const alt of chord.split(' / ')) {
      const strokes = normalize(alt).split(' ')
      const hit = strokes.length === 1 ? strokes[0] === stroke : strokes.length === 2 && prefix === strokes[0] && strokes[1] === stroke
      if (strokes.length === 2 && strokes[0] === stroke) sequenceStart = true
      if (!hit) continue
      if (typing && !c.globalInInput) continue
      if (!available(c)) continue
      const handled = c.run!({ event: e }) !== false
      if (handled) {
        e.preventDefault()
        if (capture) e.stopPropagation()
        pending = undefined
        return
      }
    }
  }
  if (!capture) pending = sequenceStart ? { stroke, at: now } : undefined
}

/** Tastenauswertung einmal am Fenster anmelden (Capture- und Bubble-Phase). */
export function ensureDispatcher(): void {
  if (installed || typeof window === 'undefined') return
  installed = true
  declare()
  window.addEventListener('keydown', (e) => handle(e, true), true)
  window.addEventListener('keydown', (e) => handle(e, false))
}

let declared = false

/** Deklarierte Befehle (`lib/keymap`) einmal eintragen; danach binden Komponenten sie. */
function declare(): void {
  if (declared) return
  declared = true
  for (const d of DECLARED) registerCommand(d)
}
