import { AgentMessage, ToolCall, UserMessage } from '@/app/stream'
import type { ChangedFile } from './mock'
import { cn } from '@/lib/utils'

/** Kurzer Verlauf der Beispiel-Session als Kontext neben Panels. */
export function ShortStream({ compact }: { compact?: boolean }) {
  return (
    <>
      <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
      {!compact && <ToolCall kind="read" name="Lesen" target="src/routes/auth.ts" duration="0,1 s" />}
      <ToolCall kind="edit" name="Bearbeiten" target="src/middleware/rate-limit.ts" duration="0,3 s" />
      <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" duration="4,8 s" />
      <AgentMessage harness="claude">
        <p>
          Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Danach antwortet sie mit{' '}
          <code className="rounded-sm bg-muted px-1 text-[13px]">429</code>. Alle 10 Tests laufen.
        </p>
      </AgentMessage>
    </>
  )
}

const statusWord: Record<ChangedFile['status'], string> = { A: 'neu', M: 'geändert', D: 'gelöscht' }

/** Git-Status als Buchstabe mit Text (nicht nur Farbe). */
export function FileStatus({ status }: { status: ChangedFile['status'] }) {
  return (
    <span
      title={statusWord[status]}
      className={cn(
        'inline-flex size-4 shrink-0 items-center justify-center rounded-[2px] border font-mono text-[10px] font-semibold',
        status === 'A' && 'border-ok/50 text-ok',
        status === 'M' && 'border-foreground/40 text-foreground',
        status === 'D' && 'border-deny/50 text-deny',
      )}
    >
      {status}
      <span className="sr-only">{statusWord[status]}</span>
    </span>
  )
}

export function AddDel({ add, del }: { add: number; del: number }) {
  return (
    <span className="shrink-0 font-mono text-[11px] tabular-nums">
      <span className="text-ok">+{add}</span> <span className="text-deny">−{del}</span>
    </span>
  )
}
