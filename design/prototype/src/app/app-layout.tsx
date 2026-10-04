import type { ReactNode } from 'react'
import { Bot, CalendarClock, Gauge, Inbox, MessagesSquare, Plus, Search, Settings, ShieldCheck } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { harnesses, projects, sessions } from '@/mock/data'
import { StatusMark, VoiceDot, voiceVar } from './harness'

export type NavItem = 'sessions' | 'inbox' | 'agents' | 'automations' | 'usage' | 'policies' | 'settings'

const NAV: { id: NavItem; label: string; icon: typeof Inbox; badge?: number }[] = [
  { id: 'sessions', label: 'Sessions', icon: MessagesSquare },
  { id: 'inbox', label: 'Inbox', icon: Inbox, badge: 2 },
  { id: 'agents', label: 'Agents', icon: Bot },
  { id: 'automations', label: 'Automationen', icon: CalendarClock },
  { id: 'usage', label: 'Verbrauch', icon: Gauge },
  { id: 'policies', label: 'Policies', icon: ShieldCheck },
  { id: 'settings', label: 'Einstellungen', icon: Settings },
]

type Props = {
  nav?: NavItem
  /** Session-Liste links anzeigen (Standard bei `nav="sessions"`). */
  sessionList?: boolean
  activeSession?: string
  rail?: ReactNode
  children: ReactNode
  /** Verbindungsstatus unten links. */
  connection?: 'local' | 'server' | 'offline'
}

/** Grundlayout der App: Navigationsspalte, optional Session-Liste, Hauptbereich, optional Workspace-Rail. */
export function AppLayout({ nav = 'sessions', sessionList, activeSession, rail, children, connection = 'local' }: Props) {
  const showList = sessionList ?? nav === 'sessions'
  return (
    <div className="flex h-full min-h-0 bg-background text-foreground">
      <F id="WEB-001" className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-border bg-sidebar py-2" badge="bottom-left">
        {NAV.map((n) => (
          <button
            key={n.id}
            title={n.label}
            aria-label={n.label}
            aria-current={n.id === nav ? 'page' : undefined}
            className={cn(
              'relative flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground',
              n.id === nav && 'bg-accent text-foreground',
            )}
          >
            <n.icon className="size-[18px]" />
            {n.badge ? (
              <span className="chamfer-sm absolute -top-0.5 -right-0.5 min-w-4 bg-signal px-0.5 text-[10px] leading-4 font-semibold text-signal-foreground">
                {n.badge}
              </span>
            ) : null}
          </button>
        ))}
        <div className="mt-auto flex flex-col items-center gap-2 pb-1">
          <span
            title={
              connection === 'local'
                ? 'Lokaler Server auf diesem Rechner'
                : connection === 'server'
                  ? 'Verbunden mit team.example.com'
                  : 'Offline – lokale Sessions laufen weiter'
            }
            className={cn(
              'size-2 rounded-full',
              connection === 'local' && 'bg-ok',
              connection === 'server' && 'bg-voice-codex',
              connection === 'offline' && 'border border-muted-foreground',
            )}
          />
          <span className="flex size-7 items-center justify-center rounded-full bg-foreground text-[11px] font-semibold text-background">IF</span>
        </div>
      </F>
      {showList && <SessionList active={activeSession} />}
      <div className="flex min-w-0 flex-1 flex-col">{children}</div>
      {rail}
    </div>
  )
}

export function SessionList({ active }: { active?: string }) {
  return (
    <F id={['WEB-003', 'SES-012']} className="flex w-64 shrink-0 flex-col border-r border-border bg-sidebar">
      <div className="flex items-center gap-1 p-2">
        <label className="flex h-8 flex-1 items-center gap-2 rounded-md border border-input bg-card px-2 text-xs text-muted-foreground">
          <Search className="size-3.5" />
          <span>Sessions durchsuchen</span>
          <kbd className="ml-auto font-mono text-[10px]">⌘K</kbd>
        </label>
        <button title="Neue Session" aria-label="Neue Session" className="flex size-8 items-center justify-center rounded-md bg-foreground text-background">
          <Plus className="size-4" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {projects.map((p) => {
          const list = sessions.filter((s) => s.project === p.id)
          if (!list.length) return null
          return (
            <div key={p.id} className="mt-2">
              <div className="flex items-baseline justify-between px-3 pb-1">
                <span className="text-xs font-semibold">{p.name}</span>
                <span className="font-mono text-[10px] text-muted-foreground">{p.path}</span>
              </div>
              {list.map((s) => (
                <div
                  key={s.id}
                  className={cn(
                    'group flex cursor-default items-center gap-2 border-l-2 px-3 py-1.5',
                    s.id === active ? 'border-signal bg-accent' : 'border-transparent hover:bg-accent/60',
                  )}
                >
                  <StatusMark status={s.status} />
                  <div className="min-w-0 flex-1">
                    <div className={cn('truncate text-[13px]', s.unread && 'font-semibold')}>{s.title}</div>
                    <div className="flex items-center gap-1.5 overflow-hidden text-[11px] whitespace-nowrap text-muted-foreground">
                      <VoiceDot voice={harnesses[s.harness].voice} className="size-1.5" />
                      <span>{harnesses[s.harness].name}</span>
                      {s.async && <span>· im Hintergrund</span>}
                      {s.shared && <span>· geteilt</span>}
                      <span className="ml-auto shrink-0 pl-1">{s.updated}</span>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )
        })}
      </div>
    </F>
  )
}

/** Dünne Farbleiste einer Stimme, z. B. links an Nachrichten. */
export function VoiceRule({ voice }: { voice: keyof typeof voiceVar }) {
  return <span aria-hidden className="w-0.5 shrink-0 self-stretch rounded-full" style={{ background: voiceVar[voice] }} />
}
