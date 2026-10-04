/**
 * TypeScript-SDK für die beton-API (API-004). Typen sind aus den Rust-Typen generiert
 * (`src/gen`, `cargo xtask codegen`); Client und Event-Strom sind handgeschrieben.
 */
export type * from './gen/index.js'
export { BetonClient, Session, type ClientOptions, type SessionListOptions } from './client.js'
export { BetonError, type Problem } from './errors.js'
export { Backoff, BACKOFF_BASE_MS, BACKOFF_MAX_MS, SHUTDOWN_JITTER_MS, shutdownDelay } from './backoff.js'
export {
  eventStream,
  defaultWebSocketFactory,
  SUBPROTOCOL,
  PROTOCOL_VERSION,
  type StreamOptions,
  type WebSocketFactory,
  type WebSocketLike,
} from './stream.js'
