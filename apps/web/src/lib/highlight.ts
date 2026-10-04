import type { GrammarState, HighlighterGeneric } from 'shiki'

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
    return createHighlighter({ themes: ['github-light', 'github-dark'], langs: [], engine: createJavaScriptRegexEngine() })
  })()
  return highlighter
}

async function ready(lang: string): Promise<Highlighter> {
  const h = await load()
  if (!h.getLoadedLanguages().includes(lang)) await h.loadLanguage(lang as never)
  return h
}

const isDark = () => document.documentElement.classList.contains('dark')

/** Ganzen Code auf einmal hervorheben (Codeblöcke im Chat). */
export async function highlight(code: string, lang: string): Promise<Lines> {
  const h = await ready(lang)
  const { tokens } = h.codeToTokens(code, { lang: lang as never, theme: isDark() ? 'github-dark' : 'github-light' })
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
export async function highlightChunked(lines: readonly string[], lang: string, onChunk: (from: number, chunk: Lines) => void, signal?: AbortSignal): Promise<void> {
  const h = await ready(lang)
  const theme = isDark() ? 'github-dark' : 'github-light'
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
