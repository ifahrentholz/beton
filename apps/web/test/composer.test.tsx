import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { Capabilities } from '@beton/sdk'
import { Composer, pickerOptions } from '@/components/composer'

const base = {
  mode: 'native',
  transport: 'in_proc',
  auth_sources: ['none'],
  approval: 'native_request',
  tool_call_gate: 'full',
  model_switch: 'live',
  effort_switch: 'none',
  resume: 'warm',
  fork_history: 'preamble',
  interrupt: true,
  steering: true,
  subagents: 'none',
  usage_reporting: 'tokens_and_cost',
  compaction: 'native',
  instructions_delivery: 'first_message_prefix',
  mcp_injection: false,
  images: false,
  transcript_import: false,
} as unknown as Capabilities

function renderComposer(caps: Capabilities, sessionId = 'ses_a') {
  return render(
    <Composer sessionId={sessionId} harness="fake" capabilities={caps} running={false} queued={[]} onSend={() => undefined} onInterrupt={() => undefined} />,
  )
}

beforeEach(() => localStorage.clear())
afterEach(() => cleanup())

describe('Composer', () => {
  it('WEB-004 AC1: Picker zeigen nur Optionen, die der Harness unterstützt', () => {
    const caps = { ...base, models: ['fake-small', 'fake-large'], efforts: ['low', 'high'] } as Capabilities
    expect(pickerOptions(caps)).toEqual({ models: ['fake-small', 'fake-large'], efforts: [] })
    renderComposer(caps)
    const model = screen.getByLabelText('Modell') as HTMLSelectElement
    expect([...model.options].map((o) => o.value)).toEqual(['fake-small', 'fake-large'])
    expect(screen.queryByLabelText('Effort')).toBeNull()
    cleanup()
    renderComposer({ ...caps, effort_switch: 'live' } as Capabilities)
    const effort = screen.getByLabelText('Effort') as HTMLSelectElement
    expect([...effort.options].map((o) => o.value)).toEqual(['low', 'high'])
    cleanup()
    renderComposer({ ...base } as Capabilities)
    expect(screen.queryByLabelText('Modell')).toBeNull()
  })

  it('WEB-004 AC3: Der Entwurf je Session überlebt einen Reload', () => {
    renderComposer(base)
    fireEvent.change(screen.getByLabelText('Nachricht'), { target: { value: 'halb fertig' } })
    cleanup()
    renderComposer(base)
    expect((screen.getByLabelText('Nachricht') as HTMLTextAreaElement).value).toBe('halb fertig')
    cleanup()
    renderComposer(base, 'ses_b')
    expect((screen.getByLabelText('Nachricht') as HTMLTextAreaElement).value).toBe('')
  })

  it('Senden mit Enter, Zeilenumbruch mit Shift+Enter', () => {
    const sent: string[] = []
    render(<Composer sessionId="ses_c" harness="fake" running={false} queued={[]} onSend={(t) => sent.push(t)} onInterrupt={() => undefined} />)
    const box = screen.getByLabelText('Nachricht')
    fireEvent.change(box, { target: { value: 'Zeile' } })
    act(() => {
      fireEvent.keyDown(box, { key: 'Enter', shiftKey: true })
    })
    expect(sent).toEqual([])
    act(() => {
      fireEvent.keyDown(box, { key: 'Enter' })
    })
    expect(sent).toEqual(['Zeile'])
  })
})
