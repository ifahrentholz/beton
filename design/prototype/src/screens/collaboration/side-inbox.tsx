import type { ReactNode } from 'react'
import { AtSign, Bell, Check, CornerDownRight, Eye, MessageSquareReply, Share2 } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { Composer, WorkspaceRail } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, NetNote, PageHeader, Segmented, SessionShell, ShortStream, Tag } from '@/app/kit/workspace'

/* ---------- Side-Chats ---------- */

function SideChatPanel({ adopted }: { adopted?: boolean }) {
  return (
    <F id="COL-008" className="flex min-h-full flex-col">
      <div className="flex items-center gap-2 border-b border-border px-3 py-2 text-[12px]">
        <span className="rounded-md bg-accent px-2 py-0.5 font-medium">Was macht refillPerMs?</span>
        <span className="px-2 py-0.5 text-muted-foreground">Warum Map statt LRU?</span>
        <span className="ml-auto text-muted-foreground">/side für neue Frage</span>
      </div>
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        <Eye className="size-3" /> Nur lesend: Dateien sehen, nicht ändern · nur für dich sichtbar · zählt zum Budget der Session
      </div>
      <div className="flex flex-1 flex-col gap-3 p-3">
        <UserMessage>Was macht refillPerMs genau, und warum wird da durch WINDOW_MS geteilt?</UserMessage>
        <ToolCall kind="read" name="Lesen" target="src/middleware/rate-limit.ts" duration="0,1 s" />
        <AgentMessage harness="claude">
          <p>
            <code className="rounded-sm bg-muted px-1 text-[13px]">refillPerMs</code> ist die Nachfüllrate des Token-Buckets: bei 5 Versuchen pro Minute 5 / 60.000 ≈ 0,000083
            Tokens pro Millisekunde. Nach 12 Sekunden ist also genau ein Versuch wieder frei.
          </p>
        </AgentMessage>
        <UserMessage>Dann schreib den Kommentar direkt in die Datei.</UserMessage>
        <ToolCall kind="edit" name="Bearbeiten" target="src/middleware/rate-limit.ts" status="denied" sandbox />
        <div className="ml-9 text-[12px] text-muted-foreground">
          Side-Chats ändern keine Dateien. Übernimm die Antwort in die Hauptsession, dann kann der Agent dort schreiben.
        </div>
        {adopted && <SystemNote tone="ok">In die Hauptsession übernommen · 14:20</SystemNote>}
      </div>
      <div className="sticky bottom-0 flex items-center gap-2 border-t border-border bg-background p-3">
        <Btn variant="primary" disabled={adopted}>
          <CornerDownRight className="size-3.5" /> In Hauptsession übernehmen
        </Btn>
        <span className="text-[12px] text-muted-foreground">Reiht die letzte Antwort als Nachricht ein.</span>
      </div>
    </F>
  )
}

export function SideChatScreen({ state }: { state: string }) {
  const adopted = state === 'adopted'
  return (
    <SessionShell
      status="running"
      rail={
        <WorkspaceRail active="side" width="w-[460px]">
          <SideChatPanel adopted={adopted} />
        </WorkspaceRail>
      }
      composer={
        <F id="COL-008">
          <Composer
            harness="claude"
            running
            queued={adopted ? ['Aus Side-Chat „Was macht refillPerMs?“: Kommentar zur Nachfüllrate in rate-limit.ts ergänzen'] : undefined}
          />
        </F>
      }
    >
      <ShortStream />
      <UserMessage>Jetzt bitte auch die Register-Route absichern.</UserMessage>
      <AgentMessage harness="claude" streaming>
        <p>Ich hänge dieselbe Middleware an die Register-Route, mit 3 Versuchen pro</p>
      </AgentMessage>
    </SessionShell>
  )
}

/* ---------- Inbox & Benachrichtigungen ---------- */

type Item = { icon: ReactNode; title: ReactNode; meta: string; action?: ReactNode; waiting?: boolean; done?: boolean }

const open: Item[] = [
  {
    icon: <span className="chamfer-sm block size-2.5 bg-signal" />,
    title: (
      <>
        Freigabe in geteilter Session: <code className="font-mono text-[12px]">git push -u origin beton/checkout-a11y-6q2a</code>
      </>
    ),
    meta: 'Anna Becker · Checkout-Formular barrierefrei machen · vor 1 Min.',
    waiting: true,
    action: (
      <>
        <Btn size="sm" variant="signal">
          Erlauben
        </Btn>
        <Btn size="sm">Ablehnen</Btn>
      </>
    ),
  },
  {
    icon: <AtSign className="size-3.5" />,
    title: '@ingo in einem Kommentar: „Kannst du dir Zeile 16 ansehen? Register fehlt.“',
    meta: 'Jonas Weber · Rate-Limiter für die Login-API · src/routes/auth.ts Z. 16 · vor 6 Min.',
    action: <Btn size="sm">Zum Kommentar</Btn>,
  },
  {
    icon: <MessageSquareReply className="size-3.5" />,
    title: 'Antwort in deinem Thread: „Den Body brauchen wir im Frontend nicht.“',
    meta: 'Anna Becker · Rate-Limiter für die Login-API · vor 9 Min.',
    action: <Btn size="sm">Zum Thread</Btn>,
  },
  {
    icon: <Share2 className="size-3.5" />,
    title: 'Session mit dir geteilt: „Preisberechnung für Bundles“ · Kommentieren & freigeben',
    meta: 'Jonas Weber · vor 32 Min.',
    action: <Btn size="sm">Öffnen</Btn>,
  },
]

const done: Item[] = [
  { icon: <Check className="size-3.5" />, title: 'Freigabe pnpm vitest run middleware · entschieden von Anna Becker', meta: 'Rate-Limiter für die Login-API · 14:12', done: true },
  { icon: <Check className="size-3.5" />, title: '3 adressierte Kommentare erledigt in Turn 4', meta: 'Rate-Limiter für die Login-API · 14:31', done: true },
]

function InboxRow({ it }: { it: Item }) {
  return (
    <div className={cn('flex items-start gap-3 border-b border-border px-3 py-2.5', it.waiting && 'chamfer border-l-4 border-l-signal bg-signal-soft', it.done && 'text-muted-foreground')}>
      <span className="mt-1 flex size-4 shrink-0 items-center justify-center text-muted-foreground">{it.icon}</span>
      <span className="min-w-0 flex-1">
        <span className="block text-[13px]">{it.title}</span>
        <span className="block text-[12px] text-muted-foreground">{it.meta}</span>
      </span>
      {it.action && <span className="flex shrink-0 gap-1.5">{it.action}</span>}
    </div>
  )
}

function Inbox() {
  return (
    <F id="COL-009" className="px-6 py-3">
      <div className="mb-2 flex items-center gap-2">
        <Segmented
          value="open"
          items={[
            { id: 'open', label: 'Offen · 4' },
            { id: 'done', label: 'Erledigt' },
          ]}
        />
      </div>
      <div className="border-t border-border">
        {open.map((it, i) => (
          <InboxRow key={i} it={it} />
        ))}
      </div>
      <div className="mt-6 mb-1 text-[12px] font-medium text-muted-foreground">Heute erledigt</div>
      <div className="border-t border-border">
        {done.map((it, i) => (
          <InboxRow key={i} it={it} />
        ))}
      </div>
    </F>
  )
}

const kinds = [
  ['Freigabe nötig', [true, true, false]],
  ['Agent fertig (dein letzter Input)', [true, true, false]],
  ['Session fehlgeschlagen', [true, true, false]],
  ['Erwähnung in Kommentar', [true, false, false]],
  ['Antwort in deinem Thread', [true, false, false]],
  ['Session mit dir geteilt', [true, false, false]],
] as const

function Cell({ on, disabled }: { on: boolean; disabled?: boolean }) {
  return (
    <span
      role="checkbox"
      aria-checked={on}
      aria-disabled={disabled}
      className={cn('mx-auto flex size-3.5 items-center justify-center rounded-[3px] border text-[10px]', on ? 'border-foreground bg-foreground text-background' : 'border-muted-foreground', disabled && 'opacity-40')}
    >
      {on ? '✓' : ''}
    </span>
  )
}

function Settings() {
  return (
    <F id="COL-010" className="max-w-3xl px-6 py-4">
      <table className="w-full text-[13px]">
        <thead className="text-[12px] text-muted-foreground">
          <tr className="border-b border-border">
            <th className="py-2 text-left font-medium">Ereignis</th>
            <th className="w-28 py-2 font-medium">In der App</th>
            <th className="w-28 py-2 font-medium">Desktop</th>
            <th className="w-36 py-2 font-medium">Web-Push (optional)</th>
          </tr>
        </thead>
        <tbody>
          {kinds.map(([label, ch]) => (
            <tr key={label} className="border-b border-border">
              <td className="py-2">{label}</td>
              <td>
                <Cell on={ch[0]} disabled />
              </td>
              <td>
                <Cell on={ch[1]} />
              </td>
              <td>
                <Cell on={ch[2]} disabled />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mt-4 flex flex-col gap-2 text-[13px]">
        <p>
          <span className="font-medium">Nicht stören, wenn du hinsiehst:</span> Solange du eine Session auf einem deiner Geräte im Vordergrund hast, meldet beton dafür nichts
          zusätzlich.
        </p>
        <p>
          <span className="font-medium">Zusammenfassen:</span> Gleiche Ereignisse einer Session innerhalb von 30 Sekunden kommen als eine Meldung, etwa „5 Freigaben ausstehend“.
        </p>
        <div className="flex items-center gap-3 border-t border-border pt-3">
          <NetNote>Web-Push ist aus. Es läuft über den Push-Dienst deines Browsers (Apple, Google oder Mozilla). Ohne Web-Push bleibt alles auf deinen Geräten.</NetNote>
          <Btn size="sm" className="ml-auto">
            Web-Push einschalten
          </Btn>
        </div>
      </div>
    </F>
  )
}

function DesktopToast() {
  return (
    <F id="COL-010" className="absolute top-3 right-3 z-40 w-[340px]">
      <div className="flex gap-3 rounded-xl border border-border bg-popover/95 p-3 shadow-[0_16px_40px_-20px_rgb(0_0_0/0.5)]">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-foreground text-[11px] font-bold text-background">b</span>
        <div className="min-w-0 text-[13px]">
          <div className="flex items-baseline gap-2">
            <span className="font-semibold">beton</span>
            <span className="ml-auto text-[11px] text-muted-foreground">jetzt</span>
          </div>
          <div className="font-medium">5 Freigaben ausstehend</div>
          <div className="text-muted-foreground">Checkout-Formular barrierefrei machen · Anna Becker</div>
        </div>
      </div>
      <div className="mt-2 flex items-start gap-2 rounded-md border border-border bg-card px-2.5 py-1.5 text-[12px] text-muted-foreground">
        <Bell className="mt-0.5 size-3.5 shrink-0" />
        „Rate-Limiter: Agent fertig“ nicht gemeldet, weil du die Session gerade auf dem Desktop ansiehst.
      </div>
    </F>
  )
}

export function InboxScreen({ state }: { state: string }) {
  return (
    <div className="relative h-full">
      <AppLayout nav="inbox" sessionList={false} connection="server">
        <PageHeader
          title={state === 'settings' ? 'Benachrichtigungen' : 'Inbox'}
          subtitle={
            state === 'settings'
              ? 'Wann dich beton auf welchem Weg erreicht. In der App siehst du immer alles.'
              : 'Was auf dich wartet: Freigaben, Erwähnungen, Antworten und neue Freigaben anderer.'
          }
          actions={state === 'settings' ? <Tag tone="muted">Gilt für alle deine Geräte</Tag> : <Btn size="sm">Einstellungen</Btn>}
        />
        <div className="min-h-0 flex-1 overflow-y-auto">{state === 'settings' ? <Settings /> : <Inbox />}</div>
      </AppLayout>
      {state === 'notification' && <DesktopToast />}
    </div>
  )
}
