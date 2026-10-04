/**
 * Hilfen für den Composer (WEB-006): Anhänge prüfen und beschriften, `@`-Erwähnungen und
 * Slash-Befehle erkennen und einfügen. Rein und ohne DOM, damit sie testbar bleiben.
 */

/** Grenzen laut Server (`GET /v1/info`, `attachments.*`). */
export interface AttachmentLimits {
  max_file_bytes: number
  max_files: number
}

/** Defaults des Servers, solange `/v1/info` noch nicht geladen ist. */
export const DEFAULT_LIMITS: AttachmentLimits = { max_file_bytes: 20 * 1024 * 1024, max_files: 10 }

export type AttachmentKind = 'image' | 'pdf' | 'text'

const IMAGE = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']
const TEXT_APP = ['application/json', 'application/xml', 'application/yaml', 'application/x-yaml', 'application/toml', 'application/x-sh']
const TEXT_EXT = /\.(txt|md|markdown|csv|tsv|log|json|ya?ml|toml|xml|html?|css|js|jsx|ts|tsx|rs|py|go|rb|java|kt|swift|c|h|cpp|hpp|cs|php|sh|sql|ini|env|diff|patch)$/i

/** Medientyp einer Datei; Browser liefern für Quelltext oft keinen. */
export function mimeOf(name: string, type: string): string {
  const t = type.split(';')[0]?.trim().toLowerCase() ?? ''
  if (t) return t
  if (/\.png$/i.test(name)) return 'image/png'
  if (/\.jpe?g$/i.test(name)) return 'image/jpeg'
  if (/\.pdf$/i.test(name)) return 'application/pdf'
  if (TEXT_EXT.test(name)) return 'text/plain'
  return 'application/octet-stream'
}

/** Erlaubte Arten (Bilder, PDF, Text), sonst `undefined`. */
export function kindOf(mime: string): AttachmentKind | undefined {
  const m = mime.toLowerCase()
  if (IMAGE.includes(m)) return 'image'
  if (m === 'application/pdf') return 'pdf'
  if (m.startsWith('text/') || TEXT_APP.includes(m)) return 'text'
  return undefined
}

/** `accept` für den Dateidialog. */
export const ACCEPT = [...IMAGE, 'application/pdf', 'text/*', ...TEXT_APP, '.md', '.rs', '.ts', '.tsx', '.py', '.go', '.yaml', '.yml', '.toml', '.log'].join(',')

const de = new Intl.NumberFormat('de-DE', { maximumFractionDigits: 1 })

/** Größe wie im Prototyp: „412 KB“, „2,1 MB“. */
export function size(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${de.format(bytes / (1024 * 1024))} MB`
  return `${Math.max(1, Math.round(bytes / 1024))} KB`
}

/** Ganze MB für Grenzen („20 MB“). */
function wholeMb(bytes: number): string {
  return `${Math.ceil(bytes / (1024 * 1024))} MB`
}

/** Beschriftung unter einem Anhang. */
export function caption(kind: AttachmentKind, name: string, bytes: number): string {
  if (kind === 'image') return `${/^(image|bildschirmfoto|screenshot)\b/i.test(name) ? 'Bildschirmfoto' : name} · ${size(bytes)}`
  return `${kind === 'pdf' ? 'PDF' : 'Text'} · ${size(bytes)}`
}

/** Warum eine Datei nicht angehängt wird, sonst `undefined`. */
export function rejection(file: { name: string; size: number; type: string }, count: number, limits: AttachmentLimits): string | undefined {
  if (count >= limits.max_files) return `höchstens ${limits.max_files} Dateien pro Nachricht.`
  if (!kindOf(mimeOf(file.name, file.type))) return 'Erlaubt sind Bilder (PNG, JPEG, GIF, WebP), PDF und Textdateien.'
  if (file.size > limits.max_file_bytes) {
    return `${wholeMb(file.size)}, erlaubt sind ${wholeMb(limits.max_file_bytes)} pro Datei. Kürze die Datei oder hänge einen Ausschnitt an.`
  }
  if (file.size === 0) return 'Die Datei ist leer.'
  return undefined
}

/** Name für eingefügte Screenshots ohne Dateinamen. */
export function pastedName(file: { name: string; type: string }, now = new Date()): string {
  if (file.name && file.name !== 'image.png') return file.name
  const pad = (n: number) => String(n).padStart(2, '0')
  const ext = file.type.split('/')[1] ?? 'png'
  return `Bildschirmfoto ${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}.${pad(now.getMinutes())}.${pad(now.getSeconds())}.${ext}`
}

/** Offene `@`-Erwähnung vor dem Cursor: Anfrage und Startposition des `@`. */
export function mentionAt(text: string, caret: number): { query: string; start: number } | undefined {
  const before = text.slice(0, caret)
  const m = /(^|\s)@([^\s@]*)$/.exec(before)
  if (!m) return undefined
  return { query: m[2] ?? '', start: before.length - (m[2]?.length ?? 0) - 1 }
}

/** Ersetzt die offene Erwähnung durch `@<pfad> `; liefert Text und neue Cursorposition. */
export function insertMention(text: string, caret: number, path: string): { text: string; caret: number } {
  const m = mentionAt(text, caret)
  if (!m) return { text, caret }
  const ref = `@${path} `
  const next = text.slice(0, m.start) + ref + text.slice(caret).replace(/^\s+/, '')
  return { text: next, caret: m.start + ref.length }
}

/** Slash-Befehl in Arbeit: der ganze Entwurf ist `/<wort>`. */
export function slashQuery(text: string): string | undefined {
  const m = /^\/(\S*)$/.exec(text)
  return m ? (m[1] ?? '') : undefined
}

/** Teilt einen Pfad in Treffer- und Nicht-Treffer-Stücke (Hervorhebung im Menü). */
export function highlight(path: string, query: string): { text: string; hit: boolean }[] {
  const q = query.trim()
  if (!q) return [{ text: path, hit: false }]
  const sensitive = /[A-Z]/.test(q)
  const hay = sensitive ? path : path.toLowerCase()
  const needle = sensitive ? q : q.toLowerCase()
  // Zusammenhängender Treffer, bevorzugt im Dateinamen.
  const base = hay.lastIndexOf('/') + 1
  let at = hay.indexOf(needle, base)
  if (at < 0) at = hay.indexOf(needle)
  if (at >= 0) {
    return [
      { text: path.slice(0, at), hit: false },
      { text: path.slice(at, at + needle.length), hit: true },
      { text: path.slice(at + needle.length), hit: false },
    ].filter((p) => p.text)
  }
  // Unscharf: einzelne Zeichen in Reihenfolge.
  const parts: { text: string; hit: boolean }[] = []
  let qi = 0
  for (let i = 0; i < path.length; i++) {
    const hit = qi < needle.length && hay[i] === needle[qi]
    if (hit) qi++
    const last = parts.at(-1)
    if (last && last.hit === hit) last.text += path[i]
    else parts.push({ text: path[i] ?? '', hit })
  }
  return parts
}
