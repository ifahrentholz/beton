import { describe, expect, it, vi } from 'vitest'
import { Announcer, finishedParagraphs } from '@/lib/announce'

describe('Live-Region für das Streaming (WEB-016)', () => {
  it('WEB-016: nur abgeschlossene Absätze, höchstens alle 2 s, ohne den bisherigen Verlauf', () => {
    expect(finishedParagraphs('Erster Absatz.\n\nZweiter, noch im Fluss', true)).toEqual(['Erster Absatz.'])
    expect(finishedParagraphs('Erster Absatz.\n\nZweiter fertig.\n\n', true)).toEqual(['Erster Absatz.', 'Zweiter fertig.'])
    expect(finishedParagraphs('**Fett** und `code`', false)).toEqual(['Fett und code'])

    vi.useFakeTimers()
    try {
      let now = 0
      const said: string[] = []
      const a = new Announcer((t) => said.push(t), 2000, () => now)
      // Beim Öffnen: alter Verlauf wird nicht angesagt.
      a.update([{ key: 'm0', text: 'Alt.\n\nAuch alt.' }])
      expect(said).toEqual([])
      // Streaming: der unfertige Absatz bleibt still.
      a.update([{ key: 'm1', text: 'Eins.\n\nZw', streaming: true }])
      expect(said).toEqual(['Eins.'])
      // Weitere Absätze binnen 2 s werden gesammelt.
      now = 500
      a.update([{ key: 'm1', text: 'Eins.\n\nZwei.\n\nDrei.\n\nVi', streaming: true }])
      expect(said).toEqual(['Eins.'])
      now = 2000
      vi.advanceTimersByTime(1500)
      expect(said).toEqual(['Eins.', 'Zwei. Drei.'])
      // Fertige Nachricht mit derselben ID: nur der Rest.
      now = 5000
      a.update([{ key: 'm1', text: 'Eins.\n\nZwei.\n\nDrei.\n\nVier.' }])
      expect(said).toEqual(['Eins.', 'Zwei. Drei.', 'Vier.'])
      a.dispose()
    } finally {
      vi.useRealTimers()
    }
  })
})
