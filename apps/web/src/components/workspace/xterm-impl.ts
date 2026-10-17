/**
 * xterm.js mit Fit-, WebGL- und Weblinks-Addon (WEB-010). Nur über `import()` geladen, damit
 * das initiale Bundle schlank bleibt. Ohne WebGL (z. B. in Tests oder alten GPUs) zeichnet der
 * DOM-Renderer.
 */
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { Terminal, type ITheme } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'

export interface XTerm {
  term: Terminal
  /** An die Größe des Containers anpassen. */
  fit: () => void
  dispose: () => void
}

export interface TerminalOptions {
  /** Ohne Eingabe und ohne blinkenden Cursor (Nur-Lese-Rolle, beendetes Terminal). */
  readOnly: boolean
  /** Farben (aus `terminalTheme`, UX-005); später über `term.options.theme` änderbar. */
  theme: ITheme
  /** Feste Größe, z. B. die des Snapshots; sonst passt Fit sie an. */
  cols?: number
  rows?: number
  /**
   * WebGL-Renderer (Default). Ohne zeichnet der DOM-Renderer: Für ruhende Bildschirme wie
   * Snapshots reicht er, und der Text bleibt im DOM lesbar.
   */
  webgl?: boolean
}

export function createTerminal(container: HTMLElement, opts: TerminalOptions): XTerm {
  const term = new Terminal({
    fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
    fontSize: 12,
    lineHeight: 1.2,
    scrollback: 10_000,
    cursorBlink: !opts.readOnly,
    cursorInactiveStyle: opts.readOnly ? 'none' : 'outline',
    disableStdin: opts.readOnly,
    allowProposedApi: false,
    theme: opts.theme,
    ...(opts.cols && opts.rows ? { cols: opts.cols, rows: opts.rows } : {}),
  })
  const fit = new FitAddon()
  term.loadAddon(fit)
  // Links öffnen nur mit Klick, in neuem Tab ohne Opener.
  term.loadAddon(
    new WebLinksAddon((_e, uri) => {
      window.open(uri, '_blank', 'noopener,noreferrer')
    }),
  )
  term.open(container)
  if (opts.webgl !== false) {
    try {
      const gl = new WebglAddon()
      gl.onContextLoss(() => gl.dispose())
      term.loadAddon(gl)
    } catch {
      // DOM-Renderer
    }
  }
  return {
    term,
    fit: () => {
      try {
        fit.fit()
      } catch {
        // Container noch ohne Größe
      }
    },
    dispose: () => term.dispose(),
  }
}
