import type { ScreenGroup } from '@/proto/types'
import { SyncForced, SyncHome, SyncMobile, SyncOffline, SyncSettings, SyncTake } from './sync'

/**
 * D-5 · Sync & Offline: Home-Knoten (Single-Writer), Read-Replicas, Input-Forwarding,
 * geplante und erzwungene Übernahme mit Fork, Policy-Cache, Budget-Leases, Outbox.
 */
export const group: ScreenGroup = {
  id: 'sync',
  title: 'Sync & Offline',
  order: 150,
  screens: [
    {
      id: 'sync-home',
      title: 'Home-Knoten und Lesekopie',
      description:
        'An jeder Session sichtbar: wo sie läuft (Home), Epoche und Sync-Zustand. Auf anderen Geräten eine Lesekopie, deren Eingaben an den Home weitergeleitet werden.',
      features: ['SYNC-001', 'SYNC-002', 'SYNC-003', 'SYNC-008'],
      states: [
        { id: 'home-here', title: 'Home hier' },
        { id: 'replica', title: 'Lesekopie' },
        { id: 'unreachable', title: 'Home nicht erreichbar' },
        { id: 'local-only', title: 'Nur lokal' },
      ],
      component: SyncHome,
    },
    {
      id: 'sync-take',
      title: 'Session hierher holen',
      description: 'Geplante Übernahme, z. B. vor einer Zugfahrt: wartet auf das Turn-Ende (oder unterbricht), überträgt alles, startet den Agent hier neu.',
      features: ['SYNC-004', 'SYNC-001'],
      states: [
        { id: 'confirm', title: 'Bestätigen' },
        { id: 'waiting', title: 'Wartet auf Turn-Ende' },
        { id: 'done', title: 'Übernommen' },
      ],
      component: SyncTake,
    },
    {
      id: 'sync-forced',
      title: 'Erzwungene Übernahme und Fork',
      description:
        'Ist der Home nicht erreichbar, kann der Owner erzwingen. Beim Wiedersehen: bestätigt, wenn nur einer schrieb – sonst automatischer Fork, nie ein Merge.',
      features: ['SYNC-005', 'SYNC-001'],
      states: [
        { id: 'force-dialog', title: 'Erzwingen' },
        { id: 'provisional', title: 'Vorläufig hier' },
        { id: 'confirmed', title: 'Bestätigt' },
        { id: 'fork', title: 'Divergenz → Fork' },
      ],
      component: SyncForced,
    },
    {
      id: 'sync-offline',
      title: 'Offline arbeiten',
      description:
        'Offline ist ein vollwertiger Zustand: Policy-Cache (nur verschärfbar, mit Ablauf), Budget-Lease, und beim Reconnect Abgleich samt zugestellter oder verworfener Outbox-Einträge.',
      features: ['SYNC-002', 'SYNC-006', 'SYNC-007', 'SYNC-008', 'POL-008', 'POL-021'],
      states: [
        { id: 'offline', title: 'Offline' },
        { id: 'stale', title: 'Policy-Cache veraltet' },
        { id: 'reconnect', title: 'Wieder verbunden' },
      ],
      component: SyncOffline,
    },
    {
      id: 'sync-mobile',
      title: 'Vom Handy: Weiterleitung und Outbox',
      description: 'Freigaben und Nachrichten vom Handy gehen an den Rechner, auf dem die Session läuft; ist er offline, warten sie als „ausstehend“.',
      features: ['SYNC-003', 'SYNC-008'],
      frame: 'mobile',
      states: [
        { id: 'forwarded', title: 'Zugestellt' },
        { id: 'home-offline', title: 'Rechner offline' },
        { id: 'pending', title: 'Ausstehend' },
        { id: 'discarded', title: 'Verworfen' },
      ],
      component: SyncMobile,
    },
    {
      id: 'sync-settings',
      title: 'Team-Server & Sync (Einstellungen)',
      description: 'Ob und welche Sessions synchronisiert werden, Stand des Policy-Bundles und der Budget-Lease. Ohne Team-Server: alles lokal.',
      features: ['SYNC-001', 'SYNC-002', 'SYNC-006', 'SYNC-007'],
      states: [
        { id: 'connected', title: 'Verbunden' },
        { id: 'off', title: 'Kein Team-Server' },
      ],
      component: SyncSettings,
    },
  ],
}
