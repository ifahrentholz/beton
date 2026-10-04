import { BetonClient } from '@beton/sdk'

/**
 * Client gegen den eigenen Origin: Die Web-UI wird von `beton serve` ausgeliefert und
 * authentisiert sich über das Session-Cookie aus `beton open` (AUTH-004).
 */
export const client = new BetonClient({ baseUrl: window.location.origin })
