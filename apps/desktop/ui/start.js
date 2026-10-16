// Startbildschirm (DESK-002, Entwurf `desktop-daemon` Zustand „Start“) und Dialog für einen
// inkompatiblen Daemon (Zustand „Daemon inkompatibel“). Den Stand liefert die App; sobald sie
// verbunden ist, öffnet sie in diesem Fenster die Web-UI des Daemons.
import { el, invoke } from './ipc.js'

const steps = document.getElementById('steps')
const dialog = document.getElementById('incompatible')
let last = ''

function render(state) {
  const key = JSON.stringify(state)
  if (key === last) return
  last = key
  document.getElementById('home').textContent = state.home
  steps.replaceChildren(
    ...state.steps.map((s) => el('li', { class: s.state }, el('span', { class: `mark ${s.state}`, 'aria-hidden': 'true' }), s.text)),
  )
  const inc = state.incompatible
  dialog.hidden = !inc
  if (inc) {
    document.getElementById('incompatible-text').textContent =
      `Daemon ${inc.daemon} (Protokoll ${inc.daemon_protocol}) läuft noch, die App ist ${inc.app} (Protokoll ${inc.app_protocol}). Der Abstand ist zu groß, um sicher zu verbinden.`
    document.getElementById('incompatible-hint').textContent = inc.waiting
      ? `Der Neustart wartet, bis keine Session mehr arbeitet (${inc.waiting} arbeiten gerade); deine Sessions bleiben erhalten.`
      : `beton kann den mitgelieferten Daemon ${inc.app} starten. Der Neustart wartet, bis keine Session mehr arbeitet; deine Sessions bleiben erhalten.`
    const upgrade = document.getElementById('upgrade')
    upgrade.textContent = `Daemon auf ${inc.app} aktualisieren`
    upgrade.disabled = inc.upgrading
  }
}

async function tick() {
  try {
    render(await invoke('startup_state'))
  } catch {
    // Die App hat das Fenster schon zur Web-UI navigiert.
  }
}

document.getElementById('upgrade').addEventListener('click', () => void invoke('daemon_upgrade'))
document.getElementById('quit').addEventListener('click', () => void invoke('quit_now'))
void tick()
setInterval(tick, 200)
