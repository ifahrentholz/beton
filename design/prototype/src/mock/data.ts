// Beispieldaten für den Prototyp. Realistisch, aber erfunden.

export type HarnessId = 'claude' | 'codex' | 'gemini' | 'ollama'
export type Voice = 'claude' | 'codex' | 'acp' | 'direct' | 'human'

export type Harness = {
  id: HarnessId
  name: string
  voice: Voice
  transport: 'nativ' | 'ACP' | 'PTY' | 'Direkt-API'
  auth: string
  models: string[]
}

export const harnesses: Record<HarnessId, Harness> = {
  claude: {
    id: 'claude',
    name: 'Claude Code',
    voice: 'claude',
    transport: 'nativ',
    auth: 'Claude Max · über claude-CLI angemeldet',
    models: ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-4-5'],
  },
  codex: {
    id: 'codex',
    name: 'Codex',
    voice: 'codex',
    transport: 'nativ',
    auth: 'ChatGPT Pro · über codex-CLI angemeldet',
    models: ['gpt-5.3-codex', 'gpt-5.3'],
  },
  gemini: {
    id: 'gemini',
    name: 'Gemini CLI',
    voice: 'acp',
    transport: 'ACP',
    auth: 'Google-Konto · über gemini-CLI angemeldet',
    models: ['gemini-3-pro'],
  },
  ollama: {
    id: 'ollama',
    name: 'Ollama (lokal)',
    voice: 'direct',
    transport: 'Direkt-API',
    auth: 'lokal · kein Schlüssel nötig',
    models: ['qwen3-coder:30b'],
  },
}

export type SessionStatus = 'running' | 'waiting' | 'idle' | 'failed' | 'stopped'

export type SessionSummary = {
  id: string
  title: string
  project: string
  harness: HarnessId
  status: SessionStatus
  branch?: string
  updated: string
  unread?: boolean
  shared?: boolean
  async?: boolean
}

export const projects = [
  { id: 'beton', name: 'beton', path: '~/Develop/ai/beton' },
  { id: 'shop', name: 'shop-frontend', path: '~/code/shop-frontend' },
  { id: 'infra', name: 'infra', path: '~/code/infra' },
]

export const sessions: SessionSummary[] = [
  { id: 'ses_7f3k', title: 'Rate-Limiter für die Login-API', project: 'shop', harness: 'claude', status: 'waiting', branch: 'beton/rate-limiter-7f3k', updated: 'jetzt', unread: true },
  { id: 'ses_7f3m', title: 'Review: Rate-Limiter', project: 'shop', harness: 'codex', status: 'running', branch: 'beton/rate-limiter-7f3k', updated: 'vor 1 Min.' },
  { id: 'ses_6q2a', title: 'Checkout-Formular barrierefrei machen', project: 'shop', harness: 'claude', status: 'idle', branch: 'beton/checkout-a11y-6q2a', updated: 'vor 12 Min.', shared: true },
  { id: 'ses_6p9z', title: 'Event-Log: seq lückenlos halten', project: 'beton', harness: 'codex', status: 'idle', branch: 'wp-03-persistence', updated: 'vor 1 Std.' },
  { id: 'ses_6n1c', title: 'Nächtliches Dependency-Update', project: 'infra', harness: 'claude', status: 'running', updated: 'vor 2 Std.', async: true },
  { id: 'ses_6m4d', title: 'Terraform-Plan erklären', project: 'infra', harness: 'gemini', status: 'stopped', updated: 'gestern' },
  { id: 'ses_6k8e', title: 'Flaky Test in payment_spec', project: 'shop', harness: 'claude', status: 'failed', updated: 'gestern' },
  { id: 'ses_6h2f', title: 'Lokales Modell testen', project: 'beton', harness: 'ollama', status: 'stopped', updated: 'Mo.' },
]

export const currentSession = sessions[0]

export type DiffLine = { kind: 'ctx' | 'add' | 'del'; text: string; n?: number }

export const rateLimiterDiff: DiffLine[] = [
  { kind: 'ctx', n: 12, text: "import { Router } from 'express'" },
  { kind: 'add', n: 13, text: "import { rateLimit } from '../middleware/rate-limit'" },
  { kind: 'ctx', n: 14, text: '' },
  { kind: 'ctx', n: 15, text: 'export const auth = Router()' },
  { kind: 'del', n: 16, text: "auth.post('/login', login)" },
  { kind: 'add', n: 16, text: "auth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), login)" },
]

export const usage = {
  context: { used: 71_400, window: 200_000 },
  subscription: { label: 'Claude Max', windowUsedPct: 38, resetsIn: '2 Std. 14 Min.' },
  costEur: 0,
}
