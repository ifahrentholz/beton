import type { Command } from './commands'
import { tk } from './i18n'

/**
 * Deklarierte Befehle mit Standard-Kürzeln (UX-004, Entwurf `inbox-shortcuts`). Ausgeführt
 * werden sie dort, wo sie wirken (`useCommand`); das Overlay listet sie auch, solange sie gerade
 * nicht verfügbar sind. Die Freigabe-Kürzel stammen aus der Freigabe-Leiste (WEB-007), die
 * Inbox-Tasten aus UX-001.
 */
export const DECLARED: (Partial<Command> & { id: string })[] = [
  // Allgemein
  { id: 'palette.open', title: tk('cmd.palette'), shortcut: 'Mod+K', section: 'general', order: 0, globalInInput: true, palette: false },
  { id: 'session.new', title: tk('cmd.newSession'), shortcut: 'Mod+N', section: 'general', group: 'general', order: 1, keywords: ['new', 'neu'] },
  { id: 'switcher.open', title: tk('cmd.switcher'), shortcut: 'Mod+Alt+S', section: 'general', order: 2, globalInInput: true, palette: false },
  { id: 'goto.inbox', title: tk('cmd.inbox'), shortcut: 'Mod+Shift+I', section: 'general', group: 'goto', order: 3, keywords: ['inbox', 'freigaben', 'approvals'] },
  { id: 'goto.settings', title: tk('cmd.settings'), shortcut: 'Mod+,', section: 'general', group: 'goto', order: 5, keywords: ['settings', 'einstellungen'] },
  { id: 'shortcuts.open', title: tk('cmd.shortcuts'), shortcut: 'Mod+/', section: 'general', order: 6, palette: false },
  // Session
  { id: 'session.send', title: tk('cmd.send'), shortcut: 'Enter', section: 'session', order: 0, local: true },
  { id: 'session.interrupt', title: tk('cmd.interrupt'), shortcut: 'Escape Escape', section: 'session', group: 'session', order: 3, globalInInput: true, keywords: ['interrupt', 'stop', 'stopp'] },
  { id: 'session.model', title: tk('cmd.model'), shortcut: 'Mod+Shift+M', section: 'session', group: 'session', order: 5, keywords: ['model', 'modell'] },
  { id: 'session.fork', title: tk('cmd.fork'), shortcut: 'Mod+Shift+F', section: 'session', group: 'session', order: 0, keywords: ['fork', 'abzweigen'] },
  { id: 'session.rename', title: tk('cmd.rename'), shortcut: 'F2', section: 'session', order: 6, palette: false },
  // Freigaben (WEB-007)
  { id: 'approval.allow', title: tk('cmd.approvalAllow'), shortcut: 'Mod+Enter', section: 'approvals', order: 0, globalInInput: true, capture: true, palette: false },
  { id: 'approval.allowSession', title: tk('cmd.approvalAllowSession'), shortcut: 'Mod+Shift+Enter', section: 'approvals', order: 1, globalInInput: true, capture: true, palette: false },
  { id: 'approval.deny', title: tk('cmd.approvalDeny'), shortcut: 'Mod+Backspace', section: 'approvals', order: 2, globalInInput: true, capture: true, palette: false },
  { id: 'approval.focus', title: tk('cmd.approvalFocus'), shortcut: 'Mod+Shift+A', section: 'approvals', order: 3, globalInInput: true, capture: true, palette: false },
  { id: 'approval.step', title: tk('cmd.approvalStep'), shortcut: 'ArrowDown / ArrowUp', section: 'approvals', order: 4, local: true },
  { id: 'approval.back', title: tk('cmd.approvalBack'), shortcut: 'Escape', section: 'approvals', order: 5, local: true },
  // Inbox (UX-001)
  { id: 'inbox.step', title: tk('cmd.inboxStep'), shortcut: 'J / K', section: 'inbox', order: 0, local: true },
  { id: 'inbox.allow', title: tk('cmd.inboxAllow'), shortcut: 'A', section: 'inbox', order: 1, local: true },
  { id: 'inbox.deny', title: tk('cmd.inboxDeny'), shortcut: 'D', section: 'inbox', order: 2, local: true },
  { id: 'inbox.open', title: tk('cmd.inboxOpen'), shortcut: 'Enter', section: 'inbox', order: 3, local: true },
]
