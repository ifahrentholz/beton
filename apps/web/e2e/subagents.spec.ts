import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Page } from '@playwright/test'
import { Daemon, attachLogs, expect, test } from './fixtures'
import { fakeCli } from './fake-cli'

/*
 * Sub-Agent-Graph (WEB-012) gegen den echten Daemon: ein Projekt-Agent auf Claude (Fake-CLI)
 * startet Sub-Agents auf Codex und Claude (Fake-CLIs, AGT-009). Eine Szenario-Datei je Harness
 * bedient Parent und Childs (`select: by_input`).
 */

const call = (tool: string, args: unknown) => ({ mcp_call: { server: 'beton', tool, args } })

/** Fake-CLIs für Claude und Codex mit je einem Szenario (JSON ist gültiges YAML). */
function vendorEnv(claude: unknown[], codex: unknown[]): Record<string, string> {
  const dir = mkdtempSync(join(tmpdir(), 'beton-e2e-sub-'))
  const write = (name: string, turns: unknown[]) => {
    const p = join(dir, name)
    writeFileSync(p, JSON.stringify({ select: 'by_input', turns }))
    return p
  }
  // Ein Link namens `codex`, damit der Versions-Probe die Codex-Version liest.
  const link = join(dir, 'codex')
  symlinkSync(fakeCli(), link)
  return {
    BETON_CLAUDE_PATH: `${fakeCli()} --protocol stream-json --scenario ${write('claude.yaml', claude)}`,
    BETON_CODEX_PATH: `${link} --protocol app-server --scenario ${write('codex.yaml', codex)}`,
  }
}

const LEAD = `spec_version: 1
name: lead
executor: { harness: claude }
agents:
  tests: { executor: { harness: claude }, description: Schreibt Tests }
  review: { executor: { harness: codex }, description: Reviewt }
spawn: { agents: [tests, review], max_concurrent: 3 }
`

const rail = (page: Page) => page.getByTestId('workspace-rail')
const node = (page: Page, id: string) => rail(page).locator(`[data-testid="agent-node"][data-session="${id}"]`)

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 })
})

test('WEB-012 AC1 + AC2: neue Sub-Agents erscheinen binnen 1 s als Knoten mit Harness, Status und Kosten', async ({ page }, testInfo) => {
  const daemon = await Daemon.start({
    env: vendorEnv(
      [
        {
          expect_input: 'los',
          emit: [
            call('session_spawn', { agent: 'review', prompt: 'review', async: true }),
            { message: 'Review läuft', delay_ms: 2500 },
            call('session_spawn', { agent: 'tests', prompt: 'tests', async: true }),
            call('session_wait', { timeout: '50s' }),
            { message: 'Beide fertig.' },
          ],
        },
        { expect_input: 'tests', emit: [{ message: 'Tests geschrieben', delay_ms: 1500 }, { usage: { input_tokens: 1200, output_tokens: 80 } }] },
      ],
      [{ expect_input: 'review', emit: [{ message: 'Keine Befunde', delay_ms: 3000 }, { usage: { input_tokens: 900, output_tokens: 40 } }] }],
    ),
  })
  try {
    const cwd = daemon.workspace({ 'README.md': '# Projekt\n' }, true)
    mkdirSync(join(cwd, '.beton/agents/lead'), { recursive: true })
    writeFileSync(join(cwd, '.beton/agents/lead/agent.yaml'), LEAD)
    const { id } = await daemon.api<{ id: string }>('POST', '/v1/sessions', { agent: 'lead', cwd, title: 'Rate-Limiter' })
    await daemon.waitStatus(id, 'idle')

    await daemon.login(page, `/s/${id}?tab=agents`)
    await expect(rail(page).getByRole('tab', { name: /^Agents/ })).toHaveAttribute('aria-selected', 'true')
    await expect(rail(page).getByText('Noch keine Sub-Agents')).toBeVisible()
    // Im Browser festhalten, wann ein Knoten erscheint (Uhr des Rechners wie die des Daemons).
    await page.evaluate(() => {
      const seen: Record<string, number> = {}
      ;(window as unknown as { __seen: Record<string, number> }).__seen = seen
      new MutationObserver(() => {
        for (const el of document.querySelectorAll<HTMLElement>('[data-testid="agent-node"]')) {
          const sid = el.dataset.session
          if (sid && !(sid in seen)) seen[sid] = Date.now()
        }
      }).observe(document.body, { subtree: true, childList: true })
    })

    await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'los' })
    await expect.poll(async () => (await daemon.events(id)).filter((e) => e.type === 'agent.spawned').length, { timeout: 20_000 }).toBe(2)
    const spawned = (await daemon.api<{ items: Array<{ type: string; ts: string; payload: { child_session_id: string; harness: string } }> }>('GET', `/v1/sessions/${id}/events?after_seq=0&limit=200`)).items.filter(
      (e) => e.type === 'agent.spawned',
    )
    for (const e of spawned) {
      await expect(node(page, e.payload.child_session_id)).toBeVisible()
      const seenAt = await page.evaluate((sid) => (window as unknown as { __seen: Record<string, number> }).__seen[sid], e.payload.child_session_id)
      // AC1: höchstens 1 s nach dem Start-Event.
      expect(seenAt - Date.parse(e.ts)).toBeLessThanOrEqual(1000)
    }
    await rail(page).screenshot({ path: testInfo.outputPath('graph-live.png') })

    // AC2: Harness-Icon (Stimmfarbe + Name), Status und kumulierte Kosten je Knoten.
    const review = node(page, spawned[0]!.payload.child_session_id)
    await expect(review).toHaveAttribute('data-harness', 'codex')
    await expect(review).toContainText('Codex')
    await expect(review.locator('[data-status]')).toHaveCount(1)
    await expect.poll(async () => (await daemon.events(id)).filter((e) => e.type === 'agent.completed').length, { timeout: 20_000 }).toBe(2)
    await expect(review.getByTestId('agent-node-cost')).toHaveText(/940 Tokens · Subscription/)
    await expect(review.locator('[data-status]')).toHaveAttribute('data-status', 'idle')
    await expect(review).toContainText('fertig')
    const root = node(page, id)
    await expect(root.getByTestId('agent-node-cost')).toContainText('Tokens')
    await expect(rail(page).getByTestId('agents-summary')).toContainText('3 Sessions')
    await rail(page).screenshot({ path: testInfo.outputPath('graph-done.png') })

    // Klick öffnet die Sub-Session.
    await review.click()
    await expect(page).toHaveURL(new RegExp(`/s/${spawned[0]!.payload.child_session_id}`))
  } finally {
    await attachLogs(daemon, testInfo)
    daemon.stop()
  }
})
