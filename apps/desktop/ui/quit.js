// Beenden mit laufenden Sessions (DESK-002, Entwurf `desktop-daemon` Zustand „Beenden“).
// Default: Daemon weiterlaufen lassen.
import { invoke } from './ipc.js'

const names = await invoke('working_sessions')
const quoted = names.map((n) => `„${n}“`)
const list = quoted.length > 1 ? `${quoted.slice(0, -1).join(', ')} und ${quoted.at(-1)}` : (quoted[0] ?? '')
document.getElementById('working').textContent =
  names.length === 1 ? `1 Session arbeitet gerade: ${list}.` : `${names.length} Sessions arbeiten gerade: ${list}.`

document.getElementById('confirm').addEventListener('click', () => {
  const stop = document.querySelector('input[name="daemon"]:checked')?.value === 'stop'
  void invoke('quit_choice', { stopDaemon: stop })
})
document.getElementById('cancel').addEventListener('click', () => void invoke('close_self'))
document.getElementById('confirm').focus()
