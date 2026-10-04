import type { ChangedFile } from './mock'
import { cn } from '@/lib/utils'

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
