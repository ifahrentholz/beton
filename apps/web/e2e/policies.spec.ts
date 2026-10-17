import AxeBuilder from '@axe-core/playwright'
import { expect, test } from './fixtures'

// Shadow-Modus der Policies (POL-027 AC4): Treffer einer Regel im `dry_run` erscheinen je Regel
// und Tag als „würde ablehnen“, ohne den Tool-Call zu verhindern.

const SHADOW = `spec_version: 1
name: shadow-npm
mode: dry_run
rules:
  - id: no-npm-publish
    on: tool_call
    when: 'tool.command.contains("publish")'
    action: deny
    reason: Veröffentlichen erst nach Review
`

const PUBLISH = `turns:
  - expect_input: "Bitte veröffentlichen"
    emit:
      - { tool_call: { name: Bash, kind: shell, args: { command: "npm publish --dry-run" } } }
      - { tool_result: "ok" }
      - { message: "Veröffentlicht." }
`

test('POL-027 AC4: ein Shadow-Treffer blockiert nicht und erscheint als „würde ablehnen“ je Tag', async ({ daemon, page }) => {
  const cwd = daemon.workspace({ '.beton/policies/shadow-npm.yaml': SHADOW })
  const id = await daemon.session(PUBLISH, 'Release vorbereiten', cwd)
  await daemon.login(page, `/s/${id}`)
  await page.getByLabel('Nachricht').fill('Bitte veröffentlichen')
  await page.getByLabel('Nachricht').press('Enter')
  // Der Tool-Call läuft durch: keine Freigabe, keine Ablehnung.
  await expect(page.getByText('Veröffentlicht.')).toBeVisible()
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  // Die Entscheidung zum Call steht im Log, mit dem Shadow-Treffer und `allow`.
  await expect
    .poll(async () => (await daemon.events(id)).find((e) => e.type === 'policy.decision' && JSON.stringify(e.payload).includes('no-npm-publish'))?.payload?.outcome)
    .toBe('allow')

  await page.getByTestId('nav-policies').click()
  await expect(page.getByRole('heading', { name: 'Shadow-Modus' })).toBeVisible()
  const row = page.getByTestId('shadow-rule')
  await expect(row).toHaveCount(1)
  await expect(row).toContainText('no-npm-publish')
  await expect(row).toContainText('shadow-npm (Projekt)')
  await expect(row).toContainText('würde ablehnen')
  await expect(row.getByTestId('shadow-count').last()).toHaveText('1')
  await expect(row).toContainText('Bash: npm publish --dry-run')
  await expect(row.getByRole('link', { name: 'Release vorbereiten' })).toBeVisible()
  const axe = await new AxeBuilder({ page }).include('main').analyze()
  expect(axe.violations).toEqual([])
})
