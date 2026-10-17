import { useNavigate } from '@tanstack/react-router'
import { Gauge, Inbox, Moon, Plus, Settings } from 'lucide-react'
import { tk, useCommand } from '@/lib/commands'
import { toggleMode } from '@/lib/theme'
import { openOnly, useUi } from './ui-state'

/**
 * Befehle, die überall gelten (UX-002, UX-004): Palette, Switcher, Tastenkürzel, neue Session,
 * Theme und Navigation zu den Seiten. Session-Befehle bindet die Session-Ansicht.
 */
export function useGlobalCommands({ inboxCount }: { inboxCount: number }): void {
  const navigate = useNavigate()
  const toggle = (which: 'palette' | 'switcher' | 'shortcuts') => {
    const s = useUi.getState()
    if (which === 'palette' ? s.palette : which === 'switcher' ? s.switcher : s.shortcuts) s.set({ [which]: false })
    else openOnly(which)
  }
  useCommand({ id: 'palette.open', run: () => toggle('palette') })
  useCommand({ id: 'switcher.open', run: () => toggle('switcher') })
  useCommand({ id: 'shortcuts.open', run: () => toggle('shortcuts') })
  useCommand({
    id: 'session.new',
    icon: Plus,
    run: () => useUi.getState().set({ newSession: true, palette: false, switcher: false, shortcuts: false }),
  })
  useCommand({ id: 'theme.toggle', title: tk('cmd.theme'), icon: Moon, group: 'general', order: 3, keywords: ['theme', 'dunkel', 'hell', 'dark', 'light'], run: toggleMode })
  useCommand({
    id: 'goto.inbox',
    icon: Inbox,
    hint: inboxCount > 0 ? (t) => t('cmd.inboxHint', { count: inboxCount }) : undefined,
    run: () => void navigate({ to: '/inbox' }),
  })
  useCommand({ id: 'goto.usage', title: tk('cmd.usage'), icon: Gauge, group: 'goto', order: 4, keywords: ['usage', 'verbrauch', 'kosten', 'cost'], run: () => void navigate({ to: '/usage' }) })
  useCommand({ id: 'goto.settings', icon: Settings, run: () => void navigate({ to: '/settings/appearance' }) })
}
