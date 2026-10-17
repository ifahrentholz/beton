/**
 * xterm.js mit Fit-, WebGL- und Weblinks-Addon (WEB-010). Nur über `import()` geladen, damit
 * das initiale Bundle schlank bleibt. Ohne WebGL (z. B. in Tests oder alten GPUs) zeichnet der
 * DOM-Renderer.
 */
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'

export interface XTerm {
  term: Terminal
  /** An die Größe des Containers anpassen. */
  fit: () => void
  dispose: () => void
}

/** Farbe aus den Design-Tokens am Container (das Terminal sitzt in `.dark`, wie im Prototyp). */
function token(el: HTMLElement, name: string, fallback: string): string {
  const v = getComputedStyle(el).getPropertyValue(name).trim()
  return v || fallback
}

export function createTerminal(container: HTMLElement, readOnly: boolean): XTerm {
  const term = new Terminal({
    fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
    fontSize: 12,
    lineHeight: 1.2,
    scrollback: 10_000,
    cursorBlink: !readOnly,
    disableStdin: readOnly,
    allowProposedApi: false,
    theme: {
      background: token(container, '--sunken', '#161819'),
      foreground: token(container, '--foreground', '#dcddd8'),
      cursor: token(container, '--foreground', '#dcddd8'),
    },
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
  try {
    const gl = new WebglAddon()
    gl.onContextLoss(() => gl.dispose())
    term.loadAddon(gl)
  } catch {
    // DOM-Renderer
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
