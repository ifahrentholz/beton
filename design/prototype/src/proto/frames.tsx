import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import type { Frame } from './types'

/** Rahmen, in dem ein Screen gezeigt wird: Desktop-Fenster, Handy, Terminal oder ohne. */
export function ScreenFrame({ frame = 'desktop', title, children }: { frame?: Frame; title: string; children: ReactNode }) {
  if (frame === 'none') return <div className="h-full">{children}</div>
  if (frame === 'mobile') return <MobileFrame>{children}</MobileFrame>
  if (frame === 'terminal') return <TerminalFrame title={title}>{children}</TerminalFrame>
  return <DesktopFrame title={title}>{children}</DesktopFrame>
}

function WindowDots() {
  return (
    <div className="flex gap-1.5" aria-hidden>
      <span className="size-3 rounded-full bg-[#ec6a5e]" />
      <span className="size-3 rounded-full bg-[#f4bf4f]" />
      <span className="size-3 rounded-full bg-[#61c554]" />
    </div>
  )
}

export function DesktopFrame({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex h-full min-h-[640px] flex-col overflow-hidden rounded-lg border border-border bg-background shadow-[0_1px_0_var(--border),0_18px_40px_-24px_rgb(0_0_0/0.45)]">
      <div className="flex h-9 shrink-0 items-center gap-3 border-b border-border bg-sidebar px-3">
        <WindowDots />
        <span className="flex-1 truncate text-center text-xs text-muted-foreground">{title} — beton</span>
        <span className="w-[54px]" />
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  )
}

export function MobileFrame({ children }: { children: ReactNode }) {
  return (
    <div className="mx-auto flex h-[760px] w-[372px] flex-col overflow-hidden rounded-[36px] border-[10px] border-[#24272b] bg-background shadow-xl">
      <div className="flex h-7 shrink-0 items-center justify-between px-6 text-[11px] font-medium">
        <span>9:41</span>
        <span className="h-4 w-20 rounded-full bg-[#24272b]" />
        <span>5G</span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">{children}</div>
    </div>
  )
}

/** Terminal-Fenster: immer dunkel (Klasse `dark`), Farben aus den Tokens der aktiven Theme-Familie. */
export function TerminalFrame({ title, children, className }: { title: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn('dark flex h-full min-h-[560px] flex-col overflow-hidden rounded-lg border border-border bg-sunken text-foreground shadow-lg', className)}>
      <div className="flex h-8 shrink-0 items-center gap-3 border-b border-border px-3">
        <WindowDots />
        <span className="flex-1 truncate text-center font-mono text-[11px] text-muted-foreground">{title}</span>
        <span className="w-[54px]" />
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-4 font-mono text-[12.5px] leading-[1.55]">{children}</div>
    </div>
  )
}
