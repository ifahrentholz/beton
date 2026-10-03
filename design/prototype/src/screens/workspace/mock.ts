// Beispieldaten der Gruppe Workspace (Session „Rate-Limiter für die Login-API“ in shop-frontend).
import type { DiffLine } from '@/mock/data'

export const repo = {
  name: 'shop-frontend',
  root: '~/code/shop-frontend',
  worktree: '~/.beton/worktrees/shop-frontend-3fa2c1d9/rate-limiter-7f3k',
  branch: 'beton/rate-limiter-7f3k',
  base: 'main',
}

export type TreeNode = {
  name: string
  depth: number
  dir?: boolean
  open?: boolean
  /** Git-Status: A = neu, M = geändert, D = gelöscht. */
  status?: 'A' | 'M' | 'D'
  active?: boolean
  /** Symlink aus dem Workspace hinaus (nicht lesbar). */
  outside?: string
}

export const tree: TreeNode[] = [
  { name: '.beton', depth: 0, dir: true },
  { name: 'node_modules', depth: 0, dir: true },
  { name: 'public', depth: 0, dir: true },
  { name: 'src', depth: 0, dir: true, open: true },
  { name: 'controllers', depth: 1, dir: true },
  { name: 'middleware', depth: 1, dir: true, open: true },
  { name: 'cors.ts', depth: 2 },
  { name: 'rate-limit.spec.ts', depth: 2, status: 'A' },
  { name: 'rate-limit.ts', depth: 2, status: 'A', active: true },
  { name: 'routes', depth: 1, dir: true, open: true },
  { name: 'auth.ts', depth: 2, status: 'M' },
  { name: 'checkout.ts', depth: 2 },
  { name: 'server.ts', depth: 1 },
  { name: '.cache', depth: 0, outside: '/Volumes/Scratch/shop-cache' },
  { name: '.env.example', depth: 0 },
  { name: 'package.json', depth: 0 },
  { name: 'README.md', depth: 0 },
  { name: 'tsconfig.json', depth: 0 },
]

export const rateLimitSource = [
  "import type { RequestHandler } from 'express'",
  '',
  "type Options = { window: '1m' | '10m' | '1h'; max: number; key: 'ip' | 'user' }",
  'type Bucket = { tokens: number; updated: number }',
  '',
  "const WINDOW_MS = { '1m': 60_000, '10m': 600_000, '1h': 3_600_000 } as const",
  '',
  'export function rateLimit(opts: Options): RequestHandler {',
  '  const buckets = new Map<string, Bucket>()',
  '  const refillPerMs = opts.max / WINDOW_MS[opts.window]',
  '',
  '  return (req, res, next) => {',
  "    const id = opts.key === 'ip' ? req.ip : req.user?.id",
  '    const now = Date.now()',
  '    const b = buckets.get(id) ?? { tokens: opts.max, updated: now }',
  '    b.tokens = Math.min(opts.max, b.tokens + (now - b.updated) * refillPerMs)',
  '    b.updated = now',
  '    if (b.tokens < 1) {',
  "      res.set('Retry-After', String(Math.ceil((1 - b.tokens) / refillPerMs / 1000)))",
  "      return res.status(429).json({ error: 'too_many_requests' })",
  '    }',
  '    b.tokens -= 1',
  '    buckets.set(id, b)',
  '    next()',
  '  }',
  '}',
]

export const readmeSource = [
  '# shop-frontend',
  '',
  'Storefront und Checkout für den Shop. Express-Backend unter `src/`.',
  '',
  '## Entwicklung',
  '',
  '```bash',
  'pnpm install',
  'pnpm dev        # http://localhost:5173',
  'pnpm vitest',
  '```',
  '',
  '## Rate-Limits',
  '',
  '- Login: 5 Versuche pro Minute und IP',
]

export type ChangedFile = { path: string; status: 'A' | 'M' | 'D'; add: number; del: number }

export const uncommitted: ChangedFile[] = [
  { path: 'src/routes/auth.ts', status: 'M', add: 2, del: 1 },
  { path: 'src/middleware/rate-limit.ts', status: 'A', add: 26, del: 0 },
  { path: 'src/middleware/rate-limit.spec.ts', status: 'A', add: 48, del: 0 },
]

export const branchFiles: ChangedFile[] = [
  ...uncommitted,
  { path: 'README.md', status: 'M', add: 4, del: 0 },
  { path: 'src/server.ts', status: 'M', add: 1, del: 1 },
]

export const turnFiles: ChangedFile[] = [{ path: 'src/middleware/rate-limit.spec.ts', status: 'A', add: 48, del: 0 }]

export const specDiff: DiffLine[] = [
  { kind: 'add', n: 1, text: "import { describe, expect, it, vi } from 'vitest'" },
  { kind: 'add', n: 2, text: "import { rateLimit } from './rate-limit'" },
  { kind: 'add', n: 3, text: '' },
  { kind: 'add', n: 4, text: "describe('rateLimit', () => {" },
  { kind: 'add', n: 5, text: "  it('lässt 5 Anfragen pro Minute durch', () => {" },
  { kind: 'add', n: 6, text: "    const mw = rateLimit({ window: '1m', max: 5, key: 'ip' })" },
  { kind: 'add', n: 7, text: '    const { req, res, next } = fakeHttp()' },
  { kind: 'add', n: 8, text: '    for (let i = 0; i < 5; i++) mw(req, res, next)' },
  { kind: 'add', n: 9, text: '    expect(next).toHaveBeenCalledTimes(5)' },
  { kind: 'add', n: 10, text: '  })' },
  { kind: 'add', n: 11, text: '' },
  { kind: 'add', n: 12, text: "  it('antwortet beim sechsten Versuch mit 429 und Retry-After', () => {" },
]

export const serverDiff: DiffLine[] = [
  { kind: 'ctx', n: 8, text: "app.use(express.json({ limit: '1mb' }))" },
  { kind: 'del', n: 9, text: "app.set('trust proxy', false)" },
  { kind: 'add', n: 9, text: "app.set('trust proxy', 1) // req.ip hinter dem Load-Balancer" },
  { kind: 'ctx', n: 10, text: "app.use('/auth', auth)" },
]

export const worktrees = [
  {
    branch: 'beton/rate-limiter-7f3k',
    session: 'Rate-Limiter für die Login-API',
    status: 'waiting' as const,
    path: '~/.beton/worktrees/shop-frontend-3fa2c1d9/rate-limiter-7f3k',
    base: 'main @ 4c1e9a2',
    state: '3 Dateien nicht committet',
    stateTone: 'signal' as const,
    size: '412 MB',
  },
  {
    branch: 'beton/checkout-a11y-6q2a',
    session: 'Checkout-Formular barrierefrei machen',
    status: 'idle' as const,
    path: '~/.beton/worktrees/shop-frontend-3fa2c1d9/checkout-a11y-6q2a',
    base: 'main @ 9be0d17',
    state: '2 Commits nicht gepusht',
    stateTone: 'neutral' as const,
    size: '398 MB',
  },
  {
    branch: 'beton/payment-flaky-6k8e',
    session: 'Flaky Test in payment_spec',
    status: 'failed' as const,
    path: '~/.beton/worktrees/shop-frontend-3fa2c1d9/payment-flaky-6k8e',
    base: 'main @ 9be0d17',
    state: 'Sauber, gemergt in main',
    stateTone: 'ok' as const,
    size: '401 MB',
  },
]
