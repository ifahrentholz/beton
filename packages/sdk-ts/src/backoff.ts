/**
 * Reconnect-Backoff wie `beton_proto::ws::Backoff` (PROTO-009): 0,5 s → 30 s exponentiell,
 * ±20 % Jitter. Nach Close 4503 (geordnetes Herunterfahren) sofort, aber gleichverteilt
 * über {@link SHUTDOWN_JITTER_MS}, damit nicht alle Clients gleichzeitig kommen.
 */
export const BACKOFF_BASE_MS = 500
export const BACKOFF_MAX_MS = 30_000
export const SHUTDOWN_JITTER_MS = 2_000

export class Backoff {
  private attempt = 0

  constructor(
    private readonly baseMs: number = BACKOFF_BASE_MS,
    private readonly maxMs: number = BACKOFF_MAX_MS,
  ) {}

  /** Wartezeit für den nächsten Versuch; `jitter` ∈ [-1, 1] skaliert ±20 %. */
  nextWith(jitter: number): number {
    const base = this.baseMs * 2 ** Math.min(this.attempt, 16)
    const capped = Math.min(base, this.maxMs)
    this.attempt += 1
    const j = Math.max(-1, Math.min(1, jitter))
    return Math.max(0, capped * (1 + 0.2 * j))
  }

  next(): number {
    return this.nextWith(Math.random() * 2 - 1)
  }

  reset(): void {
    this.attempt = 0
  }
}

/** Wartezeit nach Close 4503. */
export function shutdownDelay(random: number = Math.random()): number {
  return SHUTDOWN_JITTER_MS * Math.max(0, Math.min(1, random))
}
