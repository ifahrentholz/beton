import { memo, useEffect, useId, useState } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { codeThemeName, highlight, readableLines, surface, useCodeTheme, type Lines } from '@/lib/highlight'
import { useTheme } from '@/lib/theme'

/**
 * Markdown im Chat-Stream (WEB-002). Kein Roh-HTML (react-markdown rendert nur Elemente),
 * Code-Hervorhebung mit Shiki und Diagramme mit Mermaid werden erst bei Bedarf geladen und
 * erst nach Abschluss der Nachricht angewandt: Während des Streamings bleibt ein Codeblock
 * schlichtes `<pre>` mit derselben Metrik, so springt das Layout nicht (WEB-002 AC1).
 */
export const Markdown = memo(function Markdown({ text, streaming }: { text: string; streaming?: boolean }) {
  // Inkrementell (WEB-015): fertige Blöcke bleiben gemerkt, nur der letzte wird neu geparst.
  const blocks = splitBlocks(text)
  return (
    <div className="md">
      {blocks.map((b, i) => (
        <MarkdownBlock key={i} text={b} streaming={!!streaming && i === blocks.length - 1} />
      ))}
    </div>
  )
})

/**
 * Teilt Markdown an Leerzeilen in Blöcke, aber nie innerhalb eines Code-Fence. Jeder Block
 * ist für sich gültiges Markdown (Absätze, Listen, Codeblöcke).
 */
export function splitBlocks(text: string): string[] {
  const out: string[] = []
  let current: string[] = []
  let fence: string | undefined
  for (const line of text.split('\n')) {
    const m = /^\s{0,3}(`{3,}|~{3,})/.exec(line)
    if (m) {
      if (!fence) fence = m[1]![0]!.repeat(m[1]!.length)
      else if (line.trim().startsWith(fence)) fence = undefined
    }
    if (!fence && line.trim() === '' && current.length > 0 && !/^\s*([-*+]|\d+\.)\s/.test(current.at(-1) ?? '')) {
      out.push(current.join('\n'))
      current = []
      continue
    }
    current.push(line)
  }
  if (current.length) out.push(current.join('\n'))
  return out.length ? out : ['']
}

const MarkdownBlock = memo(function MarkdownBlock({ text, streaming }: { text: string; streaming: boolean }) {
  const components: Components = {
    code({ className, children }) {
      const lang = /language-([\w-]+)/.exec(className ?? '')?.[1]
      const code = String(children ?? '').replace(/\n$/, '')
      if (!lang && !code.includes('\n')) return <code>{children}</code>
      if (streaming) return <code>{code}</code>
      if (lang === 'mermaid') return <Mermaid code={code} />
      return <Highlighted code={code} lang={lang} />
    },
    pre({ children }) {
      return <pre>{children}</pre>
    },
    a({ href, children }) {
      return (
        <a href={href} target="_blank" rel="noreferrer noopener">
          {children}
        </a>
      )
    },
  }
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {text}
    </ReactMarkdown>
  )
})

function Highlighted({ code, lang }: { code: string; lang?: string | undefined }) {
  const [lines, setLines] = useState<Lines | undefined>()
  // Theme-Wechsel färbt neu, ohne Neuladen (UX-005 AC1).
  const theme = useCodeTheme()
  useEffect(() => {
    if (!lang) return
    let alive = true
    highlight(code, lang, codeThemeName(theme))
      // Lesbar auf der Fläche der Code-Blöcke (`--sunken`, WEB-016).
      .then((l) => alive && setLines(readableLines(l, [surface('--sunken')])))
      .catch(() => undefined)
    return () => {
      alive = false
    }
  }, [code, lang, theme])
  if (!lines) return <code>{code}</code>
  return (
    <code>
      {lines.map((line, i) => (
        <span key={i}>
          {line.map(([text, color], j) => (
            <span key={j} style={color ? { color } : undefined}>
              {text}
            </span>
          ))}
          {i < lines.length - 1 ? '\n' : null}
        </span>
      ))}
    </code>
  )
}

function Mermaid({ code }: { code: string }) {
  const id = useId().replace(/:/g, '')
  const [svg, setSvg] = useState<string | undefined>()
  const [failed, setFailed] = useState(false)
  const dark = useTheme((s) => s.mode === 'dark')
  useEffect(() => {
    let alive = true
    import('mermaid')
      .then(async ({ default: mermaid }) => {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          theme: dark ? 'dark' : 'neutral',
        })
        const { svg } = await mermaid.render(`m${id}`, code)
        if (alive) setSvg(svg)
      })
      .catch(() => alive && setFailed(true))
    return () => {
      alive = false
    }
  }, [code, id, dark])
  if (failed || !svg) return <code>{code}</code>
  // Mermaid erzeugt das SVG mit `securityLevel: strict` (kein HTML, keine Skripte).
  return <span className="block overflow-x-auto" dangerouslySetInnerHTML={{ __html: svg }} />
}
