import { useEffect, useRef } from 'react'
import { Announcer, type Spoken } from '@/lib/announce'
import { useTheme } from '@/lib/theme'
import type { Item } from '@/lib/timeline'

/**
 * ARIA-Live-Region für das Streaming (WEB-016): sagt abgeschlossene Absätze der Agent-Antworten
 * an, gedrosselt; abschaltbar unter Darstellung → „Streaming für Screenreader ansagen“.
 *
 * Die Region liegt in einem geschlossenen Shadow-Root: Screenreader lesen sie (sie ist Teil des
 * Accessibility-Baums), Textsuche und DOM-Abfragen finden den Text aber nicht ein zweites Mal
 * neben der sichtbaren Nachricht.
 */
export function StreamAnnouncer({ items }: { items: readonly Item[] }) {
  const enabled = useTheme((s) => s.announce)
  const host = useRef<HTMLDivElement>(null)
  const region = useRef<HTMLDivElement | undefined>(undefined)
  const announcer = useRef<Announcer | undefined>(undefined)
  useEffect(() => {
    const el = host.current
    if (!el || region.current) return
    const root = el.attachShadow({ mode: 'closed' })
    const live = document.createElement('div')
    live.setAttribute('aria-live', 'polite')
    live.setAttribute('aria-atomic', 'true')
    live.style.cssText = 'position:absolute;width:1px;height:1px;margin:-1px;padding:0;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0'
    root.append(live)
    region.current = live
  }, [])
  useEffect(() => {
    if (!enabled) return
    const a = new Announcer((text) => {
      if (region.current) region.current.textContent = text
    })
    announcer.current = a
    return () => {
      a.dispose()
      announcer.current = undefined
      if (region.current) region.current.textContent = ''
    }
  }, [enabled])
  useEffect(() => {
    const spoken: Spoken[] = []
    for (const it of items) if (it.kind === 'assistant') spoken.push({ key: it.messageId ?? it.key, text: it.text, streaming: it.streaming })
    announcer.current?.update(spoken)
  }, [items, enabled])
  return <div ref={host} data-testid="stream-announcer" />
}
