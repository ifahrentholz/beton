import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { SessionTitle } from '@/components/session-title'
import { PLACEHOLDER, titleFrom, windowTitle } from '@/lib/title'
import { ev, resetSeq } from './fixtures'

beforeEach(() => resetSeq())
afterEach(() => cleanup())

const titled = (title: string, source: string) => ev('session.title_changed', { title, source })

describe('Session-Titel (UX-009)', () => {
  it('UX-009 AC1: erzeugter Titel ersetzt den Platzhalter, auch im Fenstertitel', () => {
    expect(titleFrom({ title: '' }, [])).toEqual({ title: '', source: undefined })
    expect(windowTitle('')).toBe(`${PLACEHOLDER} · beton`)
    const t = titleFrom({ title: '' }, [titled('Rate-Limiter für die Login-API', 'generated')])
    expect(t).toEqual({ title: 'Rate-Limiter für die Login-API', source: 'generated' })
    expect(windowTitle(t.title)).toBe('Rate-Limiter für die Login-API · beton')
  })

  it('UX-009 AC2: nach dem Umbenennen werden erzeugte Titel nicht mehr angewendet', () => {
    const events = [titled('Erzeugt', 'generated'), titled('Von mir', 'user'), titled('Später erzeugt', 'generated')]
    expect(titleFrom({ title: '' }, events)).toEqual({ title: 'Von mir', source: 'user' })
    // Auch wenn der Listen-Stand schon den User-Titel kennt und ein erzeugter nachkommt.
    expect(titleFrom({ title: 'Von mir', title_source: 'user' }, [titled('Später erzeugt', 'generated')])).toEqual({
      title: 'Von mir',
      source: 'user',
    })
    // Der User darf weiter umbenennen.
    expect(titleFrom({ title: 'Von mir', title_source: 'user' }, [titled('Neu', 'user')]).title).toBe('Neu')
  })

  it('UX-009: Platzhalter, „automatisch“ und „von dir benannt“', () => {
    const { rerender } = render(<SessionTitle title="" onRename={() => Promise.resolve()} />)
    const h = screen.getByRole('heading')
    expect(h.textContent).toBe(PLACEHOLDER)
    expect(h.className).toContain('italic')
    rerender(<SessionTitle title="Rate-Limiter" source="generated" onRename={() => Promise.resolve()} />)
    expect(screen.getByRole('heading').textContent).toBe('Rate-Limiter')
    expect(screen.getByText('automatisch')).toBeTruthy()
    rerender(<SessionTitle title="Login-Schutz" source="user" onRename={() => Promise.resolve()} />)
    expect(screen.getByText('von dir benannt')).toBeTruthy()
  })

  it('UX-009 AC2: Inline-Umbenennen per Doppelklick bzw. F2, Esc bricht ab', async () => {
    const renamed: string[] = []
    render(<SessionTitle title="Rate-Limiter" source="generated" onRename={(t) => (renamed.push(t), Promise.resolve())} />)
    fireEvent.doubleClick(screen.getByRole('heading'))
    const input = screen.getByLabelText('Titel der Session') as HTMLInputElement
    expect(screen.getByText(/danach kein automatischer Titel mehr/)).toBeTruthy()
    fireEvent.change(input, { target: { value: 'Login-Schutz gegen Brute-Force' } })
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' })
    })
    expect(renamed).toEqual(['Login-Schutz gegen Brute-Force'])
    expect(screen.queryByLabelText('Titel der Session')).toBeNull()
    // F2 außerhalb von Eingabefeldern öffnet das Umbenennen, Esc bricht ab.
    act(() => {
      fireEvent.keyDown(window, { key: 'F2' })
    })
    const again = screen.getByLabelText('Titel der Session')
    fireEvent.change(again, { target: { value: 'verworfen' } })
    fireEvent.keyDown(again, { key: 'Escape' })
    expect(screen.queryByLabelText('Titel der Session')).toBeNull()
    expect(renamed).toEqual(['Login-Schutz gegen Brute-Force'])
  })
})
