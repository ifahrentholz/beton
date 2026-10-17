import type { GrammarState, HighlighterGeneric } from 'shiki'
import { shikiTheme, useTheme } from './theme'

/** Zeilen aus Tokens `[Text, Farbe]`. Nur Farben werden übernommen, Text bleibt Text. */
export type Lines = [string, string][][]

type Highlighter = HighlighterGeneric<any, any>
let highlighter: Promise<Highlighter> | undefined

/**
 * Shiki mit der JavaScript-Regex-Engine: Die CSP der Web-UI erlaubt kein WebAssembly
 * (`script-src 'self'` ohne `wasm-unsafe-eval`). Shiki und Sprachen werden erst bei Bedarf
 * aus dem eigenen Bundle geladen (WEB-001, ADR-0033).
 */
function load(): Promise<Highlighter> {
  highlighter ??= (async () => {
    const [{ createHighlighter }, { createJavaScriptRegexEngine }] = await Promise.all([import('shiki'), import('shiki/engine/javascript')])
    return createHighlighter({ themes: [], langs: [], engine: createJavaScriptRegexEngine() })
  })()
  return highlighter
}

/** Shiki-Theme passend zum UI-Theme (UX-005); Komponenten hören auf `useCodeTheme`. */
export function currentCodeTheme(): string {
  const { family, mode } = useTheme.getState()
  return shikiTheme(family, mode)
}

/**
 * Aktuelles Shiki-Theme als Hook: Ein Theme-Wechsel färbt Code-Blöcke ohne Neuladen neu. Der
 * Wert enthält Familie und Modus, weil die Flächen (und damit die lesbaren Farben) auch dann
 * wechseln, wenn das Shiki-Theme gleich bleibt.
 */
export function useCodeTheme(): string {
  return useTheme((s) => `${shikiTheme(s.family, s.mode)}@${s.family}-${s.mode}`)
}

/** Shiki-Theme aus dem Wert von `useCodeTheme`. */
export const codeThemeName = (key: string) => key.split('@')[0]!

async function ready(lang: string, theme: string): Promise<Highlighter> {
  const h = await load()
  const loads: Promise<unknown>[] = []
  if (!h.getLoadedLanguages().includes(lang)) loads.push(h.loadLanguage(lang as never))
  if (!h.getLoadedThemes().includes(theme)) loads.push(h.loadTheme(theme as never))
  await Promise.all(loads)
  return h
}

/** Ganzen Code auf einmal hervorheben (Codeblöcke im Chat). */
export async function highlight(code: string, lang: string, theme: string = currentCodeTheme()): Promise<Lines> {
  const h = await ready(lang, theme)
  const { tokens } = h.codeToTokens(code, { lang: lang as never, theme })
  return tokens.map((line) => line.map((t): [string, string] => [t.content, t.color ?? '']))
}

/** Zeitbudget je Abschnitt (ms): kurz genug, dass Scrollen dazwischen flüssig bleibt. */
const SLICE_MS = 4

/**
 * Viele Zeilen in Abschnitten hervorheben, ohne den Haupt-Thread lange zu blockieren
 * (Diffs mit tausenden Zeilen, WEB-011 AC1). Die Abschnittsgröße passt sich an, sodass jeder
 * Abschnitt etwa {@link SLICE_MS} dauert. Der Grammatik-Zustand wird weitergereicht,
 * mehrzeilige Kommentare und Strings bleiben korrekt.
 */
export async function highlightChunked(
  lines: readonly string[],
  lang: string,
  onChunk: (from: number, chunk: Lines) => void,
  signal?: AbortSignal,
  theme: string = currentCodeTheme(),
): Promise<void> {
  const h = await ready(lang, theme)
  let state: GrammarState | undefined
  let size = 20
  for (let from = 0; from < lines.length; ) {
    if (signal?.aborted) return
    const t0 = performance.now()
    const code = lines.slice(from, from + size).join('\n')
    const res = h.codeToTokens(code, { lang: lang as never, theme, tokenizeMaxLineLength: 2000, ...(state ? { grammarState: state } : {}) })
    state = res.grammarState
    onChunk(
      from,
      res.tokens.map((line) => line.map((t): [string, string] => [t.content, t.color ?? ''])),
    )
    from += size
    const took = Math.max(performance.now() - t0, 0.25)
    size = Math.max(5, Math.min(400, Math.round((size * SLICE_MS) / took)))
    // Dem Browser zwischendurch Zeit für Scrollen und Rendern lassen.
    await new Promise((r) => setTimeout(r, 0))
  }
}

// --- Lesbarkeit der Token-Farben (WEB-016) ---------------------------------------------------

function rgb(hex: string): [number, number, number] | undefined {
  const m = /^#([0-9a-f]{6})([0-9a-f]{2})?$/i.exec(hex.trim())
  if (!m) return undefined
  const n = parseInt(m[1]!, 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}

function luminance([r, g, b]: [number, number, number]): number {
  const f = (c: number) => {
    const s = c / 255
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

export function contrast(a: string, b: string): number {
  const x = rgb(a)
  const y = rgb(b)
  if (!x || !y) return 21
  const [hi, lo] = [luminance(x), luminance(y)].sort((p, q) => q - p)
  return (hi! + 0.05) / (lo! + 0.05)
}

const hex = (c: [number, number, number]) => `#${c.map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')}`
const readableCache = new Map<string, string>()

/**
 * Token-Farbe so weit Richtung Schwarz bzw. Weiß verschieben, dass sie auf allen Flächen
 * mindestens `min` Kontrast hat (WCAG AA für Text). Shiki-Themes sind für ihre eigene Fläche
 * gemacht; die Code-Blöcke und Diff-Zeilen liegen auf den Flächen des UI-Themes.
 */
export function readable(color: string, backgrounds: readonly string[], min = 4.5): string {
  const key = `${color}|${backgrounds.join(',')}|${min}`
  const hit = readableCache.get(key)
  if (hit) return hit
  const fg = rgb(color)
  const bgs = backgrounds.map(rgb).filter((b): b is [number, number, number] => !!b)
  let out = color
  if (fg && bgs.length) {
    const ok = (c: string) => backgrounds.every((b) => contrast(c, b) >= min)
    if (!ok(color)) {
      const light = bgs.reduce((s, b) => s + luminance(b), 0) / bgs.length > 0.18
      const target: [number, number, number] = light ? [0, 0, 0] : [255, 255, 255]
      for (let t = 0.05; t <= 1.0001; t += 0.05) {
        const mixed = hex([0, 1, 2].map((i) => fg[i]! + (target[i]! - fg[i]!) * t) as [number, number, number])
        out = mixed
        if (ok(mixed)) break
      }
    }
  }
  readableCache.set(key, out)
  return out
}

/** Farbe eines Design-Tokens am Wurzelelement (z. B. `--sunken`). */
export function surface(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

/** Zeilen mit lesbaren Farben auf den angegebenen Flächen. */
export function readableLines(lines: Lines, backgrounds: readonly string[]): Lines {
  return lines.map((line) => line.map(([text, color]): [string, string] => [text, color ? readable(color, backgrounds) : color]))
}
