import { useEffect, useRef, type ReactNode } from 'react'
import { cn } from '@/lib/utils'

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])'

/**
 * Modales Overlay über der App (Befehlspalette, Session-Switcher, Tastenkürzel) wie im Entwurf:
 * abgedunkelter Hintergrund, Inhalt oben mittig. Fokus bleibt im Overlay (Tab läuft im Kreis),
 * `Esc` und ein Klick daneben schließen, danach kehrt der Fokus zurück (WEB-016).
 */
export function Overlay({
  label,
  labelledBy,
  onClose,
  children,
  className,
  testId,
  escape = true,
}: {
  label?: string
  labelledBy?: string
  onClose: () => void
  children: ReactNode
  className?: string
  testId?: string
  /** `Esc` schließt (aus, solange der Inhalt `Esc` selbst braucht). */
  escape?: boolean
}) {
  const ref = useRef<HTMLDivElement>(null)
  const close = useRef(onClose)
  close.current = onClose
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null
    return () => {
      if (before && before.isConnected) before.focus({ preventScroll: true })
    }
  }, [])
  // `Esc` auch, wenn der Fokus gerade nicht im Overlay liegt (z. B. nach einem entfernten Knopf).
  const escapeRef = useRef(escape)
  escapeRef.current = escape
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || !escapeRef.current || e.defaultPrevented) return
      if (ref.current?.contains(e.target as Node)) return
      e.preventDefault()
      close.current()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-foreground/25 px-4 pt-[min(5rem,10vh)]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close.current()
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        aria-labelledby={labelledBy}
        data-testid={testId}
        className={cn('max-h-[calc(100dvh-6rem)] max-w-full', className)}
        onKeyDown={(e) => {
          if (e.key === 'Escape' && escape && !e.defaultPrevented) {
            e.preventDefault()
            e.stopPropagation()
            close.current()
            return
          }
          if (e.key !== 'Tab') return
          const items = [...(ref.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])].filter((el) => el.offsetParent !== null || el === document.activeElement)
          if (items.length === 0) {
            e.preventDefault()
            return
          }
          const first = items[0]!
          const last = items[items.length - 1]!
          if (e.shiftKey && document.activeElement === first) {
            e.preventDefault()
            last.focus()
          } else if (!e.shiftKey && document.activeElement === last) {
            e.preventDefault()
            first.focus()
          }
        }}
      >
        {children}
      </div>
    </div>
  )
}
