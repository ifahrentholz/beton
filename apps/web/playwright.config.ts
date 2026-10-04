import { defineConfig, devices } from '@playwright/test'

/**
 * E2E gegen den echten Daemon (`beton serve --dev`) mit Fake-Harness; die Web-UI kommt aus
 * `dist/` (vorher `pnpm build`). Binary: `BETON_BIN`, sonst `target/debug/beton`.
 */
export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  // Das M0-Szenario läuft auf allen drei Engines (QA-007 AC1), der Rest auf Chromium.
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] }, grep: /@alle-engines/ },
    { name: 'webkit', use: { ...devices['Desktop Safari'] }, grep: /@alle-engines/ },
  ],
})
