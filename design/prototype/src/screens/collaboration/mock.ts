// Beispieldaten der Gruppe Zusammenarbeit.

export type Role = 'view' | 'comment_approve' | 'drive'

export const roleLabel: Record<Role, string> = {
  view: 'Ansehen',
  comment_approve: 'Kommentieren & freigeben',
  drive: 'Mitsteuern',
}

export const roleHint: Record<Role, string> = {
  view: 'Verlauf live mitlesen und in eine eigene Session forken. Dateien nur, wenn du es unten erlaubst.',
  comment_approve: 'Zusätzlich kommentieren und Freigaben erteilen oder ablehnen.',
  drive: 'Zusätzlich Nachrichten senden, unterbrechen, Terminals nutzen, Kommentare an den Agent geben.',
}

export const people = {
  ingo: { name: 'Ingo Fahrentholz', handle: 'ingo' },
  anna: { name: 'Anna Becker', handle: 'anna' },
  jonas: { name: 'Jonas Weber', handle: 'jonas' },
  mara: { name: 'Mara Schulz', handle: 'mara' },
}

export const shares: { who: string; sub: string; role: Role | 'owner'; team?: boolean }[] = [
  { who: 'Ingo Fahrentholz', sub: 'Du · Owner', role: 'owner' },
  { who: 'Anna Becker', sub: '@anna · Team Frontend', role: 'drive' },
  { who: 'Jonas Weber', sub: '@jonas · Team Frontend', role: 'comment_approve' },
  { who: 'Team Plattform', sub: '7 Personen', role: 'view', team: true },
]

export const teamServer = 'team.example.com'
