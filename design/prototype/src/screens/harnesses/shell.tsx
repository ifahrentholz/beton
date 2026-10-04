import type { ReactNode } from 'react'
import { SettingsFrame } from '@/app/settings-shell'

export type HarnessSection = 'setup' | 'catalog' | 'acp' | 'direct-api' | 'import'

/** Einstellungen im Bereich „Harnesses“; Navigation und Sektionsliste aus `@/app/settings-shell`. */
export function HarnessSettings({ active, children }: { active: HarnessSection; children: ReactNode }) {
  return (
    <SettingsFrame active={active}>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">{children}</div>
    </SettingsFrame>
  )
}
