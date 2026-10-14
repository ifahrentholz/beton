import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApprovalBar, KEY_DELAY_MS } from '@/components/approval-bar'
import { approvalItem, type ApprovalItem } from '@/lib/timeline'
import { editPreview, resolutionText, timeLeft } from '@/lib/approvals'

const api = vi.hoisted(() => ({ resolve: vi.fn() }))
vi.mock('@/lib/client', () => ({
  client: { session: () => ({ resolveApproval: api.resolve }) },
}))

const future = new Date(Date.now() + 28 * 60_000 + 30_000).toISOString()

function card(id: string, over: Partial<{ command: string; options: string[]; kind: string; subject: unknown }> = {}): ApprovalItem {
  return approvalItem(id, {
    approval_id: id,
    kind: over.kind ?? 'policy',
    subject: over.subject ?? {
      phase: 'tool_call',
      title: 'Shell-Kommando bestätigen',
      summary: over.command ?? `git push ${id}`,
      tool: { name: 'Bash', kind: 'shell', args: { command: over.command ?? `git push ${id}` } },
      reasons: [{ rule_id: 'git-guard', scope: 'project', set: 'shop', reason: 'Push verlässt den Rechner', severity: 'high' }],
      requested_by: { actor: 'agent', harness: 'claude' },
    },
    options: over.options ?? ['approve', 'approve_session', 'deny'],
    expires_at: future,
    on_timeout: 'deny',
  })
}

function decided(item: ApprovalItem, decision = 'allow'): ApprovalItem {
  return { ...item, decision, resolution: { decision, via: 'user', actor: { kind: 'user', id: 'usr_local' } } }
}

beforeEach(() => {
  api.resolve.mockReset()
  api.resolve.mockResolvedValue(undefined)
})
afterEach(cleanup)

/** Eine neue oberste Card ist erst nach kurzer Sichtzeit per Taste entscheidbar. */
const armed = () => new Promise((r) => setTimeout(r, KEY_DELAY_MS + 50))

const key = (k: string, opts: Partial<KeyboardEventInit> = {}) =>
  act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: k, metaKey: true, bubbles: true, cancelable: true, ...opts }))
  })

describe('Freigabe-Leiste (WEB-007)', () => {
  it('WEB-007 AC1: ⌘⏎ erlaubt die oberste Freigabe, danach zeigt die Leiste die nächste', async () => {
    const a = card('apr_a')
    const b = card('apr_b')
    const { rerender } = render(<ApprovalBar sessionId="ses_1" approvals={[a, b]} />)
    expect(screen.getByTestId('approval-top').dataset.approvalId).toBe('apr_a')
    expect(screen.getByText('Danach · 1 weitere')).toBeTruthy()
    expect(screen.getByText('1 von 2')).toBeTruthy()
    await armed()
    key('Enter')
    await waitFor(() => expect(api.resolve).toHaveBeenCalledWith('apr_a', { decision: 'allow' }))
    // Das Event `approval.resolved` kommt an: die nächste rückt nach oben.
    rerender(<ApprovalBar sessionId="ses_1" approvals={[decided(a), b]} />)
    expect(screen.getByTestId('approval-top').dataset.approvalId).toBe('apr_b')
    expect(screen.getByTestId('approval-notice').textContent).toContain('Erlaubt von dir')
    expect(screen.getByTestId('approval-notice').textContent).not.toContain('anderen Fenster')
  })

  it('WEB-007 AC1: ⌘⇧⏎ erlaubt für die Session nur, wenn die Card es anbietet', async () => {
    const { rerender } = render(<ApprovalBar sessionId="ses_1" approvals={[card('apr_a', { options: ['approve', 'deny'] })]} />)
    expect(screen.queryByText('Für diese Session erlauben')).toBeNull()
    await armed()
    key('Enter', { shiftKey: true })
    expect(api.resolve).not.toHaveBeenCalled()
    rerender(<ApprovalBar sessionId="ses_1" approvals={[card('apr_b')]} />)
    await armed()
    key('Enter', { shiftKey: true })
    await waitFor(() => expect(api.resolve).toHaveBeenCalledWith('apr_b', { decision: 'allow', remember: 'session' }))
  })

  it('WEB-007: ⌘⌫ öffnet Ablehnen mit Grund, ein zweites ⌘⌫ lehnt ab', async () => {
    render(<ApprovalBar sessionId="ses_1" approvals={[card('apr_a')]} />)
    await armed()
    key('Backspace')
    const reason = screen.getByLabelText('Grund für die Ablehnung, geht an den Agent')
    await waitFor(() => expect(document.activeElement).toBe(reason))
    fireEvent.change(reason, { target: { value: 'Erst die CI abwarten' } })
    key('Backspace')
    await waitFor(() => expect(api.resolve).toHaveBeenCalledWith('apr_a', { decision: 'deny', reason: 'Erst die CI abwarten' }))
  })

  it('WEB-007: in einem Feld mit Text bleibt ⌘⌫ Löschen', () => {
    render(
      <>
        <textarea aria-label="Nachricht" defaultValue="halber Satz" />
        <ApprovalBar sessionId="ses_1" approvals={[card('apr_a')]} />
      </>,
    )
    const box = screen.getByLabelText('Nachricht')
    act(() => {
      box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Backspace', metaKey: true, bubbles: true, cancelable: true }))
    })
    expect(screen.queryByLabelText('Grund für die Ablehnung, geht an den Agent')).toBeNull()
  })

  it('WEB-007 AC2: entscheidet ein anderer Client, verschwindet die Freigabe mit Vermerk', () => {
    const a = card('apr_a')
    const { rerender } = render(<ApprovalBar sessionId="ses_1" approvals={[a]} />)
    rerender(<ApprovalBar sessionId="ses_1" approvals={[decided(a)]} />)
    expect(screen.queryByTestId('approval-top')).toBeNull()
    expect(screen.getByTestId('approval-notice').textContent).toContain('Erlaubt von dir in einem anderen Fenster')
  })

  it('WEB-007 AC3: role="alertdialog" mit Titel und Beschreibung, ⌘⇧A springt hinein, Esc zurück', async () => {
    render(
      <>
        <textarea aria-label="Nachricht" />
        <ApprovalBar sessionId="ses_1" approvals={[card('apr_a'), card('apr_b')]} />
      </>,
    )
    const bar = screen.getByRole('alertdialog')
    expect(bar.getAttribute('aria-labelledby')).toBeTruthy()
    expect(document.getElementById(bar.getAttribute('aria-labelledby')!)?.textContent).toContain('Freigabe nötig')
    expect(document.getElementById(bar.getAttribute('aria-describedby')!)?.textContent).toContain('git push apr_a')
    key('a', { shiftKey: true })
    expect(document.activeElement).toBe(bar)
    // ↓ holt den nächsten Eintrag nach vorn.
    fireEvent.keyDown(bar, { key: 'ArrowDown' })
    expect(screen.getByTestId('approval-top').dataset.approvalId).toBe('apr_b')
    fireEvent.keyDown(bar, { key: 'Escape' })
    expect(document.activeElement).toBe(screen.getByLabelText('Nachricht'))
  })

  it('AGT-007 AC4: Fragen des Agents beantworten (Vorschlag oder eigene Antwort)', async () => {
    const q = card('apr_q', { kind: 'question', subject: { question: 'Pro IP oder pro Account?', choices: ['Pro IP', 'Pro Account'] }, options: ['answer'] })
    render(<ApprovalBar sessionId="ses_1" approvals={[q]} />)
    expect(screen.getByRole('alertdialog').textContent).toContain('Pro IP oder pro Account?')
    fireEvent.click(screen.getByRole('button', { name: 'Pro Account' }))
    await armed()
    key('Enter')
    await waitFor(() => expect(api.resolve).toHaveBeenCalledWith('apr_q', { decision: 'answer', answer: 'Pro Account' }))
  })

  it('WEB-007: Argumente ausklappbar, Datei-Edits als Diff', () => {
    const edit = card('apr_e', {
      subject: {
        title: 'Datei-Änderung bestätigen',
        summary: 'src/a.ts',
        tool: { name: 'Edit', kind: 'file_edit', args: { file_path: 'src/a.ts', old_string: 'alt', new_string: 'neu\nzwei' } },
        reasons: [],
      },
    })
    render(<ApprovalBar sessionId="ses_1" approvals={[edit]} />)
    fireEvent.click(screen.getByRole('button', { name: /Änderung an src\/a.ts/ }))
    const details = screen.getByTestId('approval-details')
    expect(details.textContent).toContain('alt')
    expect(details.textContent).toContain('neu')
  })
})

describe('Freigabe-Leiste: Schutz vor versehentlichen Entscheidungen (Security-Review M1)', () => {
  it('WEB-007: ⌘⏎ im Grund-Feld lehnt mit Grund ab statt zu erlauben', async () => {
    render(<ApprovalBar sessionId="ses_1" approvals={[card('apr_a')]} />)
    await armed()
    key('Backspace')
    const reason = screen.getByLabelText('Grund für die Ablehnung, geht an den Agent')
    fireEvent.change(reason, { target: { value: 'Nein' } })
    act(() => {
      reason.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', metaKey: true, bubbles: true, cancelable: true }))
    })
    await waitFor(() => expect(api.resolve).toHaveBeenCalledWith('apr_a', { decision: 'deny', reason: 'Nein' }))
    expect(api.resolve).toHaveBeenCalledTimes(1)
  })

  it('WEB-007: gehaltenes ⌘⏎ (Wiederholung) entscheidet nichts', async () => {
    render(<ApprovalBar sessionId="ses_1" approvals={[card('apr_a')]} />)
    await armed()
    key('Enter', { repeat: true })
    key('Enter', { shiftKey: true, repeat: true })
    expect(api.resolve).not.toHaveBeenCalled()
  })

  it('WEB-007: eine neue oberste Card ist erst nach kurzer Sichtzeit per Taste entscheidbar', async () => {
    const a = card('apr_a')
    const b = card('apr_b')
    const { rerender } = render(<ApprovalBar sessionId="ses_1" approvals={[a, b]} />)
    // Sofort nach dem Erscheinen: nichts.
    key('Enter')
    expect(api.resolve).not.toHaveBeenCalled()
    await armed()
    key('Enter')
    await waitFor(() => expect(api.resolve).toHaveBeenCalledTimes(1))
    // Die nächste rückt nach: wieder erst nach der Sichtzeit.
    rerender(<ApprovalBar sessionId="ses_1" approvals={[decided(a), b]} />)
    key('Enter')
    expect(api.resolve).toHaveBeenCalledTimes(1)
    await armed()
    key('Enter')
    await waitFor(() => expect(api.resolve).toHaveBeenLastCalledWith('apr_b', { decision: 'allow' }))
  })
})

describe('Hilfen der Freigaben', () => {
  it('Restzeit und Vermerke', () => {
    const now = Date.parse('2026-10-14T12:00:00Z')
    expect(timeLeft('2026-10-14T12:28:00Z', 'deny', now)).toEqual({ text: 'läuft in 28 Min. ab · dann ablehnen', urgent: false })
    expect(timeLeft('2026-10-14T12:04:00Z', 'abort', now)?.urgent).toBe(true)
    expect(timeLeft('2026-10-14T14:30:00Z', undefined, now)?.text).toBe('läuft in 2 Std. 30 Min. ab')
    expect(resolutionText({ decision: 'deny', via: 'timeout', actor: {}, onTimeoutApplied: 'abort' }, false)).toContain('angewendet: abgebrochen')
    expect(resolutionText({ decision: 'allow', via: 'user', actor: { kind: 'user', id: 'usr_mara' }, remember: 'session' }, false)).toBe(
      'Für diese Session erlaubt von usr_mara',
    )
  })

  it('Diff aus apply_patch und Write', () => {
    const patch = editPreview({ tool: 'apply_patch', args: { input: '*** Begin Patch\n*** Update File: a.txt\n@@\n-a\n+b\n*** End Patch' } })
    expect(patch?.file).toBe('a.txt')
    expect(patch?.stat).toBe('+1 −1')
    const write = editPreview({ tool: 'Write', toolKind: 'file_write', args: { file_path: 'x.md', content: 'eins\nzwei' } })
    expect(write?.stat).toBe('+2 −0')
    expect(editPreview({ tool: 'Bash', args: { command: 'ls' } })).toBeUndefined()
  })
})
