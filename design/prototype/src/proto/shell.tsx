import { useEffect, useState, type ReactNode } from 'react'
import { Link, Outlet, useRouterState } from '@tanstack/react-router'
import { Moon, Sun, Tags } from 'lucide-react'
import { cn } from '@/lib/utils'
import { FeatureOverlayContext } from './feature-marker'
import { coverageStats, groups } from './registry'

function readPref(key: string, fallback: string) {
  try {
    return localStorage.getItem(key) ?? fallback
  } catch {
    return fallback
  }
}
function writePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* privater Modus o. Ä. – Einstellung gilt dann nur für diese Sitzung */
  }
}

export function Wordmark({ className }: { className?: string }) {
  // Die Wortmarke: breit gesetzt, der Taktstock als schräger Strich über dem „t“.
  return (
    <span className={cn('type-wide relative inline-flex items-baseline font-[750] tracking-tight', className)}>
      be
      <span className="relative">
        t
        <span aria-hidden className="absolute top-[0.2em] -left-[0.12em] h-[0.11em] w-[0.72em] -rotate-[22deg] rounded-full bg-signal" />
      </span>
      on
    </span>
  )
}

function Toggle({ on, onClick, label, children }: { on: boolean; onClick: () => void; label: string; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        'inline-flex h-7 items-center gap-1.5 rounded-md border px-2 text-xs transition-colors',
        on ? 'border-signal bg-signal-soft text-foreground' : 'border-border text-muted-foreground hover:bg-accent hover:text-foreground',
      )}
    >
      {children}
      {label}
    </button>
  )
}

export function DesignShell() {
  const [dark, setDark] = useState(() => readPref('beton-proto-theme', 'light') === 'dark')
  const [overlay, setOverlay] = useState(() => readPref('beton-proto-overlay', 'off') === 'on')
  const path = useRouterState({ select: (s) => s.location.pathname })
  const stats = coverageStats()

  useEffect(() => {
    document.documentElement.classList.toggle('dark', dark)
    writePref('beton-proto-theme', dark ? 'dark' : 'light')
  }, [dark])
  useEffect(() => writePref('beton-proto-overlay', overlay ? 'on' : 'off'), [overlay])

  return (
    <FeatureOverlayContext.Provider value={overlay}>
      <div className="flex h-screen flex-col">
        <header className="flex h-12 shrink-0 items-center gap-4 border-b border-border bg-sidebar px-4">
          <Link to="/" className="flex items-baseline gap-2">
            <Wordmark className="text-xl" />
            <span className="text-xs text-muted-foreground">Design-Prototyp</span>
          </Link>
          <nav className="flex items-center gap-1 text-sm">
            <Link to="/" className="rounded-md px-2 py-1 hover:bg-accent [&.active]:bg-accent" activeOptions={{ exact: true }}>
              Übersicht
            </Link>
            <Link to="/catalog" className="rounded-md px-2 py-1 hover:bg-accent [&.active]:bg-accent">
              Feature-Katalog
            </Link>
          </nav>
          <span className="ml-auto text-xs text-muted-foreground tabular-nums">
            {stats.withScreen + stats.noUi} von {stats.total} Features zugeordnet
          </span>
          <Toggle on={overlay} onClick={() => setOverlay((v) => !v)} label="Feature-IDs">
            <Tags className="size-3.5" />
          </Toggle>
          <Toggle on={dark} onClick={() => setDark((v) => !v)} label={dark ? 'Dunkel' : 'Hell'}>
            {dark ? <Moon className="size-3.5" /> : <Sun className="size-3.5" />}
          </Toggle>
        </header>
        <div className="flex min-h-0 flex-1">
          <aside className="w-64 shrink-0 overflow-y-auto border-r border-border bg-sidebar py-3">
            {groups.map((g) => (
              <div key={g.id} className="mb-3">
                <div className="px-4 pb-1 text-xs font-semibold text-muted-foreground">{g.title}</div>
                {g.screens.map((s) => {
                  const active = path === `/s/${s.id}`
                  return (
                    <Link
                      key={s.id}
                      to="/s/$screenId"
                      params={{ screenId: s.id }}
                      className={cn(
                        'flex items-baseline justify-between gap-2 border-l-2 px-4 py-1 text-[13px]',
                        active ? 'border-signal bg-accent font-medium' : 'border-transparent hover:bg-accent/60',
                      )}
                    >
                      <span className="truncate">{s.title}</span>
                      <span className="font-mono text-[10px] text-muted-foreground">{s.features.length}</span>
                    </Link>
                  )
                })}
              </div>
            ))}
          </aside>
          <main className="min-w-0 flex-1 overflow-auto">
            <Outlet />
          </main>
        </div>
      </div>
    </FeatureOverlayContext.Provider>
  )
}
