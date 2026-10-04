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

export type InboxKind = 'approval' | 'question' | 'async_done' | 'async_failed' | 'mention' | 'notice'
export type InboxItem = {
  id: string
  kind: InboxKind
  title: string
  /** Titel ist ein Befehl (Monospace). */
  mono?: boolean
  session: string
  project: string
  harness: HarnessId
  when: string
  body?: string
  paused?: boolean
  later?: string
  feature?: string
}

/** Offene Inbox-Einträge: Freigaben und Fragen, die auf dich warten. */
export const inboxOpen: InboxItem[] = [
  {
    id: 'inb_1',
    kind: 'approval',
    title: 'git push -u origin beton/rate-limiter-7f3k',
    mono: true,
    session: 'Rate-Limiter für die Login-API',
    project: 'shop-frontend',
    harness: 'claude',
    when: 'vor 2 Min.',
    body: 'Pushes verlassen deinen Rechner. Regel git-push-fragen (Projekt shop-frontend) verlangt deine Freigabe.',
  },
  {
    id: 'inb_2',
    kind: 'question',
    title: 'ESLint 10 bricht 3 Regeln – anpassen oder bei 9 bleiben?',
    session: 'Nächtliches Dependency-Update',
    project: 'infra',
    harness: 'claude',
    when: 'vor 41 Min.',
    paused: true,
    body: 'Das Update von eslint 9.31 auf 10.0 meldet 3 entfernte Regeln in .eslintrc. Ich kann sie durch die Nachfolger ersetzen oder das Update auslassen.',
  },
]

/** Zur Info: Hinweise, fertige oder fehlgeschlagene Hintergrund-Agents, Erwähnungen. */
export const inboxFyi: InboxItem[] = [
  { id: 'inb_3', kind: 'notice', title: 'Claude Max: 92 % des 5-Stunden-Fensters genutzt', session: 'Kontingent', project: '—', harness: 'claude', when: 'vor 6 Min.', body: 'Setzt um 14:46 zurück. Neue Turns mit Claude Code können bis dahin abgelehnt werden.', feature: 'USE-004' },
  { id: 'inb_4', kind: 'async_failed', title: 'Flaky Test in payment_spec – nach 3 Versuchen abgebrochen', session: 'Flaky Test in payment_spec', project: 'shop-frontend', harness: 'claude', when: 'gestern 23:10', later: 'ab M5' },
  { id: 'inb_5', kind: 'mention', title: 'Mara: „@ingo schaust du dir den Fokus-Trap im Dialog an?“', session: 'Checkout-Formular barrierefrei machen', project: 'shop-frontend', harness: 'claude', when: 'vor 1 Std.', later: 'ab M4' },
  { id: 'inb_6', kind: 'async_done', title: 'Docs-Linkcheck fertig – 2 tote Links, PR #482 geöffnet', session: 'Docs-Linkcheck (geplant)', project: 'beton', harness: 'codex', when: 'heute 06:00', later: 'ab M5' },
]

export const inboxDone: (InboxItem & { resolution: string })[] = [
  { id: 'inb_7', kind: 'approval', title: 'pnpm add -D vitest-axe', mono: true, session: 'Checkout-Formular barrierefrei machen', project: 'shop-frontend', harness: 'claude', when: 'vor 3 Std.', resolution: 'Erlaubt von dir, am Handy' },
  { id: 'inb_8', kind: 'approval', title: 'curl https://api.stripe.com/v1/charges', mono: true, session: 'Flaky Test in payment_spec', project: 'shop-frontend', harness: 'claude', when: 'gestern', resolution: 'Nach 30 Min. ohne Antwort abgelehnt (Timeout)' },
  { id: 'inb_9', kind: 'question', title: 'Soll der Limiter pro IP oder pro Account zählen?', session: 'Rate-Limiter für die Login-API', project: 'shop-frontend', harness: 'codex', when: 'gestern', resolution: 'Beantwortet von dir: „Pro IP, zusätzlich pro Account bei /login“' },
]

/** Zähler „du bist dran“: offene Freigaben und Fragen (Badge an Inbox, Dock, Menüleiste). */
export const inboxWaitingCount = inboxOpen.filter((i) => i.kind === 'approval' || i.kind === 'question').length

export type DiffLine = { kind: 'ctx' | 'add' | 'del'; text: string; n?: number }

export const rateLimiterDiff: DiffLine[] = [
  { kind: 'ctx', n: 12, text: "import { Router } from 'express'" },
  { kind: 'add', n: 13, text: "import { rateLimit } from '../middleware/rate-limit'" },
  { kind: 'ctx', n: 14, text: '' },
  { kind: 'ctx', n: 15, text: 'export const auth = Router()' },
  { kind: 'del', n: 16, text: "auth.post('/login', login)" },
  { kind: 'add', n: 16, text: "auth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), login)" },
]

/** Kontextfenster einer Session (USE-008). */
export type ContextUsage = { used: number; window: number }

/** Subscription-Fenster eines Harness (USE-004). Ohne Prozentwert meldet der Harness nichts. */
export type Quota = { label: string; windowUsedPct?: number; resetsIn?: string }

/** Kontext der Beispiel-Session. */
export const sampleContext: ContextUsage = { used: 71_400, window: 200_000 }

/** Kontingente je Harness; lokale Modelle (Ollama) haben keins. */
export const quotas: Partial<Record<HarnessId, Quota>> = {
  claude: { label: 'Claude Max', windowUsedPct: 38, resetsIn: '2 Std. 14 Min.' },
  codex: { label: 'ChatGPT Pro', windowUsedPct: 12, resetsIn: '4 Std. 2 Min.' },
  gemini: { label: 'Gemini' },
}
