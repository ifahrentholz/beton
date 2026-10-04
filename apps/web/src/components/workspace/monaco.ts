/**
 * Monaco aus dem eigenen Bundle (WEB-009, WEB-001): kein CDN-Loader, der Editor-Worker kommt
 * als eigene Datei vom selben Origin (CSP `worker-src 'self' blob:`). Nur der Kern mit den
 * Editor-Funktionen (Suche, Faltung …) und Monarch-Hervorhebung, ohne Sprachdienste (die
 * bräuchten weitere Worker). Dieses Modul wird nur über `import()` geladen und liegt damit
 * nicht im initialen Bundle (WEB-009 AC2).
 */
import * as monaco from 'monaco-editor/editor/editor.api.js'
import 'monaco-editor/features/register.all.js'
import 'monaco-editor/languages/definitions/register.all.js'
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker'

declare global {
  interface Window {
    MonacoEnvironment?: { getWorker: (workerId: string, label: string) => Worker }
  }
}

window.MonacoEnvironment = { getWorker: () => new EditorWorker() }

/** Farbe aus den Design-Tokens (`--card` …) als Hex für Monaco. */
function token(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return /^#[0-9a-f]{6}$/i.test(v) ? v : fallback
}

/** Monaco-Themes aus den Tokens von `apps/web` (hell/dunkel), Hervorhebung aus `vs`/`vs-dark`. */
export function applyTheme(): void {
  const dark = document.documentElement.classList.contains('dark')
  const bg = token('--card', dark ? '#26292d' : '#f0f0ec')
  const fg = token('--foreground', dark ? '#dcddd8' : '#24272b')
  const muted = token('--muted-foreground', dark ? '#9a9ea4' : '#585c62')
  const accent = token('--accent', dark ? '#30343a' : '#d5d5cf')
  const border = token('--border', dark ? '#363a40' : '#c4c4bd')
  const ok = token('--ok-soft', dark ? '#233a2b' : '#cfe3d4')
  const deny = token('--deny-soft', dark ? '#42231f' : '#f0d2ce')
  monaco.editor.defineTheme('beton', {
    base: dark ? 'vs-dark' : 'vs',
    inherit: true,
    rules: [],
    colors: {
      'editor.background': bg,
      'editor.foreground': fg,
      'editorLineNumber.foreground': muted,
      'editorLineNumber.activeForeground': fg,
      'editor.lineHighlightBackground': `${accent}80`,
      'editor.selectionBackground': `${accent}`,
      'editorWidget.background': token('--popover', bg),
      'editorWidget.border': border,
      'editorGutter.background': bg,
      'diffEditor.insertedLineBackground': ok,
      'diffEditor.removedLineBackground': deny,
    },
  })
  monaco.editor.setTheme('beton')
}

new MutationObserver(applyTheme).observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
applyTheme()

export { monaco }
