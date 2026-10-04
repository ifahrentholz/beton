import type { Page } from '@playwright/test'
import { Daemon, attachLogs, expect, test } from './fixtures'
import { call, vendorEnv } from './vendors'

/*
 * M1-Demo in der Web-UI (WP-27): `maestra` lässt Claude und Codex parallel in eigenen Worktrees
 * implementieren und vom jeweils anderen Vendor reviewen; der Agents-Tab zeigt den Lauf als
 * Graph (WEB-012). Den ganzen Demo-Ablauf mit Fork und Import prüft
 * `crates/beton-cli/tests/m1_demo.rs` gegen `beton serve --dev`.
 */

const wait = (a: string, b: string) => call('session_wait', { ids: [a, b], mode: 'all', timeout: '50s' })
const approve = { message: 'VERDICT: approve\nBLOCKING:\n- keine' }
const usage = { usage: { input_tokens: 800, output_tokens: 120 } }

const rail = (page: Page) => page.getByTestId('workspace-rail')

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 })
})

test('M1-Demo (AGT-011 AC1, WEB-012 AC2): maestra verteilt auf Claude und Codex, der Graph zeigt Implementierungen und Cross-Reviews', async ({ page }, testInfo) => {
  const daemon = await Daemon.start({
    env: vendorEnv(
      [
        {
          expect_input: 'Baue einen Rate-Limiter mit Tests',
          emit: [
            call('session_list', {}),
            { message: 'Plan: 1. Limiter (impl-claude) 2. Tests (impl-codex)' },
            call('session_spawn', { agent: 'impl-claude', prompt: 'Teilaufgabe 1: Limiter', async: true, worktree: 'new' }),
            call('session_spawn', { agent: 'impl-codex', prompt: 'Teilaufgabe 2: Tests', async: true, worktree: 'new' }),
            wait('${mcp.1.session_id}', '${mcp.2.session_id}'),
            call('session_spawn', { agent: 'review-codex', prompt: 'Review Teilaufgabe 1', async: true }),
            call('session_spawn', { agent: 'review-claude', prompt: 'Review Teilaufgabe 2', async: true }),
            wait('${mcp.4.session_id}', '${mcp.5.session_id}'),
            { message: '| Limiter | impl-claude | review-codex | approved |\n| Tests | impl-codex | review-claude | approved |' },
          ],
        },
        { expect_input: 'Teilaufgabe 1: Limiter', emit: [{ write_file: { path: 'src/limiter.rs', content: 'pub struct Limiter;\n' } }, { message: 'BRANCH: limiter', delay_ms: 1500 }] },
        { expect_input: 'Review Teilaufgabe 2', emit: [approve] },
      ],
      [
        { expect_input: 'Teilaufgabe 2: Tests', emit: [{ write_file: { path: 'tests/limiter.rs', content: '#[test] fn limits() {}\n' } }, { message: 'BRANCH: tests', delay_ms: 1500 }, usage] },
        { expect_input: 'Review Teilaufgabe 1', emit: [approve, usage] },
      ],
    ),
  })
  try {
    const cwd = daemon.workspace({ 'README.md': '# Demo\n' }, true)
    const { id } = await daemon.api<{ id: string }>('POST', '/v1/sessions', { agent: 'maestra', cwd, title: 'Rate-Limiter mit Tests' })
    await daemon.waitStatus(id, 'idle')
    await daemon.login(page, `/s/${id}?tab=agents`)
    await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'Baue einen Rate-Limiter mit Tests' })
    await expect.poll(async () => (await daemon.events(id)).filter((e) => e.type === 'agent.completed').length, { timeout: 30_000 }).toBe(4)

    const nodes = rail(page).getByTestId('agent-node')
    await expect(nodes).toHaveCount(5)
    for (const [agent, harness] of [
      ['impl-claude', 'claude'],
      ['impl-codex', 'codex'],
      ['review-codex', 'codex'],
      ['review-claude', 'claude'],
    ] as const) {
      const n = nodes.filter({ hasText: `Agent ${agent}` })
      await expect(n).toHaveAttribute('data-harness', harness)
      await expect(n).toContainText('fertig')
    }
    await expect(rail(page).getByTestId('agents-summary')).toContainText('5 Sessions')
    await expect(rail(page).getByTestId('agents-summary')).toContainText('alles über Subscriptions')
    await expect(page.getByText('| Limiter | impl-claude | review-codex | approved |', { exact: false })).toBeVisible()
    await rail(page).screenshot({ path: testInfo.outputPath('maestra-graph.png') })
    await page.screenshot({ path: testInfo.outputPath('maestra-session.png') })
  } finally {
    await attachLogs(daemon, testInfo)
    daemon.stop()
  }
})
