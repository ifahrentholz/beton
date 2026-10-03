import type { ReactNode } from 'react'
import { Bot, Check, CornerDownRight, MessageSquare } from 'lucide-react'
import { WorkspaceRail } from '@/app/session-chrome'
import { AgentMessage, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { rateLimiterDiff } from '@/mock/data'
import { Avatar, Btn, Callout, FakeInput, Segmented, SessionShell, Tag } from '../workspace/parts'

type C = { who: string; when: string; text: ReactNode }

function Comment({ c, system }: { c: C; system?: boolean }) {
  return (
    <div className="flex gap-2">
      {system ? (
        <span className="flex size-5 shrink-0 items-center justify-center rounded-full border border-border">
          <Bot className="size-3 text-muted-foreground" />
        </span>
      ) : (
        <Avatar name={c.who} size="sm" />
      )}
      <div className="min-w-0 flex-1 text-[13px]">
        <span className="font-medium">{c.who}</span> <span className="text-[11px] text-muted-foreground">{c.when}</span>
        <div className="mt-0.5 leading-relaxed">{c.text}</div>
      </div>
    </div>
  )
}

function Thread({
  anchor,
  comments,
  status,
  actions = 'owner',
  selectable,
  selected,
  addressedNote,
  compact,
}: {
  anchor: ReactNode
  comments: C[]
  status?: 'open' | 'resolved' | 'outdated' | 'addressed' | 'suggested'
  actions?: 'owner' | 'comment' | 'none'
  selectable?: boolean
  selected?: boolean
  addressedNote?: boolean
  compact?: boolean
}) {
  return (
    <div className={cn('rounded-md border bg-card', selected ? 'border-foreground' : 'border-border', status === 'resolved' && 'opacity-60')}>
      <div className="flex items-center gap-2 border-b border-border px-2.5 py-1.5 text-[11px] text-muted-foreground">
        {selectable && (
          <span role="checkbox" aria-checked={selected} className={cn('flex size-3.5 items-center justify-center rounded-[3px] border text-[10px]', selected ? 'border-foreground bg-foreground text-background' : 'border-muted-foreground')}>
            {selected ? '✓' : ''}
          </span>
        )}
        <span className="min-w-0 truncate">{anchor}</span>
        <span className="ml-auto flex shrink-0 gap-1">
          {status === 'outdated' && <Tag tone="muted">veraltet</Tag>}
          {status === 'resolved' && <Tag tone="muted">aufgelöst</Tag>}
          {status === 'addressed' && <Tag tone="ok">adressiert</Tag>}
          {status === 'suggested' && <Tag>für Agent vorgeschlagen</Tag>}
        </span>
      </div>
      <div className="flex flex-col gap-2 p-2.5">
        {(compact ? comments.slice(0, 1) : comments).map((c, i) => (
          <Comment key={i} c={c} />
        ))}
        {compact && comments.length > 1 && <span className="pl-7 text-[11px] text-muted-foreground">{comments.length - 1} Antwort</span>}
        {addressedNote && (
          <Comment
            system
            c={{
              who: 'beton',
              when: '14:31',
              text: (
                <>
                  Adressiert in Turn 4. <span className="underline underline-offset-2">Turn ansehen</span>
                </>
              ),
            }}
          />
        )}
      </div>
      {!compact && actions !== 'none' && status !== 'resolved' && (
        <div className="flex flex-wrap items-center gap-1 border-t border-border px-2 py-1.5">
          <FakeInput placeholder="Antworten – @ erwähnt jemanden" className="h-7 min-w-40 flex-1 text-xs" />
          <Btn size="sm" variant="ghost">
            <Check className="size-3.5" /> Auflösen
          </Btn>
          {actions === 'owner' ? (
            <Btn size="sm" variant="ghost">
              <CornerDownRight className="size-3.5" /> An Agent
            </Btn>
          ) : (
            <Btn size="sm" variant="ghost">
              <CornerDownRight className="size-3.5" /> Für Agent vorschlagen
            </Btn>
          )}
        </div>
      )}
    </div>
  )
}

const tMessage: C[] = [
  { who: 'Jonas Weber', when: '14:09', text: '429 ohne Body wäre konsistenter mit dem Rest der API. Wir schicken sonst überall nur den Status.' },
  { who: 'Ingo Fahrentholz', when: '14:11', text: 'Stimmt. @anna, brauchst du den Body im Frontend?' },
]
const tDiff: C[] = [{ who: 'Jonas Weber', when: '14:14', text: 'Die Register-Route braucht dasselbe Limit, sonst ist das die neue Lücke.' }]
const tFile: C[] = [{ who: 'Anna Becker', when: '14:16', text: 'buckets wächst ohne Grenze. Bitte alte Einträge nach dem Fenster wegräumen.' }]
const tOutdated: C[] = [{ who: 'Anna Becker', when: '13:58', text: 'Hier fehlt der Import-Typ.' }]

function DiffWithThreads() {
  return (
    <F id="COL-006" className="flex flex-col gap-3 p-3">
      <div className="overflow-hidden rounded-md border border-border bg-card font-mono text-[12px]">
        <div className="border-b border-border bg-sunken px-3 py-1 text-[11px] text-muted-foreground">src/routes/auth.ts</div>
        {rateLimiterDiff.map((l, i) => (
          <div key={i}>
            <div className={cn('flex', l.kind === 'add' && 'bg-ok-soft', l.kind === 'del' && 'bg-deny-soft')}>
              <span className="w-10 shrink-0 pr-2 text-right text-muted-foreground select-none">{l.n}</span>
              <span className="w-4 shrink-0 text-muted-foreground select-none">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>
              <span className="whitespace-pre">{l.text}</span>
              {l.kind === 'add' && l.n === 16 && <MessageSquare className="mt-0.5 mr-2 ml-auto size-3.5 shrink-0 text-foreground" aria-label="1 Kommentar" />}
            </div>
            {l.kind === 'add' && l.n === 16 && (
              <div className="border-y border-border bg-background p-2 font-sans">
                <Thread anchor="Z. 16 neu" comments={tDiff} />
              </div>
            )}
          </div>
        ))}
      </div>
      <Thread
        anchor="src/middleware/rate-limit.ts · Z. 1 (alt) · der kommentierte Bereich wurde geändert"
        status="outdated"
        comments={[
          ...tOutdated,
          {
            who: 'beton',
            when: '14:02',
            text: <pre className="mt-1 rounded-sm bg-sunken px-2 py-1 font-mono text-[11.5px]">{"import { RequestHandler } from 'express'"}</pre>,
          },
        ]}
      />
    </F>
  )
}

function Overview({ state }: { state: string }) {
  const address = state === 'address'
  const addressed = state === 'addressed'
  const suggest = state === 'suggest'
  return (
    <F id={['COL-005', 'COL-006', 'COL-007']} className="flex min-h-full flex-col">
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        <Segmented
          value="open"
          items={[
            { id: 'open', label: addressed ? 'Offen · 0' : 'Offen · 3' },
            { id: 'suggested', label: 'Vorgeschlagen · 1' },
            { id: 'resolved', label: 'Aufgelöst · 2' },
          ]}
        />
      </div>
      <div className="flex flex-1 flex-col gap-2 p-3">
        <Thread
          anchor="Nachricht von Claude Code · 14:06 · „antwortet sie mit 429“"
          comments={tMessage}
          compact={address || addressed}
          selectable={address}
          selected={address}
          status={addressed ? 'addressed' : undefined}
          addressedNote={addressed}
          actions={suggest ? 'comment' : 'owner'}
        />
        <Thread
          anchor="src/routes/auth.ts · Z. 16 neu"
          comments={tDiff}
          compact
          selectable={address}
          selected={address}
          status={addressed ? 'addressed' : suggest ? 'suggested' : undefined}
        />
        <Thread anchor="src/middleware/rate-limit.ts · Z. 9–10" comments={tFile} compact selectable={address} selected={address} status={addressed ? 'addressed' : 'suggested'} />
        {!address && <div className="px-1 pt-1 text-[12px] text-muted-foreground">2 aufgelöste Threads ausgeblendet · einblenden</div>}
        {suggest && (
          <Callout className="mt-1">
            Du hast die Rolle <span className="font-medium">Kommentieren &amp; freigeben</span>. Kommentare an den Agent geben darf, wer mitsteuert; Ingo sieht deinen Vorschlag.
          </Callout>
        )}
      </div>
      {address && (
        <div className="sticky bottom-0 border-t border-border bg-card p-3">
          <div className="mb-2 text-[12px] text-muted-foreground">Wird ein Eintrag in der Warteschlange:</div>
          <pre className="max-h-40 overflow-auto rounded-md border border-border bg-background p-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap">{`Nachricht von Claude Code, 14:06 (Kommentar von @jonas): 429 ohne Body wäre konsistenter …
> antwortet sie mit 429 und einem Retry-After-Header

Datei src/routes/auth.ts Z. 16 (Kommentar von @jonas): Die Register-Route braucht dasselbe Limit …
> auth.post('/login', rateLimit({ window: '1m', max: 5, key: 'ip' }), login)

Datei src/middleware/rate-limit.ts Z. 9–10 (Kommentar von @anna): buckets wächst ohne Grenze …`}</pre>
          <div className="mt-2 flex items-center gap-2">
            <Btn variant="primary">3 Kommentare an den Agent geben</Btn>
            <Btn variant="ghost">Auswahl aufheben</Btn>
          </div>
        </div>
      )}
    </F>
  )
}

export function CommentsScreen({ state }: { state: string }) {
  const suggest = state === 'suggest'
  return (
    <SessionShell
      connection="server"
      rail={<WorkspaceRail active={state === 'diff' ? 'changes' : 'comments'}>{state === 'diff' ? <DiffWithThreads /> : <Overview state={state} />}</WorkspaceRail>}
      composer={
        suggest ? (
          <div className="shrink-0 border-t border-border px-4 py-3 text-[13px] text-muted-foreground">
            Du kannst kommentieren und Freigaben erteilen. Nachrichten an den Agent schreibt, wer mitsteuert.
          </div>
        ) : undefined
      }
    >
      <UserMessage author="Ingo">Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
      <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts" duration="0,3 s" />
      <F id="COL-005">
        <AgentMessage harness="claude">
          <p>
            Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Danach{' '}
            <mark className="rounded-[2px] bg-foreground/10 text-foreground underline decoration-foreground/50 decoration-dotted underline-offset-4">
              antwortet sie mit <code className="rounded-sm bg-muted px-1 text-[13px]">429</code>
            </mark>{' '}
            und einem <code className="rounded-sm bg-muted px-1 text-[13px]">Retry-After</code>-Header.
          </p>
        </AgentMessage>
        {state === 'message' && (
          <div className="mt-2 ml-9">
            <Thread anchor="Markierter Text · „antwortet sie mit 429“" comments={tMessage} />
          </div>
        )}
        {state !== 'message' && (
          <div className="mt-1 ml-9 inline-flex items-center gap-1 text-[12px] text-muted-foreground">
            <MessageSquare className="size-3" /> 2 Kommentare
          </div>
        )}
      </F>
      {state === 'addressed' && (
        <F id="COL-007" className="flex flex-col gap-4">
          <UserMessage author="Ingo">
            <span className="text-muted-foreground">3 Kommentare an den Agent gegeben</span>
            <div className="mt-1 flex flex-col gap-1 border-l-2 border-border pl-3 text-[13px]">
              <span>Nachricht 14:06 · @jonas: 429 ohne Body wäre konsistenter …</span>
              <span>src/routes/auth.ts Z. 16 · @jonas: Die Register-Route braucht dasselbe Limit …</span>
              <span>src/middleware/rate-limit.ts Z. 9–10 · @anna: buckets wächst ohne Grenze …</span>
            </div>
          </UserMessage>
          <ToolCall kind="edit" name="Bearbeiten" target="src/middleware/rate-limit.ts" duration="0,4 s" />
          <AgentMessage harness="claude">
            <p>Erledigt: Die Register-Route hat dasselbe Limit, alte Buckets räumt ein Timer nach dem Fenster weg, und 429 kommt jetzt ohne Body. Die Threads sind als adressiert markiert.</p>
          </AgentMessage>
        </F>
      )}
    </SessionShell>
  )
}
