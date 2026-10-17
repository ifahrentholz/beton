/**
 * Ansagen des Streamings für Screenreader (WEB-016): Nur abgeschlossene Absätze werden
 * vorgelesen, höchstens alle {@link ANNOUNCE_MS} ms, damit die Live-Region nicht bei jedem Delta
 * spricht. Beim Öffnen einer Session wird der bisherige Verlauf nicht angesagt.
 */

export const ANNOUNCE_MS = 2000

/** Nachricht des Agents, wie die Zeitleiste sie liefert. */
export interface Spoken {
  key: string
  text: string
  streaming?: boolean | undefined
}

/** Abgeschlossene Absätze: beim Streamen alle bis auf den letzten, danach alle. */
export function finishedParagraphs(text: string, streaming: boolean | undefined): string[] {
  const parts = text
    .split(/\n\s*\n/)
    .map((p) => plain(p))
    .filter(Boolean)
  if (streaming) {
    // Der letzte Absatz gilt erst als fertig, wenn danach eine Leerzeile kam.
    const closed = /\n\s*\n\s*$/.test(text)
    return closed ? parts : parts.slice(0, -1)
  }
  return parts
}

/** Markdown-Zeichen entfernen, die ein Screenreader sonst vorliest. */
function plain(p: string): string {
  return p
    .replace(/```[\s\S]*?```/g, '')
    .replace(/[`*_#>]+/g, '')
    .replace(/\s+/g, ' ')
    .trim()
}

export class Announcer {
  private seen = new Map<string, number>()
  private queue: string[] = []
  private last = -Infinity
  private timer: ReturnType<typeof setTimeout> | undefined
  private seeded = false

  constructor(
    private readonly say: (text: string) => void,
    private readonly interval = ANNOUNCE_MS,
    private readonly now: () => number = () => Date.now(),
  ) {}

  /** Neuer Stand der Nachrichten; neue fertige Absätze werden gesammelt und gedrosselt angesagt. */
  update(messages: readonly Spoken[]): void {
    for (const m of messages) {
      const paras = finishedParagraphs(m.text, m.streaming)
      const done = this.seen.get(m.key)
      if (!this.seeded) {
        this.seen.set(m.key, paras.length)
        continue
      }
      const from = done ?? 0
      if (paras.length > from) {
        this.queue.push(...paras.slice(from))
        this.seen.set(m.key, paras.length)
      }
    }
    this.seeded = true
    this.schedule()
  }

  dispose(): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = undefined
  }

  private schedule(): void {
    if (this.queue.length === 0 || this.timer) return
    const wait = this.last + this.interval - this.now()
    if (wait <= 0) this.flush()
    else this.timer = setTimeout(() => {
      this.timer = undefined
      this.flush()
    }, wait)
  }

  private flush(): void {
    if (this.queue.length === 0) return
    const text = this.queue.join(' ')
    this.queue = []
    this.last = this.now()
    this.say(text)
  }
}
