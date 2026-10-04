import type { ReactNode } from 'react'
import { Lock } from 'lucide-react'

/** Browser-Fenster im Desktop-Rahmen (für Login-, Pairing- und Einmal-Link-Seiten). */
export function BrowserChrome({ url, children, secure = true, overlay }: { url: string; children: ReactNode; secure?: boolean; overlay?: ReactNode }) {
  return (
    <div className="relative flex h-full flex-col bg-background">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-sidebar px-3">
        <span className="flex gap-1 text-muted-foreground" aria-hidden>
          ‹ ›
        </span>
        <div className="flex h-7 flex-1 items-center gap-2 rounded-md border border-input bg-card px-2 font-mono text-[12px]">
          {secure ? <Lock className="size-3 text-muted-foreground" /> : <span className="text-[11px] text-muted-foreground">http</span>}
          <span className="truncate">{url}</span>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">{children}</div>
      {overlay}
    </div>
  )
}

/** Einfaches QR-artiges Muster (nur Darstellung). */
export function FakeQr({ size = 168, seed = 7 }: { size?: number; seed?: number }) {
  const n = 25
  const cells: boolean[] = []
  let x = seed * 9301 + 49297
  for (let i = 0; i < n * n; i++) {
    x = (x * 9301 + 49297) % 233280
    cells.push(x / 233280 > 0.52)
  }
  const finder = (r: number, c: number) => {
    const inBox = (r0: number, c0: number) => r >= r0 && r < r0 + 7 && c >= c0 && c < c0 + 7
    for (const [r0, c0] of [
      [0, 0],
      [0, n - 7],
      [n - 7, 0],
    ]) {
      if (inBox(r0, c0)) {
        const rr = r - r0
        const cc = c - c0
        return rr === 0 || rr === 6 || cc === 0 || cc === 6 || (rr >= 2 && rr <= 4 && cc >= 2 && cc <= 4)
      }
    }
    return null
  }
  const s = size / n
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label="QR-Code zum Koppeln" className="bg-card p-0">
      {cells.map((on, i) => {
        const r = Math.floor(i / n)
        const c = i % n
        const f = finder(r, c)
        const fill = f === null ? on : f
        return fill ? <rect key={i} x={c * s} y={r * s} width={s} height={s} fill="var(--foreground)" /> : null
      })}
    </svg>
  )
}
