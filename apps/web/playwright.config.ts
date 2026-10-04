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
    ...devices['Desktop Chrome'],
    trace: 'retain-on-failure',
  },
})
