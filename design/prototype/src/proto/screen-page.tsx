import { Link, useNavigate } from '@tanstack/react-router'
import { featureById, specUrl } from '@/catalog/types'
import { cn } from '@/lib/utils'
import { ScreenFrame } from './frames'
import { groupOfScreen, screenById } from './registry'

export function ScreenPage({ screenId, state }: { screenId: string; state?: string }) {
  const screen = screenById.get(screenId)
  const navigate = useNavigate()
  if (!screen) {
    return (
      <div className="p-8">
        <p>Diesen Screen gibt es nicht.</p>
        <Link to="/" className="underline">
          Zur Übersicht
        </Link>
      </div>
    )
  }
  const states = screen.states ?? [{ id: 'default', title: 'Standard' }]
  const current = states.find((s) => s.id === state) ?? states[0]
  const Component = screen.component
  const group = groupOfScreen.get(screen.id)

  return (
    <div className="flex min-h-full flex-col gap-4 p-6">
      <div className="flex flex-wrap items-start gap-x-8 gap-y-3">
        <div className="max-w-2xl">
          <div className="text-xs text-muted-foreground">{group?.title}</div>
          <h1 className="type-wide text-2xl font-[700]">{screen.title}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{screen.description}</p>
        </div>
        {states.length > 1 && (
          <div role="tablist" aria-label="Zustand" className="flex flex-wrap gap-1 self-end">
            {states.map((s) => (
              <button
                key={s.id}
                role="tab"
                aria-selected={s.id === current.id}
                onClick={() =>
                  navigate({ to: '/s/$screenId', params: { screenId: screen.id }, search: { state: s.id } })
                }
                className={cn(
                  'rounded-md border px-2.5 py-1 text-xs',
                  s.id === current.id ? 'border-foreground bg-foreground text-background' : 'border-border hover:bg-accent',
                )}
              >
                {s.title}
              </button>
            ))}
          </div>
        )}
      </div>
      <details className="group text-xs">
        <summary className="w-fit cursor-pointer text-muted-foreground select-none hover:text-foreground">
          Zeigt {screen.features.length} Features
        </summary>
        <div className="mt-2 flex flex-wrap gap-1">
        {screen.features.map((id) => {
          const f = featureById.get(id)
          return (
            <a
              key={id}
              href={f ? specUrl(f) : undefined}
              target="_blank"
              rel="noreferrer"
              title={f?.summary}
              className={cn(
                'rounded-sm border px-1.5 py-0.5 font-mono text-[11px]',
                f ? 'border-border text-muted-foreground hover:border-foreground hover:text-foreground' : 'border-deny text-deny',
              )}
            >
              {id}
              {f && <span className="ml-1 font-sans text-[11px]">{f.title}</span>}
            </a>
          )
        })}
        </div>
      </details>
      <div className="min-h-0 flex-1">
        <ScreenFrame frame={screen.frame} title={screen.title}>
          <Component state={current.id} />
        </ScreenFrame>
      </div>
    </div>
  )
}
