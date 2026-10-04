import type { ReactNode } from 'react'
import { ChevronDown, Minus, Plus, Square, X } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { Block, Cursor, Glyph, KeyBar, SessionListBlock, TitleBar, Tui, Voice } from './parts'

/*
 * Gruppe „Terminal (TUI)“: `beton tui` mit ratatui. Eigenes PTY-Multiplexing statt tmux,
 * Vendor-TUIs im Pane, Freigaben per Taste. Prefix ist Ctrl+G (^G).
 */

const KEYS_MAIN: [string, string][] = [
  ['^G', 'Prefix'],
  ['Tab', 'Fokus'],
  ['↵', 'Senden'],
  ['Esc', 'Unterbrechen'],
  ['^G s', 'Session'],
  ['^G a', 'Freigaben'],
  ['^G ?', 'Hilfe'],
  ['^G q', 'Beenden'],
]

function Line({ children, className }: { children?: ReactNode; className?: string }) {
  return <div className={cn('whitespace-pre-wrap', className)}>{children ?? ' '}</div>
}

function Dim({ children }: { children: ReactNode }) {
  return <span className="text-muted-foreground">{children}</span>
}

/** Tool-Call als Zeile mit festen Spalten (Name, Ziel, Ergebnis). */
function ToolLine({ name, target, result, dim, open, waiting }: { name: string; target: string; result: string; dim?: boolean; open?: boolean; waiting?: boolean }) {
  return (
    <div className={cn('grid grid-cols-[4ch_12ch_minmax(0,1fr)_20ch] whitespace-pre', dim && 'text-muted-foreground')}>
      <span>{open ? '  ▾' : '  ▸'}</span>
      <span>{name}</span>
      <span className="truncate">{target}</span>
      <span className={waiting ? 'text-signal' : 'text-ok'}>{result}</span>
    </div>
  )
}

/** Verlauf der Rate-Limiter-Session im TUI-Rendering. */
function ChatLines({ streaming }: { streaming?: boolean }) {
  return (
    <>
      <Line>
        <span className="font-semibold">Ingo ▸</span> Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit
        Tests.
      </Line>
      <Line />
      <Line>
        <Voice v="claude">Claude Code ▸</Voice> Ich lege eine Express-Middleware mit Token-Bucket pro IP an.
      </Line>
      <ToolLine dim name="Suche" target="rg 'auth.post' src/" result="✓ 0,2 s" />
      <ToolLine dim name="Lesen" target="src/routes/auth.ts" result="✓ 0,1 s" />
      <ToolLine open name="Bearbeiten" target="src/routes/auth.ts (+2 −1)" result="✓ 0,3 s" />
      <Line className="text-ok">{"      + import { rateLimit } from '../middleware/rate-limit'"}</Line>
      <Line className="text-deny">{"      − auth.post('/login', login)"}</Line>
      <Line className="text-ok">{"      + auth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), login)"}</Line>
      <ToolLine dim name="Shell" target="pnpm vitest run auth" result="✓ 4,8 s · 10 Tests" />
      <Line />
      <Line>
        <Voice v="claude">Claude Code ▸</Voice> Die Login-Route ist auf 5 Versuche pro Minute und IP begrenzt; danach kommt 429 mit Retry-After.
        {streaming ? (
          <>
            {' '}Ich schreibe noch den Test für den Header <Cursor />
          </>
        ) : (
          ' Soll ich pushen und einen PR öffnen?'
        )}
      </Line>
      {!streaming && (
        <>
          <Line />
          <Line>
            <span className="font-semibold">Ingo ▸</span> Ja, push und PR.
          </Line>
          <ToolLine name="Shell" target="git push -u origin beton/rate-limiter-7f3k" result="◆ wartet auf dich" waiting />
        </>
      )}
    </>
  )
}

function Rail({ approvals }: { approvals: number }) {
  return (
    <Block title="Rail" className="w-[32ch] shrink-0">
      <div className="font-semibold">Änderungen (2)</div>
      <div>
        <span className="text-voice-codex">M</span> src/routes/auth.ts <Dim>+2 −1</Dim>
      </div>
      <div>
        <span className="text-ok">A</span> src/middleware/rate-li… <Dim>+12</Dim>
      </div>
      <div className="mt-2 font-semibold">Terminals</div>
      <div>
        <Glyph status="running" /> 1 dev-server <Dim>:5173</Dim>
      </div>
      <div className="mt-2 font-semibold">Freigaben</div>
      {approvals > 0 ? (
        <div>
          <Glyph status="waiting" /> <span className="text-signal">{approvals} offen</span> <Dim>^G a</Dim>
        </div>
      ) : (
        <Dim>keine offen</Dim>
      )}
    </Block>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Session-Ansicht                                                                             */
/* ------------------------------------------------------------------------------------------ */

function TuiSession({ state }: { state: string }) {
  if (state === 'too-small') {
    return (
      <Tui className="items-center justify-center">
        <F id="TUI-001" className="w-[52ch] border border-foreground/40 p-3">
          <Line className="font-semibold">Terminal zu klein: 64 × 18</Line>
          <Line>beton tui braucht mindestens 80 × 24 Zeichen.</Line>
          <Line>
            <Dim>Vergrößere das Fenster – die Session läuft weiter.</Dim>
          </Line>
        </F>
      </Tui>
    )
  }
  const approval = state === 'approval'
  const other = state === 'other-approval'
  return (
    <Tui>
      <TitleBar left="local · shop-frontend" right="09:41" />
      <F id="TUI-001" className="flex min-h-0 flex-1 gap-1">
        <SessionListBlock active="ses_7f3k" waitingOther={other} />
        <Block
          title={
            <>
              Session: Rate-Limiter für die Login-API (<Voice v="claude">claude</Voice> · claude-opus-5-5)
            </>
          }
          right={approval ? <span className="text-signal">◆ wartet auf dich</span> : state === 'streaming' ? '● läuft' : '○ bereit'}
          focused
          className="relative min-w-0 flex-1"
        >
          <ChatLines streaming={state === 'streaming' || other} />
          {approval && (
            <F id="TUI-004" className="absolute inset-x-6 bottom-4">
              <Block title="Freigabe nötig ─ wartet auf dich" accent="signal" className="bg-sunken">
                <div className="grid grid-cols-[9ch_1fr] gap-x-2">
                  <Dim>Tool</Dim>
                  <span>Shell</span>
                  <Dim>Befehl</Dim>
                  <span className="font-semibold">git push -u origin beton/rate-limiter-7f3k</span>
                  <Dim>Regel</Dim>
                  <span>git-push-fragen (Projekt shop-frontend)</span>
                  <Dim>Grund</Dim>
                  <span>Pushes verlassen deinen Rechner. Die Projekt-Policy verlangt dafür deine Freigabe.</span>
                </div>
                <Line />
                <Line>
                  <span className="bg-signal px-1 font-semibold text-signal-foreground">y</span> erlauben{'   '}
                  <span className="bg-foreground/85 px-1 text-background">n</span> ablehnen{'   '}
                  <span className="bg-foreground/85 px-1 text-background">s</span> für diese Session erlauben{'   '}
                  <span className="bg-foreground/85 px-1 text-background">d</span> Details
                </Line>
              </Block>
            </F>
          )}
        </Block>
        <Rail approvals={approval ? 1 : 0} />
      </F>
      <F id="TUI-001" className="mt-2.5 shrink-0">
        <Block title="Nachricht" focused={!approval}>
          <Line>
            <span className="font-semibold">&gt;</span> {state === 'streaming' ? <Dim>Danach bitte auch die Register-Route absichern</Dim> : null}
            {!approval && <Cursor />}
          </Line>
        </Block>
      </F>
      <div className="flex shrink-0 gap-3 px-1 pt-1">
        {state === 'streaming' && <span>Queue: 1</span>}
        <span>Kontext 36 %</span>
        <span>Claude Max 38 %</span>
        {other && (
          <F id="TUI-004" as="span" badge="top-left">
            <span className="text-signal">◆ 1 Freigabe in „Review: Rate-Limiter“ · ^G a springt hin</span>
          </F>
        )}
        <span className="ml-auto text-muted-foreground">{approval ? 'y/n/s/d entscheiden · Esc zurück zum Verlauf' : '^G ? Hilfe'}</span>
      </div>
      <KeyBar keys={KEYS_MAIN} />
    </Tui>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Panes & Multiplexing                                                                        */
/* ------------------------------------------------------------------------------------------ */

function ViteLines() {
  return (
    <>
      <Line>
        <span className="text-ok">VITE v7.1.4</span> <Dim>ready in 412 ms</Dim>
      </Line>
      <Line />
      <Line>
        {'  ➜  Local:   '}
        <span className="text-voice-codex">http://localhost:5173/</span>
      </Line>
      <Line>
        <Dim>{'  ➜  Network: use --host to expose'}</Dim>
      </Line>
      <Line>
        <Dim>09:41:02</Dim> <span className="text-voice-codex">[vite]</span> hmr update /src/routes/auth.ts
      </Line>
      <Line>
        <Dim>09:41:07</Dim> <span className="text-voice-codex">[vite]</span> hmr update /src/middleware/rate-limit.ts
      </Line>
      <Line>
        <Cursor />
      </Line>
    </>
  )
}

function Bar({ pct, cls = 'text-ok' }: { pct: number; cls?: string }) {
  const n = Math.round(pct / 4)
  return (
    <>
      [<span className={cls}>{'|'.repeat(n)}</span>
      {' '.repeat(25 - n)}
      <Dim>{pct.toFixed(1).padStart(5)}%</Dim>]
    </>
  )
}

function HtopLines() {
  return (
    <>
      <Line>
        {'  1 '}
        <Bar pct={42.1} />
        {'   Tasks: 312, 1204 thr; 3 running'}
      </Line>
      <Line>
        {'  2 '}
        <Bar pct={21.4} />
        {'   Load average: 2.31 1.98 1.74'}
      </Line>
      <Line>
        {'  3 '}
        <Bar pct={61.0} />
        {'   Uptime: 3 days, 04:12:09'}
      </Line>
      <Line>
        {'Mem '}
        <Bar pct={35.0} cls="text-voice-codex" />
        {'   11.2G / 32.0G'}
      </Line>
      <Line />
      <Line className="bg-ok text-background">{'  PID USER       CPU%  MEM%  Command                              '}</Line>
      <Line>{'48121 ingo       12.4   1.1  node …/vite/bin/vite.js'}</Line>
      <Line>{'48302 ingo        8.1   2.3  claude --output-format stream-json'}</Line>
      <Line>{'47710 ingo        3.0   0.4  beton host'}</Line>
      <Line>{'47702 ingo        1.2   0.6  beton serve'}</Line>
    </>
  )
}

function TuiPanes({ state }: { state: string }) {
  const zoom = state === 'zoom'
  return (
    <Tui>
      <TitleBar left="Rate-Limiter für die Login-API · 3 Panes" right={zoom ? '[Z] 2 htop' : '09:41'} />
      {state === 'reattach' && (
        <F id="TUI-002" className="mb-2 shrink-0 px-1">
          <span className="text-ok">✓ Wieder verbunden.</span> 3 Panes wiederhergestellt; dev-server und htop liefen im Runner weiter (seit 42 Min.)
          · Scrollback 4.812 Zeilen.
        </F>
      )}
      <F id="TUI-002" className="relative flex min-h-0 flex-1 gap-1">
        {zoom ? (
          <Block title="[Z] 2 htop" right="^G z zurück" focused className="flex-1">
            <HtopLines />
          </Block>
        ) : (
          <>
            <Block title={<>0 Agent · <Voice v="claude">Claude Code</Voice></>} className="min-w-0 flex-1">
              <ChatLines streaming />
            </Block>
            <div className="flex w-[52%] min-w-0 flex-col gap-2.5">
              <Block title="1 dev-server · pnpm dev" right="PTY 96×14" focused className="flex-1">
                <ViteLines />
              </Block>
              <Block title="2 htop" right="PTY 96×14" className="flex-1">
                <HtopLines />
              </Block>
            </div>
          </>
        )}
        {state === 'prefix' && (
          <div className="absolute right-3 bottom-3 z-10">
            <Block title="^G gedrückt – nächste Taste" accent="signal" className="w-[46ch] bg-sunken">
              {[
                ['|', 'senkrecht teilen'],
                ['-', 'waagerecht teilen'],
                ['o', 'nächster Pane'],
                ['z', 'Pane zoomen'],
                ['c', 'neues Terminal'],
                ['x', 'Pane schließen'],
                ['d', 'trennen (alles läuft weiter)'],
                ['s', 'Session wechseln'],
                ['a', 'zur nächsten Freigabe'],
                ['^G', '^G an den Pane senden'],
              ].map(([k, l]) => (
                <Line key={k}>
                  <span className="inline-block w-[4ch] text-signal">{k}</span>
                  {l}
                </Line>
              ))}
            </Block>
          </div>
        )}
      </F>
      <div className="flex shrink-0 gap-3 px-1 pt-1.5">
        <span>Panes laufen im Runner (beton-pty) – kein tmux</span>
        <span className="ml-auto text-muted-foreground">Layout wird pro Session gespeichert</span>
      </div>
      <KeyBar
        keys={[
          ['^G |', 'teilen'],
          ['^G -', 'teilen'],
          ['^G o', 'nächster'],
          ['^G z', 'zoomen'],
          ['^G c', 'Terminal'],
          ['^G d', 'trennen'],
          ['^G ?', 'Hilfe'],
        ]}
      />
    </Tui>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Native-TUI-Modus                                                                            */
/* ------------------------------------------------------------------------------------------ */

function VendorTui({ waiting }: { waiting?: boolean }) {
  return (
    <>
      <Line className="text-voice-claude">╭──────────────────────────────────────────────╮</Line>
      <Line className="text-voice-claude">
        │ <span className="text-foreground">✻ Welcome to Claude Code</span>                     │
      </Line>
      <Line className="text-voice-claude">
        │ <Dim>  cwd: ~/code/shop-frontend</Dim>                  │
      </Line>
      <Line className="text-voice-claude">╰──────────────────────────────────────────────╯</Line>
      <Line />
      <Line>&gt; Teste die Checkout-Validierung und push danach den Branch.</Line>
      <Line />
      <Line>
        <span className="text-ok">⏺</span> Bash(pnpm vitest run checkout)
      </Line>
      <Line>
        <Dim>  ⎿  ✓ checkout/validation.spec.ts (12 tests)</Dim>
      </Line>
      <Line />
      <Line>
        <span className={waiting ? 'text-signal' : 'text-ok'}>⏺</span> Bash(git push -u origin beton/checkout-a11y-6q2a)
      </Line>
      <Line>
        <Dim>  ⎿  {waiting ? 'Waiting for permission…' : 'branch pushed'}</Dim>
      </Line>
      <Line />
      <Line className="text-muted-foreground">╭──────────────────────────────────────────────╮</Line>
      <Line className="text-muted-foreground">
        │ <span className="text-foreground">&gt;</span> <Cursor />                                          │
      </Line>
      <Line className="text-muted-foreground">╰──────────────────────────────────────────────╯</Line>
      <Line>
        <Dim>  ? for shortcuts</Dim>
      </Line>
    </>
  )
}

function TuiNative({ state }: { state: string }) {
  const approval = state === 'approval'
  return (
    <Tui>
      <TitleBar left="Checkout-Formular barrierefrei machen · Native TUI" right="auch im Web-Terminal offen: 1 Client" />
      <F id="TUI-003" className="flex min-h-0 flex-1 gap-1">
        <SessionListBlock active="ses_6q2a" />
        <div className="flex min-w-0 flex-1 flex-col gap-2.5">
          {approval && (
            <F id="TUI-004" className="shrink-0">
              <Block title="Freigabe aus Claude-Code-Hook ─ wartet auf dich" accent="signal">
                <Line>
                  Bash(<span className="font-semibold">git push -u origin beton/checkout-a11y-6q2a</span>) <Dim>· Regel git-push-fragen</Dim>
                </Line>
                <Line>
                  <span className="bg-signal px-1 font-semibold text-signal-foreground">^G y</span> erlauben{'   '}
                  <span className="bg-foreground/85 px-1 text-background">^G n</span> ablehnen{'   '}
                  <span className="bg-foreground/85 px-1 text-background">^G d</span> Details
                  <Dim>{'   '}– ohne ^G gehen Tasten an Claude Code</Dim>
                </Line>
              </Block>
            </F>
          )}
          <Block
            title={
              <>
                <Voice v="claude">Claude Code</Voice> · eigenes TUI im Pane
              </>
            }
            right="alle Tasten gehen an Claude Code, außer ^G"
            focused
            className="relative min-w-0 flex-1"
          >
            <VendorTui waiting={approval} />
            {state === 'prefix' && (
              <div className="absolute right-3 bottom-3">
                <Block title="^G – beton hört zu" accent="signal" className="w-[44ch] bg-sunken">
                  {[
                    ['s', 'Session wechseln'],
                    ['a', 'Freigaben anzeigen'],
                    ['y / n', 'offene Freigabe entscheiden'],
                    ['d', 'trennen (Claude Code läuft weiter)'],
                    ['^G', '^G an Claude Code senden'],
                    ['Esc', 'zurück, nichts tun'],
                  ].map(([k, l]) => (
                    <Line key={k}>
                      <span className="inline-block w-[6ch] text-signal">{k}</span>
                      {l}
                    </Line>
                  ))}
                </Block>
              </div>
            )}
          </Block>
        </div>
      </F>
      <div className="flex shrink-0 gap-3 px-1 pt-1.5">
        <span>Esc, Shift+Tab und Einfügen gehen unverändert an Claude Code</span>
        <span className="ml-auto text-muted-foreground">PTY 120×32 · Modus native</span>
      </div>
      <KeyBar
        keys={[
          ['^G', 'Prefix'],
          ['^G s', 'Session'],
          ['^G a', 'Freigaben'],
          ['^G d', 'trennen'],
          ['^G ?', 'Hilfe'],
        ]}
      />
    </Tui>
  )
}

/* ------------------------------------------------------------------------------------------ */
/* Windows                                                                                     */
/* ------------------------------------------------------------------------------------------ */

function WindowsTerminal({ children }: { children: ReactNode }) {
  return (
    <div className="dark flex h-full min-h-[600px] flex-col overflow-hidden rounded-md border border-border bg-sunken text-foreground shadow-lg">
      <div className="flex h-9 shrink-0 items-end gap-px bg-background pl-2 text-[12px]">
        <span className="flex h-8 items-center gap-2 rounded-t-md px-3 text-muted-foreground">PowerShell</span>
        <span className="flex h-8 items-center gap-2 rounded-t-md bg-sunken px-3">beton tui</span>
        <span className="flex h-8 items-center px-2 text-muted-foreground">
          <Plus className="size-3.5" />
        </span>
        <span className="flex h-8 items-center px-1 text-muted-foreground">
          <ChevronDown className="size-3.5" />
        </span>
        <span className="ml-auto flex h-full">
          {[Minus, Square, X].map((I, i) => (
            <span key={i} className="flex w-11 items-center justify-center text-muted-foreground">
              <I className="size-3.5" />
            </span>
          ))}
        </span>
      </div>
      <div className="min-h-0 flex-1 p-4">{children}</div>
    </div>
  )
}

function TuiWindows() {
  return (
    <WindowsTerminal>
      <Tui className="m-0 h-full">
        <TitleBar left="local · shop-frontend · Windows" right="Windows Terminal · ConPTY" />
        <F id="TUI-005" className="flex min-h-0 flex-1 gap-1">
          <Block title={<>0 Agent · <Voice v="claude">Claude Code</Voice></>} className="min-w-0 flex-1">
            <Line>
              <span className="font-semibold">Ingo ▸</span> Lass die Tests laufen.
            </Line>
            <ToolLine name="Shell" target="pnpm vitest run auth" result="◆ Freigabe" waiting />
            <Line>
              <span className="text-ok">  ✓ Erlaubt mit y</span> <Dim>· Ingo, TUI · 09:41:12</Dim>
            </Line>
            <ToolLine name="Shell" target="pnpm vitest run auth" result="✓ 5,2 s" />
            <Line />
            <Line>
              <Voice v="claude">Claude Code ▸</Voice> Alle 10 Tests laufen.
            </Line>
          </Block>
          <Block title="1 PowerShell 7.5" right="PTY 88×20" focused className="w-[52%] min-w-0">
            <Line>
              <span className="text-voice-codex">PS C:\Users\ingo\code\shop-frontend&gt;</span> pnpm vitest run auth
            </Line>
            <Line />
            <Line>
              <span className="text-ok"> ✓</span> auth/login.spec.ts <Dim>(6)</Dim>
            </Line>
            <Line>
              <span className="text-ok"> ✓</span> middleware/rate-limit.spec.ts <Dim>(4)</Dim>
            </Line>
            <Line />
            <Line>
              {' Test Files  '}
              <span className="text-ok">2 passed</span> <Dim>(2)</Dim>
            </Line>
            <Line>
              {'      Tests  '}
              <span className="text-ok">10 passed</span> <Dim>(10)</Dim>
            </Line>
            <Line />
            <Line>
              <span className="text-voice-codex">PS C:\Users\ingo\code\shop-frontend&gt;</span> <Cursor />
            </Line>
          </Block>
        </F>
        <div className="flex shrink-0 gap-3 px-1 pt-1.5">
          <span>Kontext 22 %</span>
          <span className="ml-auto text-muted-foreground">crossterm · Panes über ConPTY</span>
        </div>
        <KeyBar keys={KEYS_MAIN} />
      </Tui>
    </WindowsTerminal>
  )
}

export const group: ScreenGroup = {
  id: 'tui',
  title: 'Terminal (TUI)',
  order: 200,
  screens: [
    {
      id: 'tui-session',
      title: 'Session-Ansicht',
      description:
        'beton tui zeigt Session-Liste, Verlauf, Rail und Eingabe im Terminal – mit denselben Events wie Web und Desktop. Freigaben entscheidest du mit einer Taste.',
      features: ['TUI-001', 'TUI-004'],
      states: [
        { id: 'streaming', title: 'Agent arbeitet' },
        { id: 'approval', title: 'Freigabe offen' },
        { id: 'other-approval', title: 'Freigabe in anderer Session' },
        { id: 'too-small', title: 'Terminal zu klein' },
      ],
      frame: 'terminal',
      component: TuiSession,
    },
    {
      id: 'tui-panes',
      title: 'Panes & Multiplexing',
      description:
        'Terminals teilen, zoomen und wechseln – ohne tmux. Die PTYs laufen im Runner weiter, auch wenn du die TUI schließt; beim nächsten Öffnen ist alles wieder da.',
      features: ['TUI-002'],
      states: [
        { id: 'split', title: 'Geteilt' },
        { id: 'prefix', title: 'Prefix ^G' },
        { id: 'zoom', title: 'Gezoomt' },
        { id: 'reattach', title: 'Wieder verbunden' },
      ],
      frame: 'terminal',
      component: TuiPanes,
    },
    {
      id: 'tui-native',
      title: 'Native-TUI-Modus',
      description:
        'Sessions im Native-TUI-Modus zeigen das Original-TUI des Harness im Pane. Alle Tasten gehen dorthin; beton erreichst du über ^G – auch Freigaben aus Vendor-Hooks.',
      features: ['TUI-003', 'TUI-004'],
      states: [
        { id: 'native', title: 'Vendor-TUI' },
        { id: 'approval', title: 'Freigabe als Banner' },
        { id: 'prefix', title: 'Prefix ^G' },
      ],
      frame: 'terminal',
      component: TuiNative,
    },
    {
      id: 'tui-windows',
      title: 'TUI auf Windows',
      description: 'Dieselbe TUI in Windows Terminal: Panes über ConPTY, PowerShell im Pane, Freigaben per Taste.',
      features: ['TUI-005'],
      frame: 'none',
      component: TuiWindows,
    },
  ],
}
