import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import { cpSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test as base, type Page, type TestInfo } from '@playwright/test'

const repo = resolve(import.meta.dirname, '..', '..', '..')
export const BETON = process.env.BETON_BIN ?? join(repo, 'target', 'debug', 'beton')
const WEB_DIR = resolve(import.meta.dirname, '..', 'dist')

/** Freier Port auf Loopback (für Neustarts auf demselben Port). */
export async function freePort(): Promise<number> {
  const { createServer } = await import('node:net')
  return new Promise((resolve) => {
    const srv = createServer()
    srv.listen(0, '127.0.0.1', () => {
      const port = (srv.address() as { port: number }).port
      srv.close(() => resolve(port))
    })
  })
}

export interface DaemonOptions {
  /** Zusätzliche Umgebung für den Daemon (und damit die Runner), z. B. `BETON_CLAUDE_PATH`. */
  env?: Record<string, string>
  /** Fester Port statt eines beliebigen freien. */
  port?: number
}

/** Ein laufender Daemon mit eigenem Datenverzeichnis. */
export class Daemon {
  readonly home: string
  readonly work: string
  private child: ChildProcess
  url: string
  private readonly token: string
  private readonly options: DaemonOptions

  private constructor(home: string, work: string, child: ChildProcess, url: string, token: string, options: DaemonOptions) {
    this.home = home
    this.work = work
    this.child = child
    this.url = url
    this.token = token
    this.options = options
  }

  static async start(options: DaemonOptions = {}): Promise<Daemon> {
    if (!existsSync(BETON)) throw new Error(`beton fehlt unter ${BETON} (cargo build -p beton-cli)`)
    if (!existsSync(join(WEB_DIR, 'index.html'))) throw new Error('Web-UI fehlt: pnpm build')
    const home = mkdtempSync(join(tmpdir(), 'beton-e2e-home-'))
    const work = mkdtempSync(join(tmpdir(), 'beton-e2e-work-'))
    writeFileSync(join(home, 'config.yaml'), `server:\n  listen: ["127.0.0.1:${options.port ?? 0}"]\n`)
    const { child, url } = await Daemon.spawn(home, options)
    const token = readFileSync(join(home, 'auth', 'local.token'), 'utf8').trim()
    return new Daemon(home, work, child, url, token, options)
  }

  private static async spawn(home: string, options: DaemonOptions): Promise<{ child: ChildProcess; url: string }> {
    const info = join(home, 'run', 'daemon.json')
    rmSync(info, { force: true })
    const child = spawn(BETON, ['serve', '--foreground', '--dev', '-q'], {
      env: { ...process.env, BETON_HOME: home, BETON_WEB_DIR: WEB_DIR, ...options.env },
      stdio: 'ignore',
    })
    const deadline = Date.now() + 30_000
    while (!existsSync(info)) {
      if (Date.now() > deadline) throw new Error('daemon.json erscheint nicht')
      await new Promise((r) => setTimeout(r, 50))
    }
    const { http } = JSON.parse(readFileSync(info, 'utf8')) as { http: string }
    return { child, url: `http://${http}` }
  }

  /** Daemon geordnet beenden (SIGTERM) und mit demselben Datenverzeichnis neu starten. */
  async restart(): Promise<void> {
    const exited = new Promise((r) => this.child.once('exit', r))
    this.child.kill('SIGTERM')
    await exited
    const { child, url } = await Daemon.spawn(this.home, this.options)
    this.child = child
    this.url = url
  }

  /** Logs des Daemons und der Runner (für CI-Artefakte, QA-007 AC3). */
  logsDir(): string {
    return join(this.home, 'logs')
  }

  stop(): void {
    this.child.kill('SIGTERM')
    rmSync(this.work, { recursive: true, force: true })
  }

  env(extra: Record<string, string> = {}): NodeJS.ProcessEnv {
    return { ...process.env, BETON_HOME: this.home, ...extra }
  }

  /** REST mit dem lokalen Token (Testaufbau, nicht die UI). */
  async api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
    const res = await fetch(`${this.url}${path}`, {
      method,
      headers: { authorization: `Bearer ${this.token}`, 'content-type': 'application/json' },
      body: body === undefined ? null : JSON.stringify(body),
    })
    const text = await res.text()
    if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text}`)
    return (text ? JSON.parse(text) : undefined) as T
  }

  scenario(name: string, yaml: string): string {
    const p = join(this.work, `${name}.yaml`)
    writeFileSync(p, yaml)
    return p
  }

  async session(yaml: string, title = 'E2E'): Promise<string> {
    const scenario = this.scenario(`s${Date.now()}${Math.random().toString(36).slice(2)}`, yaml)
    const s = await this.api<{ id: string }>('POST', '/v1/sessions', {
      target: 'fake',
      cwd: this.work,
      title,
      harness_opts: { scenario },
    })
    await this.waitStatus(s.id, 'idle')
    return s.id
  }

  async waitStatus(id: string, status: string): Promise<void> {
    const deadline = Date.now() + 20_000
    for (;;) {
      const s = await this.api<{ status: string }>('GET', `/v1/sessions/${id}`)
      if (s.status === status) return
      if (Date.now() > deadline) throw new Error(`Session ${id} bleibt ${s.status}`)
      await new Promise((r) => setTimeout(r, 50))
    }
  }

  async events(id: string): Promise<Array<{ type: string; payload?: Record<string, unknown> }>> {
    const page = await this.api<{ items: Array<{ type: string; payload?: Record<string, unknown> }> }>(
      'GET',
      `/v1/sessions/${id}/events?after_seq=0&limit=200`,
    )
    return page.items
  }

  /** Einmal-Link wie `beton open` (AUTH-004) und Navigation zu `next`. */
  async login(page: Page, next = '/'): Promise<void> {
    const out = execFileSync(BETON, ['open', '--json'], { env: this.env({ BROWSER: 'true' }), encoding: 'utf8' })
    const { url } = JSON.parse(out) as { url: string }
    await page.goto(`${url}&next=${encodeURIComponent(next)}`)
  }
}

export const PUSH_ASK = `turns:
  - expect_input: "Bitte pushen"
    emit:
      - { message_delta: "Ich pushe jetzt.", chunk: 4, delay_ms: 10 }
      - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "ok" }, { message: "Push erledigt." }], deny: [{ message: "Push abgelehnt." }] } }
`

/** Hängt bei Fehlschlag die Daemon-Logs an den Bericht (QA-007 AC3). */
export async function attachLogs(daemon: Daemon, testInfo: TestInfo): Promise<void> {
  if (testInfo.status === testInfo.expectedStatus) return
  const dir = daemon.logsDir()
  if (!existsSync(dir)) return
  const target = testInfo.outputPath('daemon-logs')
  cpSync(dir, target, { recursive: true })
  for (const f of readdirSync(target)) {
    await testInfo.attach(`daemon-logs/${f}`, { path: join(target, f) })
  }
}

export const test = base.extend<{ daemonLogs: void }, { daemon: Daemon }>({
  daemon: [
    // oxlint-disable-next-line no-empty-pattern -- Playwright verlangt hier das Objektmuster.
    async ({}, use) => {
      const d = await Daemon.start()
      await use(d)
      d.stop()
    },
    { scope: 'worker' },
  ],
  daemonLogs: [
    async ({ daemon }, use, testInfo) => {
      await use()
      await attachLogs(daemon, testInfo)
    },
    { auto: true },
  ],
})

export { expect } from '@playwright/test'
