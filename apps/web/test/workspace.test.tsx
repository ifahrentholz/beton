import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BetonError, type Capabilities, type WorkspaceFile } from '@beton/sdk'
import { RailTabs } from '@/components/workspace/rail'
import {
  badgeCount,
  changedPaths,
  changedRegion,
  diffRows,
  lineLink,
  parseAnchor,
  turnLabel,
  turnsOf,
  validateWorkspaceSearch,
  visibleTabs,
  withAttachments,
  workspaceVersion,
} from '@/lib/workspace'
import { openFile, refresh, resolveConflict, save, setDraft, setWorkspaceApi, useWorkspace, type WorkspaceApi } from '@/store/workspace'
import { ev, resetSeq } from './fixtures'

const fsChanged = (paths: string[], source = 'watcher') => ev('fs.changed', { changes: paths.map((path) => ({ path, change: 'modified' })), source })

afterEach(() => cleanup())

describe('Workspace-Rail', () => {
  it('WEB-008 AC2: nicht verfügbare Tabs (Meilenstein, Capability, Rolle) sind ausgeblendet', () => {
    expect(visibleTabs({ role: 'owner' }).map((t) => t.id)).toEqual(['files', 'changes'])
    // Agents (WEB-012) nur bei Harnesses mit System-Tools (MCP) oder eigenen Sub-Agents.
    const native = { subagents: 'native', mcp_injection: false } as unknown as Capabilities
    expect(visibleTabs({ capabilities: native, role: 'owner' }).map((t) => t.id)).toEqual(['files', 'changes', 'agents'])
    const mcp = { subagents: 'none', mcp_injection: true } as unknown as Capabilities
    expect(visibleTabs({ capabilities: mcp, role: 'owner' }).map((t) => t.id)).toEqual(['files', 'changes', 'agents'])
    const plain = { subagents: 'none', mcp_injection: false } as unknown as Capabilities
    expect(visibleTabs({ capabilities: plain, role: 'owner' }).map((t) => t.id)).toEqual(['files', 'changes'])
    expect(visibleTabs({ role: 'viewer' }).map((t) => t.id)).not.toContain('terminal')
    render(<RailTabs tabs={visibleTabs({})} active="changes" badges={{}} onSelect={() => undefined} />)
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['Dateien', 'Änderungen'])
    for (const name of ['Terminal', 'Browser', 'Agents', 'PR', 'Side-Chats', 'Kommentare']) expect(screen.queryByText(name)).toBeNull()
  })

  it('WEB-008 AC1: fs.changed nach dem gesehenen Stand ergibt ein Change-Badge', () => {
    resetSeq()
    const events = [ev('turn.started', { turn_id: 'trn_1', author: 'usr_local' }), fsChanged(['a.ts']), fsChanged(['a.ts', 'b.ts'])]
    expect(changedPaths(events, 0)).toEqual(['a.ts', 'b.ts'])
    expect(badgeCount(events, 0)).toBe(2)
    expect(badgeCount(events, 2)).toBe(2)
    expect(badgeCount(events, 3)).toBe(0)
    expect(workspaceVersion(events)).toBe(3)
    const onSelect = vi.fn()
    render(<RailTabs tabs={visibleTabs({})} active="changes" badges={{ files: 2 }} onSelect={onSelect} />)
    expect(screen.getByTestId('badge-files').textContent).toBe('2')
    expect(screen.getByLabelText('2 Dateien geändert')).toBeTruthy()
    fireEvent.click(screen.getByRole('tab', { name: /Dateien/ }))
    expect(onSelect).toHaveBeenCalledWith('files')
  })
})

describe('Diff-Ansicht', () => {
  it('WEB-011 AC2: Zeilenanker als teilbarer Link (hin und zurück)', () => {
    const link = lineLink('http://127.0.0.1:7420', 'ses_1', { scope: 'turn', turn: 'trn_1', file: 'src/a b.ts', anchor: 'N16' })
    const url = new URL(link)
    expect(url.pathname).toBe('/s/ses_1')
    expect(validateWorkspaceSearch(Object.fromEntries(url.searchParams))).toEqual({ tab: 'changes', scope: 'turn', turn: 'trn_1', file: 'src/a b.ts', line: 'N16' })
    expect(parseAnchor('A7')).toEqual({ side: 'old', line: 7 })
    expect(parseAnchor('X1')).toBeUndefined()
    expect(validateWorkspaceSearch({ tab: 'terminal', line: 'kaputt' })).toEqual({ tab: undefined, scope: undefined, turn: undefined, file: undefined, line: undefined })
  })

  it('WEB-011: Hunks werden zu Zeilen mit Ankern auf die neue bzw. alte Seite', () => {
    const rows = diffRows({
      hunks: [
        {
          header: '@@ -15,2 +15,2 @@',
          old_start: 15,
          old_lines: 2,
          new_start: 15,
          new_lines: 2,
          lines: [
            { kind: 'context', old_line: 15, new_line: 15, text: 'a', no_newline: false },
            { kind: 'delete', old_line: 16, text: 'b', no_newline: false },
            { kind: 'add', new_line: 16, text: 'c', no_newline: false },
          ],
        },
      ],
    })
    expect(rows.map((r) => (r.kind === 'line' ? r.anchor : r.text))).toEqual(['@@ -15,2 +15,2 @@', 'N15', 'A16', 'N16'])
  })

  it('Turns mit Nummer, Uhrzeit und auslösender Nachricht', () => {
    resetSeq()
    const turns = turnsOf([
      ev('message.completed', { message_id: 'm1', role: 'user', content: [{ type: 'text', text: 'Bitte mit Tests' }] }),
      ev('turn.started', { turn_id: 'trn_1', author: 'usr_local' }),
    ])
    expect(turns).toHaveLength(1)
    expect(turnLabel(turns[0]!)).toMatch(/^Turn 1 · \d\d:\d\d · „Bitte mit Tests“$/)
  })
})

describe('Editor', () => {
  let disk: { content: string; sha: string }
  let writes: { content: string; ifMatch: string | undefined }[]
  const file = (): WorkspaceFile => ({ path: 'a.ts', size: disk.content.length, sha256: disk.sha, binary: false, too_large: false, content: disk.content })
  const api: WorkspaceApi = {
    readFile: async () => file(),
    writeFile: async (_p, content, ifMatch) => {
      writes.push({ content, ifMatch })
      if (ifMatch && ifMatch !== disk.sha) throw new BetonError(412, { status: 412, code: 'precondition_failed' })
      disk = { content, sha: `sha-${content.length}-${writes.length}` }
      return { path: 'a.ts', size: content.length, sha256: disk.sha, created: false }
    },
  }
  const f = () => useWorkspace.getState().ws('ses_e').files['a.ts']!

  beforeEach(async () => {
    disk = { content: 'eins\n', sha: 'sha-1' }
    writes = []
    useWorkspace.setState({ sessions: {} })
    setWorkspaceApi(() => api)
    await openFile('ses_e', 'a.ts')
  })

  it('WEB-009 AC1: Agent ändert eine ungespeicherte Datei → Konflikt statt stillem Überschreiben', async () => {
    setDraft('ses_e', 'a.ts', 'eins\nmeins\n')
    disk = { content: 'eins\nAgent\n', sha: 'sha-agent' }
    await refresh('ses_e', 'a.ts')
    expect(f().conflict).toMatchObject({ theirs: 'eins\nAgent\n', theirsSha: 'sha-agent', deleted: false })
    expect(f().draft).toBe('eins\nmeins\n')
    expect(writes).toHaveLength(0)
    // „Meine Version speichern“ überschreibt genau den Stand des Agents (If-Match).
    await resolveConflict('ses_e', 'a.ts', 'mine')
    expect(writes).toEqual([{ content: 'eins\nmeins\n', ifMatch: 'sha-agent' }])
    expect(f().conflict).toBeUndefined()
    expect(f().base).toBe('eins\nmeins\n')
  })

  it('WEB-009 AC1: ohne eigene Änderungen lädt der Editor still neu; „Version des Agents laden“ verwirft den Entwurf', async () => {
    disk = { content: 'zwei\n', sha: 'sha-2' }
    await refresh('ses_e', 'a.ts')
    expect(f().conflict).toBeUndefined()
    expect(f().draft).toBe('zwei\n')
    setDraft('ses_e', 'a.ts', 'meins\n')
    disk = { content: 'drei\n', sha: 'sha-3' }
    await refresh('ses_e', 'a.ts')
    await resolveConflict('ses_e', 'a.ts', 'theirs')
    expect(f()).toMatchObject({ draft: 'drei\n', base: 'drei\n', sha: 'sha-3', conflict: undefined })
  })

  it('WEB-009 AC1: Speichern mit veraltetem If-Match (412) führt in den Konflikt', async () => {
    setDraft('ses_e', 'a.ts', 'meins\n')
    disk = { content: 'Agent\n', sha: 'sha-agent' }
    expect(await save('ses_e', 'a.ts')).toBe(false)
    expect(writes).toEqual([{ content: 'meins\n', ifMatch: 'sha-1' }])
    expect(disk.content).toBe('Agent\n')
    expect(f().conflict?.theirs).toBe('Agent\n')
  })

  it('Eigenes Speichern löst beim folgenden fs.changed keinen Konflikt aus', async () => {
    setDraft('ses_e', 'a.ts', 'neu\n')
    expect(await save('ses_e', 'a.ts')).toBe(true)
    setDraft('ses_e', 'a.ts', 'neu\nweiter\n')
    await refresh('ses_e', 'a.ts')
    expect(f().conflict).toBeUndefined()
    expect(f().draft).toBe('neu\nweiter\n')
  })

  it('Gegenüberstellung zeigt den ersten geänderten Bereich mit Zeilennummer', () => {
    expect(changedRegion('a\nb\nc\n', 'a\nB\nc\n')).toEqual({ start: 2, lines: ['B'] })
  })

  it('An Agent anhängen: Referenz mit Ausschnitt in der Nachricht', () => {
    const text = withAttachments('Was passiert hier?', [{ id: 'x', path: 'src/a.ts', from: 13, to: 20, snippet: 'const id = 1' }])
    expect(text).toBe('Was passiert hier?\n\n`src/a.ts` Z. 13–20:\n```typescript\nconst id = 1\n```')
  })
})
