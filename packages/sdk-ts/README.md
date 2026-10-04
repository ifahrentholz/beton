# @ifahrentholz/beton-sdk

TypeScript-SDK für die beton-API (API-004). ESM, Node ≥ 20 und moderne Browser, ohne Laufzeit-Abhängigkeiten.

```ts
import { BetonClient } from '@ifahrentholz/beton-sdk'

const c = new BetonClient({ baseUrl: 'http://127.0.0.1:7420', token })
const s = await c.sessions.create({ target: 'claude', cwd: '/pfad/zum/projekt' })
await s.send('Hallo')
for await (const ev of s.events({ fromSeq: 0 })) {
  if (ev.type === 'message.delta') process.stdout.write(ev.payload.text)
}
```

- `events()` verbindet bei Abbrüchen automatisch neu (Backoff wie `beton_proto::ws::Backoff`: 0,5 s → 30 s, ±20 %; nach Close 4503 höchstens 2 s Jitter) und attacht ab der zuletzt gesehenen `seq`: keine Lücken, keine Duplikate.
- Im Browser authentisiert das Session-Cookie (`beton open`), in Node das lokale Token. Unter Node 20 (ohne globales `WebSocket`) eine `webSocketFactory` mit dem Paket `ws` übergeben.
- `src/gen/` enthält die aus den Rust-Typen generierten Typen (PROTO-013): Event-Modell, WebSocket-Nachrichten und REST-Modelle. Diese Dateien entstehen ausschließlich mit `cargo xtask codegen`; die CI prüft das mit `cargo xtask codegen --check`.

Befehle: `pnpm test` · `pnpm typecheck` · `pnpm build`.
