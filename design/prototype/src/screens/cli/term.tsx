/** Kleiner Fortschrittsbalken in Textform. */
export function Bar({ pct, width = 24 }: { pct: number; width?: number }) {
  const full = Math.round((pct / 100) * width)
  return (
    <span>
      <span className="text-foreground">{'█'.repeat(full)}</span>
      <span className="text-muted-foreground">{'░'.repeat(width - full)}</span>
    </span>
  )
}
