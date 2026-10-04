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

/**
 * Viele Zeilen in Abschnitten hervorheben, ohne den Haupt-Thread lange zu blockieren
 * (Diffs mit tausenden Zeilen, WEB-011 AC1). Der Grammatik-Zustand wird von Abschnitt zu
 * Abschnitt weitergereicht, mehrzeilige Kommentare und Strings bleiben korrekt.
 */
export async function highlightChunked(lines: readonly string[], lang: string, onChunk: (from: number, chunk: Lines) => void, signal?: AbortSignal, chunkSize = 300): Promise<void> {
  const h = await ready(lang)
  const theme = isDark() ? 'github-dark' : 'github-light'
  let state: GrammarState | undefined
  for (let from = 0; from < lines.length; from += chunkSize) {
    if (signal?.aborted) return
    const code = lines.slice(from, from + chunkSize).join('\n')
    const res = h.codeToTokens(code, { lang: lang as never, theme, tokenizeMaxLineLength: 2000, ...(state ? { grammarState: state } : {}) })
    state = res.grammarState
    onChunk(
      from,
      res.tokens.map((line) => line.map((t): [string, string] => [t.content, t.color ?? ''])),
    )
    // Dem Browser zwischendurch Zeit für Scrollen und Rendern lassen.
    await new Promise((r) => setTimeout(r, 0))
  }
}
