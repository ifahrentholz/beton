import { describe, expect, it } from 'vitest'
import { caption, highlight, insertMention, kindOf, mentionAt, mimeOf, pastedName, rejection, size, slashQuery } from '@/lib/composer'
import { attachmentsOf, timeline } from '@/lib/timeline'
import { ev } from './fixtures'

const limits = { max_file_bytes: 20 * 1024 * 1024, max_files: 10 }

describe('Composer-Hilfen (WEB-006)', () => {
  it('WEB-006 AC2: @ vor dem Cursor wird erkannt und durch die Referenz ersetzt', () => {
    expect(mentionAt('Wende das Muster aus @mid', 25)).toEqual({ query: 'mid', start: 21 })
    expect(mentionAt('@', 1)).toEqual({ query: '', start: 0 })
    expect(mentionAt('mail@example.com', 16)).toBeUndefined()
    expect(mentionAt('Wende @mid an', 6)).toBeUndefined()
    expect(insertMention('Wende @mid an', 10, 'src/middleware.rs')).toEqual({ text: 'Wende @src/middleware.rs an', caret: 25 })
  })

  it('WEB-006 AC2: Treffer werden zusammenhängend oder unscharf hervorgehoben', () => {
    expect(highlight('src/middleware.rs', 'mid')).toEqual([
      { text: 'src/', hit: false },
      { text: 'mid', hit: true },
      { text: 'dleware.rs', hit: false },
    ])
    const fuzzy = highlight('src/middleware.rs', 'mdw')
    expect(fuzzy.filter((p) => p.hit).map((p) => p.text).join('')).toBe('mdw')
  })

  it('WEB-006 AC3: Slash-Menü nur für einen Befehl am Anfang', () => {
    expect(slashQuery('/')).toBe('')
    expect(slashQuery('/rev')).toBe('rev')
    expect(slashQuery('/review jetzt')).toBeUndefined()
    expect(slashQuery('Text /x')).toBeUndefined()
  })

  it('WEB-006: Typen, Größen und Grenzen', () => {
    expect(kindOf('image/png')).toBe('image')
    expect(kindOf('application/pdf')).toBe('pdf')
    expect(kindOf('text/markdown')).toBe('text')
    expect(kindOf('image/svg+xml')).toBeUndefined()
    expect(mimeOf('main.rs', '')).toBe('text/plain')
    expect(size(412 * 1024)).toBe('412 KB')
    expect(size(2.1 * 1024 * 1024)).toBe('2,1 MB')
    expect(caption('pdf', 'pentest-bericht-q3.pdf', 2.1 * 1024 * 1024)).toBe('PDF · 2,1 MB')
    expect(caption('image', 'Bildschirmfoto 2026.png', 412 * 1024)).toBe('Bildschirmfoto · 412 KB')
    expect(rejection({ name: 'a.png', size: 10, type: 'image/png' }, 10, limits)).toBe('höchstens 10 Dateien pro Nachricht.')
    expect(rejection({ name: 'a.png', size: 10, type: 'image/png' }, 9, limits)).toBeUndefined()
    expect(pastedName({ name: 'image.png', type: 'image/png' }, new Date(2026, 9, 4, 9, 5, 7))).toBe('Bildschirmfoto 2026-10-04 09.05.07.png')
  })

  it('WEB-006 AC1: Anhänge gesendeter Nachrichten kommen aus dem Log', () => {
    const blob = `sha256:${'b'.repeat(64)}`
    const content = [
      { type: 'text', text: 'Was zeigt das?' },
      { type: 'attachment', blob, name: 'shot.png', mime: 'image/png', size: 3 },
    ]
    expect(attachmentsOf(content)).toEqual([{ blob, name: 'shot.png', mime: 'image/png', size: 3 }])
    const items = timeline({
      events: [ev('message.completed', { message_id: 'm', role: 'user', content })],
      streaming: {},
      reasoning: {},
      toolOutput: {},
    })
    expect(items[0]).toMatchObject({ kind: 'user', text: 'Was zeigt das?', attachments: [{ name: 'shot.png' }] })
  })
})
