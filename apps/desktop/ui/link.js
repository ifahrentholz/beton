// Hinweis bei einem Link auf einen anderen Server bzw. zum Koppeln (DESK-006 AC2, Entwurf
// `desktop-deeplink` Zustand „Server-Link vor M4“). Bis zu den Server-Profilen (DESK-003) gibt
// es nichts zuzustimmen: Die App öffnet den Link nicht und verbindet sich nicht.
import { el, invoke } from './ipc.js'

const info = await invoke('link_info')
if (info) {
  document.getElementById('link').textContent = info.link
  const text = document.getElementById('text')
  const host = el('span', { class: 'strong' }, info.host)
  text.replaceChildren(
    info.kind === 'pair' ? 'Dieser Link will diesen Rechner mit ' : 'Dieser Link gehört zu ',
    host,
    info.kind === 'pair' ? ' koppeln. ' : '. ',
    'Diese Version von beton arbeitet nur mit dem lokalen Daemon auf diesem Rechner; weitere Server kommen mit einer späteren Version.',
  )
}
document.getElementById('ok').addEventListener('click', () => void invoke('close_self'))
document.getElementById('ok').focus()
