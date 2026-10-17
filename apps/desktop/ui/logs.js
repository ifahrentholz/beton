// Daemon-Logs (DESK-002, Entwurf `desktop-daemon` Zustand „Logs“): die letzten Zeilen von
// ~/.beton/logs/daemon.log, alle 2 s neu gelesen.
import { invoke } from './ipc.js'

const pre = document.getElementById('lines')

async function load() {
  const { path, lines } = await invoke('daemon_logs')
  document.getElementById('path').textContent = path
  const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 20
  pre.textContent = lines.length ? lines.join('\n') : 'Noch keine Einträge.'
  if (atBottom) pre.scrollTop = pre.scrollHeight
}

document.getElementById('copy').addEventListener('click', () => void navigator.clipboard.writeText(pre.textContent ?? ''))
document.getElementById('close').addEventListener('click', () => void invoke('close_self'))
await load()
pre.scrollTop = pre.scrollHeight
setInterval(load, 2000)
