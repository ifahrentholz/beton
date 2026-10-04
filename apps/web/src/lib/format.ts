/** Dauer in Sekunden mit einer Nachkommastelle. */
export function seconds(ms: number | undefined): string | undefined {
  if (ms === undefined) return undefined
  return `${(ms / 1000).toLocaleString('de-DE', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} s`
}

/** Kurzform von Tool-Argumenten, z. B. der Befehl oder Dateipfad. */
export function argSummary(args: unknown): string {
  if (args && typeof args === 'object') {
    const a = args as Record<string, unknown>
    for (const k of ['command', 'file_path', 'path', 'url', 'pattern', 'query']) {
      if (typeof a[k] === 'string') return (a[k] as string).split('\n')[0] ?? ''
    }
  }
  const s = JSON.stringify(args ?? '')
  return s === '{}' || s === '""' ? '' : s.slice(0, 120)
}

/** Kosten aus Mikro-Einheiten (USD intern). */
export function cost(micro: number): string {
  return `${(micro / 1_000_000).toLocaleString('de-DE', { style: 'currency', currency: 'USD', maximumFractionDigits: 2 })}`
}

/** Relative Zeit, z. B. „vor 3 Min.“. */
export function ago(iso: string, now: number = Date.now()): string {
  const diff = Math.max(0, now - Date.parse(iso))
  const min = Math.floor(diff / 60_000)
  if (min < 1) return 'gerade eben'
  if (min < 60) return `vor ${min} Min.`
  const h = Math.floor(min / 60)
  if (h < 24) return `vor ${h} Std.`
  return new Date(iso).toLocaleDateString('de-DE')
}
