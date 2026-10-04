import type { ReactNode } from 'react'
import { ArrowLeft, ArrowUp, Check, ChevronRight, Inbox, Laptop, Lock, MessagesSquare, Mic, MoreHorizontal, Plus, RefreshCw, Share, ShieldCheck, SquarePlus, Wifi, WifiOff } from 'lucide-react'
import { HarnessBadge, StatusMark, VoiceDot } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { harnesses, inboxWaitingCount, projects, sessions } from '@/mock/data'

/* ───────────────────────── Mobile-Rahmen der PWA ───────────────────────── */

function MobileShell({
  title,
  back,
  tab = 'sessions',
  banner,
  children,
  action,
  noTabs,
  footer,
  inboxCount = inboxWaitingCount,
}: {
  footer?: ReactNode
  /** Zähler am Inbox-Tab; Standard: offene Freigaben und Fragen aus den Beispieldaten. */
  inboxCount?: number
  title: ReactNode
  back?: string
  tab?: 'sessions' | 'inbox' | 'more'
  banner?: ReactNode
  children: ReactNode
  action?: ReactNode
  noTabs?: boolean
}) {
  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
        {back && (
          <button className="-ml-1 flex items-center gap-0.5 text-[13px] text-muted-foreground" aria-label={`Zurück zu ${back}`}>
            <ArrowLeft className="size-4" />
          </button>
        )}
        <div className="min-w-0 flex-1 truncate text-[15px] font-semibold">{title}</div>
        {action}
      </div>
      {banner}
      <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
      {footer}
      {!noTabs && (
        <div className="flex h-14 shrink-0 items-start justify-around border-t border-border bg-sidebar pt-1.5">
          {[
            { id: 'sessions', label: 'Sessions', icon: MessagesSquare },
            { id: 'inbox', label: 'Inbox', icon: Inbox, badge: inboxCount },
            { id: 'more', label: 'Mehr', icon: MoreHorizontal },
          ].map((t) => (
            <button key={t.id} className={cn('relative flex w-20 flex-col items-center gap-0.5 text-[10px]', t.id === tab ? 'text-foreground' : 'text-muted-foreground')} aria-current={t.id === tab ? 'page' : undefined}>
              <t.icon className="size-5" />
              {t.label}
              {t.badge ? <span className="chamfer-sm absolute -top-0.5 right-5 min-w-4 bg-signal px-0.5 text-center text-[10px] leading-4 font-semibold text-signal-foreground">{t.badge}</span> : null}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

function Banner({ tone = 'neutral', children }: { tone?: 'neutral' | 'deny' | 'ok'; children: ReactNode }) {
  return (
    <div className={cn('flex items-center gap-2 border-b px-3 py-2 text-[12px]', tone === 'deny' && 'border-deny/40 bg-deny-soft', tone === 'ok' && 'border-ok/40 bg-ok-soft', tone === 'neutral' && 'border-border bg-card')}>
      {children}
    </div>
  )
}

/* ───────────────────────── Session-Liste ───────────────────────── */

function MobileSessions({ state }: { state: string }) {
  const stale = state === 'reconnecting' || state === 'offline'
  return (
    <MobileShell
      title="Sessions"
      action={
        <button aria-label="Neue Session" className="flex size-8 items-center justify-center rounded-md bg-foreground text-background">
          <Plus className="size-4" />
        </button>
      }
      banner={
        state === 'reconnecting' ? (
          <F id="PROTO-009">
            <Banner>
              <RefreshCw className="size-3.5 animate-spin" />
              <span className="flex-1">Verbindung zu „Ingos MacBook“ unterbrochen. Neuer Versuch in 4 s …</span>
              <button className="font-medium underline underline-offset-2">Jetzt</button>
            </Banner>
          </F>
        ) : state === 'offline' ? (
          <Banner tone="deny">
            <WifiOff className="size-3.5" />
            <span>Rechner nicht erreichbar. Du siehst den Stand von 14:02. Deine Sessions laufen dort weiter.</span>
          </Banner>
        ) : (
          <div className="flex items-center gap-1.5 border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground">
            <span className="size-1.5 rounded-full bg-ok" /> Verbunden mit Ingos MacBook · über Tailscale
          </div>
        )
      }
    >
      <F id="WEB-013" className={cn(stale && 'dimmed')}>
        {projects.map((p) => {
          const list = sessions.filter((s) => s.project === p.id)
          return (
            <div key={p.id}>
              <div className="px-3 pt-3 pb-1 text-[12px] font-semibold">{p.name}</div>
              {list.map((s) => (
                <div key={s.id} className={cn('flex items-center gap-3 border-b border-border/60 px-3 py-2.5', s.status === 'waiting' && 'border-l-4 border-l-signal bg-signal-soft')}>
                  <StatusMark status={s.status} />
                  <div className="min-w-0 flex-1">
                    <div className={cn('truncate text-[14px]', s.unread && 'font-semibold')}>{s.title}</div>
                    <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                      <VoiceDot voice={harnesses[s.harness].voice} className="size-1.5" />
                      {harnesses[s.harness].name}
                      {s.status === 'waiting' && <span className="font-medium text-foreground">· Freigabe offen</span>}
                      <span className="ml-auto">{s.updated}</span>
                    </div>
                  </div>
                  <ChevronRight className="size-4 text-muted-foreground" />
                </div>
              ))}
            </div>
          )
        })}
      </F>
    </MobileShell>
  )
}

/* ───────────────────────── Session-Verlauf kompakt ───────────────────────── */

function MobileSession({ state }: { state: string }) {
  return (
    <MobileShell
      title={
        <span className="flex flex-col leading-tight">
          <span className="truncate text-[14px]">Rate-Limiter für die Login-API</span>
          <span className="text-[11px] font-normal text-muted-foreground">
            <HarnessBadge id="claude" className="text-[11px]" /> · 38 % Kontext
          </span>
        </span>
      }
      back="Sessions"
      noTabs
      footer={
      <div className="flex items-center gap-2 border-t border-border bg-background px-2 py-2">
        <span className="flex h-9 flex-1 items-center rounded-full border border-input bg-card px-3 text-[14px] text-muted-foreground">Kurze Antwort …</span>
        <button aria-label="Diktieren" className="flex size-9 items-center justify-center rounded-full border border-border">
          <Mic className="size-4" />
        </button>
        <button aria-label="Senden" className="flex size-9 items-center justify-center rounded-full bg-foreground text-background">
          <ArrowUp className="size-4" />
        </button>
      </div>
      }
      banner={
        state === 'resumed' ? (
          <F id={['PROTO-005', 'PROTO-009']}>
            <Banner tone="ok">
              <Check className="size-3.5 text-ok" />
              Wieder verbunden. 23 neue Ereignisse nachgeladen – nichts verpasst.
            </Banner>
          </F>
        ) : undefined
      }
    >
      <F id="WEB-013" className="flex flex-col gap-3 px-3 py-3 text-[14px] leading-relaxed">
        <button className="self-center text-[12px] text-muted-foreground underline underline-offset-2">Ältere Nachrichten laden</button>
        <div className="rounded-md bg-card px-3 py-2">
          <div className="text-[11px] text-muted-foreground">Ingo</div>
          Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP.
        </div>
        <div className="flex flex-col gap-1 text-[12px] text-muted-foreground">
          <div className="flex items-center gap-2"><Check className="size-3 text-ok" /> Suche · rg 'auth.post' src/</div>
          <div className="flex items-center gap-2"><Check className="size-3 text-ok" /> Bearbeitet · src/routes/auth.ts <span className="text-ok">+2 −1</span></div>
          <div className="flex items-center gap-2"><Check className="size-3 text-ok" /> Shell · pnpm vitest run auth · 10 Tests ok</div>
        </div>
        <div className="flex gap-2">
          <span className="mt-1.5 size-2.5 shrink-0 rounded-[2px] bg-voice-claude" />
          <div>
            Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Danach antwortet sie mit <code className="rounded-sm bg-muted px-1 text-[12px]">429</code>.
            Soll ich pushen und einen PR öffnen?
          </div>
        </div>
        <div className="self-end rounded-md bg-card px-3 py-2">
          <div className="text-[11px] text-muted-foreground">Ingo · vom Handy</div>
          Ja, push und PR.
        </div>
        <div className="chamfer border-l-4 border-signal bg-signal-soft p-3">
          <div className="text-[13px] font-semibold">Freigabe nötig: Shell</div>
          <pre className="mt-1 overflow-x-auto font-mono text-[11.5px]">git push -u origin beton/rate-limiter-7f3k</pre>
          <button className="chamfer-sm mt-2 w-full bg-foreground py-2 text-[14px] font-semibold text-background">Ansehen und entscheiden</button>
        </div>
      </F>
    </MobileShell>
  )
}

/* ───────────────────────── Freigabe unterwegs ───────────────────────── */

function MobileApproval({ state }: { state: string }) {
  return (
    <MobileShell title="Freigabe" back="Session" noTabs>
      <F id={['WEB-013', 'UX-001']} className="flex h-full flex-col px-4 py-4">
        <div className="text-[12px] text-muted-foreground">
          Rate-Limiter für die Login-API · shop-frontend
        </div>
        <div className="mt-1 flex items-center gap-2 text-[12px]">
          <HarnessBadge id="claude" /> <span className="text-muted-foreground">· wartet seit 3 Min.</span>
        </div>

        <div className={cn('mt-4 border-l-4 p-3', state === 'allowed' ? 'border-border bg-card' : 'chamfer border-signal bg-signal-soft')}>
          <div className="flex items-center gap-1.5 text-[14px] font-semibold">
            <ShieldCheck className="size-4" /> {state === 'allowed' ? 'Erlaubt' : 'Claude Code möchte pushen'}
          </div>
          <pre className="mt-2 rounded-sm bg-card px-2 py-1.5 font-mono text-[12px] whitespace-pre-wrap">git push -u origin beton/rate-limiter-7f3k</pre>
          <p className="mt-2 text-[13px]">Pushes verlassen deinen Rechner. Die Projekt-Policy verlangt dafür deine Freigabe.</p>
          <div className="mt-1 font-mono text-[11px] text-muted-foreground">Regel git-push-fragen</div>
        </div>

        {state === 'deny-reason' && (
          <div className="mt-4">
            <label className="text-[12px] text-muted-foreground">Grund für den Agent (optional)</label>
            <div className="mt-1 min-h-16 rounded-md border-2 border-ring bg-card px-3 py-2 text-[14px]">
              Erst nach dem Review durch Mara
              <span className="ml-0.5 inline-block h-4 w-px animate-pulse bg-foreground" />
            </div>
          </div>
        )}

        {state === 'allowed' && (
          <div className="mt-4 flex items-center gap-2 text-[13px]">
            <Check className="size-4 text-ok" /> Der Agent macht weiter. Du kannst das Handy weglegen.
          </div>
        )}

        <div className="mt-auto flex flex-col gap-2 pb-2">
          {state === 'open' && (
            <>
              <button className="chamfer-sm h-12 bg-foreground text-[16px] font-semibold text-background">Erlauben</button>
              <button className="h-12 rounded-md border border-foreground/30 text-[16px]">Ablehnen …</button>
              <button className="h-9 text-[13px] text-muted-foreground">Für diese Session erlauben</button>
            </>
          )}
          {state === 'deny-reason' && (
            <>
              <button className="h-12 rounded-md border border-deny bg-deny-soft text-[16px] font-semibold text-deny">Ablehnen und Grund senden</button>
              <button className="h-9 text-[13px] text-muted-foreground">Abbrechen</button>
            </>
          )}
          {state === 'allowed' && <button className="h-12 rounded-md border border-border text-[15px]">Zur Session</button>}
          <p className="text-center text-[11px] text-muted-foreground">Aus der Benachrichtigung: ein Tipp zum Öffnen, ein Tipp zum Entscheiden.</p>
        </div>
      </F>
    </MobileShell>
  )
}

/* ───────────────────────── Inbox ───────────────────────── */

function MobileInbox({ state }: { state: string }) {
  return (
    <MobileShell title="Inbox" tab="inbox" inboxCount={state === 'empty' ? 0 : undefined}>
      <F id={['UX-001', 'WEB-013']}>
        {state === 'empty' ? (
          <div className="flex flex-col items-center gap-2 px-6 pt-24 text-center">
            <Check className="size-6 text-ok" />
            <div className="text-[15px] font-semibold">Nichts wartet auf dich</div>
            <div className="text-[13px] text-muted-foreground">Freigaben und Fragen erscheinen hier, sobald ein Agent dich braucht.</div>
          </div>
        ) : (
          <>
            <div className="px-3 pt-3 pb-1 text-[12px] font-medium text-muted-foreground">Du bist dran · {inboxWaitingCount}</div>
            {[
              { k: 'Freigabe', t: 'git push -u origin beton/rate-limiter-7f3k', s: 'Rate-Limiter für die Login-API', w: '3 Min.', mono: true },
              { k: 'Frage', t: 'ESLint 10 bricht 3 Regeln – anpassen oder bei 9 bleiben?', s: 'Nächtliches Dependency-Update · pausiert', w: '41 Min.' },
            ].map((x) => (
              <div key={x.t} className="chamfer mx-3 mb-2 border-l-4 border-signal bg-signal-soft px-3 py-2.5">
                <div className="flex text-[11px] text-muted-foreground">
                  <span className="font-medium text-foreground">{x.k}</span>
                  <span className="ml-auto">{x.w}</span>
                </div>
                <div className={cn('mt-0.5 text-[14px] font-medium', x.mono && 'font-mono text-[12.5px]')}>{x.t}</div>
                <div className="text-[12px] text-muted-foreground">{x.s}</div>
              </div>
            ))}
            <div className="px-3 pt-3 pb-1 text-[12px] font-medium text-muted-foreground">Zur Info</div>
            {[
              { k: 'Hinweis', t: 'Claude Max: 92 % des 5-Stunden-Fensters genutzt', w: '6 Min.' },
              { k: 'Erwähnung', t: 'Mara: „@ingo schaust du dir den Fokus-Trap an?“', w: '1 Std.' },
              { k: 'Agent fertig', t: 'Docs-Linkcheck – PR #482 geöffnet', w: '06:00' },
            ].map((x) => (
              <div key={x.t} className="border-b border-border/60 px-3 py-2.5">
                <div className="flex text-[11px] text-muted-foreground">
                  {x.k}
                  <span className="ml-auto">{x.w}</span>
                </div>
                <div className="text-[14px]">{x.t}</div>
              </div>
            ))}
          </>
        )}
      </F>
    </MobileShell>
  )
}

/* ───────────────────────── Web-Push auf dem Sperrbildschirm ───────────────────────── */

function MobilePush({ state }: { state: string }) {
  const preview = state === 'preview'
  return (
    <div className="dark concrete-grain flex h-full flex-col items-center bg-background px-3 pt-10 text-foreground">
      <Lock className="size-4 text-muted-foreground" />
      <div className="type-wide mt-2 text-[56px] leading-none font-[300] tabular-nums">14:05</div>
      <div className="mt-1 text-[14px] text-muted-foreground">Samstag, 3. Oktober</div>
      <F id="WEB-014" className="mt-10 w-full">
        <div className="rounded-2xl border border-border bg-card/90 p-3 shadow-lg backdrop-blur">
          <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
            <span className="flex size-5 items-center justify-center rounded-[4px] bg-foreground text-[10px] font-bold text-background">b</span>
            beton
            <span className="ml-auto">jetzt</span>
          </div>
          <div className="mt-1.5 text-[14px] font-semibold">Freigabe wartet · shop-frontend</div>
          <div className="text-[13px] text-muted-foreground">
            {preview ? 'Claude Code: git push -u origin beton/rate-limiter-7f3k' : 'Claude Code braucht deine Entscheidung. Tippen zum Öffnen.'}
          </div>
        </div>
        <div className="mt-2 rounded-2xl border border-border bg-card/70 p-3">
          <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
            <span className="flex size-5 items-center justify-center rounded-[4px] bg-foreground text-[10px] font-bold text-background">b</span>
            beton
            <span className="ml-auto">vor 40 Min.</span>
          </div>
          <div className="mt-1.5 text-[14px] font-semibold">Frage wartet · infra</div>
          <div className="text-[13px] text-muted-foreground">{preview ? 'ESLint 10 bricht 3 Regeln – anpassen oder bei 9 bleiben?' : 'Ein Agent hat eine Frage. Tippen zum Öffnen.'}</div>
        </div>
        <p className="mt-4 px-2 text-center text-[11px] leading-relaxed text-muted-foreground">
          Optional, über den Push-Dienst von Apple zugestellt (verschlüsselt). {preview ? 'Vorschau ist eingeschaltet: Befehl und Frage stehen im Push.' : 'Ohne Vorschau: kein Befehl, kein Nachrichtentext.'} Entschieden
          wird in der App, nicht auf dem Sperrbildschirm.
        </p>
      </F>
    </div>
  )
}

/* ───────────────────────── PWA-Installation & Update ───────────────────────── */

function MobileInstall({ state }: { state: string }) {
  if (state === 'update') {
    return (
      <MobileShell
        title="Sessions"
        banner={
          <F id="WEB-013">
            <Banner>
              <RefreshCw className="size-3.5" />
              <span className="flex-1">Neue Version von beton auf dem Rechner. Neu laden, um sie zu nutzen.</span>
              <button className="rounded-md bg-foreground px-2.5 py-1 text-[12px] font-semibold text-background">Neu laden</button>
            </Banner>
          </F>
        }
      >
        <div>
          {sessions.slice(0, 6).map((s) => (
            <div key={s.id} className="flex items-center gap-3 border-b border-border/60 px-3 py-2.5">
              <StatusMark status={s.status} />
              <div className="min-w-0 flex-1 truncate text-[14px]">{s.title}</div>
              <span className="text-[11px] text-muted-foreground">{s.updated}</span>
            </div>
          ))}
        </div>
      </MobileShell>
    )
  }
  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex h-9 shrink-0 items-center justify-center border-b border-border bg-sidebar text-[11px] text-muted-foreground">
        <Lock className="mr-1 size-3" /> ingos-macbook.tail4f2a.ts.net
      </div>
      <F id="WEB-013" className="flex flex-1 flex-col px-5 pt-10">
        <div className="chamfer flex size-14 items-center justify-center bg-foreground text-[22px] font-bold text-background">b</div>
        <div className="type-wide mt-4 text-[22px] font-[700]">beton aufs Handy</div>
        <p className="mt-2 text-[14px] text-muted-foreground">
          Lege beton auf den Home-Bildschirm. Dann bekommst du Freigaben schnell zu sehen und kannst optional Push-Benachrichtigungen erhalten.
        </p>
        <ol className="mt-6 space-y-3 text-[14px]">
          <li className="flex items-center gap-3">
            <span className="flex size-6 items-center justify-center rounded-full border border-foreground text-[12px]">1</span>
            Unten auf <Share className="inline size-4" /> „Teilen“ tippen
          </li>
          <li className="flex items-center gap-3">
            <span className="flex size-6 items-center justify-center rounded-full border border-foreground text-[12px]">2</span>
            <SquarePlus className="inline size-4" /> „Zum Home-Bildschirm“ wählen
          </li>
        </ol>
        <p className="mt-6 text-[12px] text-muted-foreground">
          Die App spricht nur mit deinem Rechner. Ohne Verbindung zu ihm zeigt sie den letzten Stand – arbeiten kannst du offline nicht.
        </p>
        <div className="mt-auto mb-3 rounded-xl border border-border bg-card p-3 text-[13px] shadow-lg">
          <div className="flex items-center gap-3">
            <SquarePlus className="size-5" />
            <span className="flex-1 font-medium">Zum Home-Bildschirm</span>
            <ChevronRight className="size-4 text-muted-foreground" />
          </div>
        </div>
      </F>
    </div>
  )
}

/* ───────────────────────── Verbindung zum Rechner ───────────────────────── */

function MobileConnect({ state }: { state: string }) {
  return (
    <MobileShell title="Mit deinem Rechner verbinden" noTabs>
      <F id="WEB-013" className="flex flex-col px-4 py-4 text-[14px]">
        <p className="text-muted-foreground">
          beton läuft auf deinem Rechner. Das Handy verbindet sich direkt dorthin – im selben WLAN oder über dein Tailscale-Netz. Kein Server dazwischen.
        </p>

        <div className="mt-4 divide-y divide-border rounded-md border border-border">
          {[
            { id: 'tailscale', icon: Laptop, t: 'Über Tailscale', s: 'ingos-macbook.tail4f2a.ts.net', note: 'auch unterwegs' },
            { id: 'lan', icon: Wifi, t: 'Im selben WLAN', s: 'ingos-macbook.local:7420', note: 'nur zu Hause' },
          ].map((x) => (
            <div key={x.id} className={cn('flex items-center gap-3 px-3 py-2.5', x.id === 'tailscale' && 'bg-accent/60')}>
              <x.icon className="size-4 text-muted-foreground" />
              <div className="min-w-0 flex-1">
                <div className="font-medium">{x.t}</div>
                <div className="font-mono text-[11px] text-muted-foreground">https://{x.s}</div>
              </div>
              <span className="text-[11px] text-muted-foreground">{x.note}</span>
            </div>
          ))}
        </div>

        {state === 'pair' && (
          <div className="mt-5">
            <div className="font-semibold">Gerät koppeln</div>
            <p className="mt-1 text-[13px] text-muted-foreground">
              Öffne auf dem Rechner <span className="text-foreground">Einstellungen › Geräte › Handy koppeln</span> und gib den angezeigten Code hier ein. Er gilt 5 Minuten.
            </p>
            <div className="mt-3 flex justify-center gap-1.5 font-mono text-[22px]">
              {['4', '8', '2', '9', '1', ''].map((d, i) => (
                <span key={i} className={cn('flex h-12 w-10 items-center justify-center rounded-md border bg-card', i === 5 ? 'border-2 border-ring' : 'border-input', i === 2 && 'mr-3')}>
                  {d}
                </span>
              ))}
            </div>
            <p className="mt-3 text-[11px] text-muted-foreground">Verbindung per HTTPS mit dem Zertifikat deines Rechners. Gekoppelte Geräte kannst du dort jederzeit entfernen.</p>
          </div>
        )}
        {state === 'unreachable' && (
          <div className="mt-5 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
            <div className="flex items-center gap-2 font-semibold">
              <span className="size-2 rotate-45 bg-deny" /> Rechner nicht erreichbar
            </div>
            <p className="mt-1">ingos-macbook.tail4f2a.ts.net antwortet nicht.</p>
            <ul className="mt-1 list-disc pl-4 text-muted-foreground">
              <li>Ist der Rechner wach und beton gestartet?</li>
              <li>Ist Tailscale auf beiden Geräten verbunden?</li>
            </ul>
            <button className="mt-2 rounded-md border border-border bg-card px-3 py-1.5">Erneut versuchen</button>
          </div>
        )}
        {state === 'incompatible' && (
          <F id="PROTO-004" className="mt-5 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
            <div className="flex items-center gap-2 font-semibold">
              <span className="size-2 rotate-45 bg-deny" /> Versionen passen nicht zusammen
            </div>
            <p className="mt-1">
              beton auf dem Rechner ist zu alt für diese App (Protokoll 1.2, nötig ist 1.3 oder 1.4). Aktualisiere beton auf dem Rechner und lade dann neu.
            </p>
            <div className="mt-1 font-mono text-[11px] text-muted-foreground">Code 4400 · unterstützt 1.3–1.4</div>
          </F>
        )}
      </F>
    </MobileShell>
  )
}

export const group: ScreenGroup = {
  id: 'mobile',
  title: 'Handy (PWA)',
  order: 190,
  screens: [
    {
      id: 'mobile-sessions',
      title: 'Session-Liste',
      description: 'Die installierte Web-App auf dem Handy: Sessions nach Projekt, offene Freigaben gelb. Verbindungsabbrüche werden mit automatischem Neuversuch angezeigt.',
      features: ['WEB-013', 'PROTO-009'],
      frame: 'mobile',
      states: [
        { id: 'default', title: 'Verbunden' },
        { id: 'reconnecting', title: 'Verbindet neu' },
        { id: 'offline', title: 'Rechner nicht erreichbar' },
      ],
      component: MobileSessions,
    },
    {
      id: 'mobile-session',
      title: 'Session kompakt',
      description: 'Lesen zuerst: Tool-Calls als einzeilige Liste, Freigabe als Karte, kurze Antworten oder Diktat unten.',
      features: ['WEB-013', 'PROTO-005', 'PROTO-009'],
      frame: 'mobile',
      states: [
        { id: 'default', title: 'Lesen' },
        { id: 'resumed', title: 'Nach Reconnect fortgesetzt' },
      ],
      component: MobileSession,
    },
    {
      id: 'mobile-approval',
      title: 'Freigabe unterwegs',
      description: 'Freigabe auf 375 px mit höchstens zwei Taps ab der Benachrichtigung; Ablehnen mit optionalem Grund.',
      features: ['WEB-013', 'UX-001'],
      frame: 'mobile',
      states: [
        { id: 'open', title: 'Offen' },
        { id: 'deny-reason', title: 'Ablehnen mit Grund' },
        { id: 'allowed', title: 'Erlaubt' },
      ],
      component: MobileApproval,
    },
    {
      id: 'mobile-inbox',
      title: 'Inbox',
      description: 'Inbox auf dem Handy: oben, was auf dich wartet; darunter Hinweise.',
      features: ['UX-001', 'WEB-013'],
      frame: 'mobile',
      states: [
        { id: 'default', title: 'Offene Einträge' },
        { id: 'empty', title: 'Leer' },
      ],
      component: MobileInbox,
    },
    {
      id: 'mobile-push',
      title: 'Web-Push auf dem Sperrbildschirm',
      description: 'Optional eingeschaltetes Web-Push. Ohne Vorschau steht kein Befehl und kein Text in der Nachricht; entschieden wird in der App.',
      features: ['WEB-014'],
      frame: 'mobile',
      states: [
        { id: 'minimal', title: 'Ohne Vorschau (Standard)' },
        { id: 'preview', title: 'Mit Vorschau' },
      ],
      component: MobilePush,
    },
    {
      id: 'mobile-install',
      title: 'Installieren & Update',
      description: 'Anleitung „Zum Home-Bildschirm“ und Hinweis, wenn eine neue Version bereitsteht.',
      features: ['WEB-013'],
      frame: 'mobile',
      states: [
        { id: 'install', title: 'Installieren' },
        { id: 'update', title: 'Update verfügbar' },
      ],
      component: MobileInstall,
    },
    {
      id: 'mobile-connect',
      title: 'Verbindung zum Rechner',
      description: 'Direkte Verbindung zum lokalen beton per Tailscale oder LAN mit HTTPS und Geräte-Kopplung; Fehlerzustände inklusive Versionskonflikt.',
      features: ['WEB-013', 'PROTO-004'],
      frame: 'mobile',
      states: [
        { id: 'pair', title: 'Koppeln' },
        { id: 'unreachable', title: 'Nicht erreichbar' },
        { id: 'incompatible', title: 'Version inkompatibel' },
      ],
      component: MobileConnect,
    },
  ],
}
