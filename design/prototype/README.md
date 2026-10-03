# beton – Design-Prototyp

Klickbarer Prototyp aller Screens und Zustände von beton, mit Beispieldaten und ohne Backend (ADR-0032). Nach Abnahme durch den Maintainer ist er die verbindliche Vorlage für die UI-Arbeitspakete.

```bash
pnpm install
pnpm dev          # http://localhost:5173
pnpm test         # prüft, dass jedes Feature der Spec zugeordnet ist
pnpm typecheck
pnpm build        # statischer Build in dist/ (Hash-Routing, relative Pfade)
pnpm gen:features # src/catalog/features.json aus docs/spec neu erzeugen
```

## Aufbau

| Pfad | Inhalt |
|---|---|
| `src/proto/` | Rahmen des Prototyps: Shell, Registry, Katalog, Feature-Marker, Fensterrahmen. Nicht für Screens ändern. |
| `src/app/` | Gemeinsame App-Bausteine: `AppLayout`, `SessionList`, `SessionHeader`, `Composer`, `WorkspaceRail`, Stream-Elemente, `Score` (Partitur), `HarnessBadge`, `StatusMark`. |
| `src/components/ui/` | shadcn/ui-Komponenten (Radix). |
| `src/mock/data.ts` | Gemeinsame Beispieldaten (Harnesses, Projekte, Sessions). |
| `src/screens/<gruppe>/index.tsx` | Eine Screen-Gruppe; exportiert `group: ScreenGroup`. Wird automatisch eingesammelt. |
| `src/catalog/features.json` | Generiert aus der Spec, nie von Hand ändern. |

Referenz für Aufbau und Stil: `src/screens/session/index.tsx`.

## Regeln für Screens

1. **Jedes Feature wird zugeordnet:** entweder ein Screen führt die ID in `features` und markiert den Bereich mit `<F id="…">`, oder die Gruppe trägt sie in `noUi` ein – mit Begründung und, wenn möglich, `visibleIn` (Screen, in dem man die Wirkung sieht). `pnpm test` muss grün sein.
2. **Zustände statt Varianten-Screens:** leer, lädt, Fehler, offline, Freigabe offen … über `states` und die `state`-Prop.
3. **Design-System einhalten:**
   - Farben nur über Tokens (`bg-card`, `text-muted-foreground`, `bg-signal`, `text-deny`, `bg-voice-claude` …), nie Hex-Werte.
   - **Schalungsgelb (`signal`) heißt „du bist dran“** – nur für offene Freigaben, Fokus und die eine Primäraktion eines Screens. Nicht als Deko.
   - **Fase (`chamfer`, `chamfer-sm`)** nur an Elementen, die auf den Menschen warten.
   - Stimmfarben (`voice-*`) kennzeichnen Harness-Familien; überall gleich.
   - Status nie nur über Farbe (Form + Text, siehe `StatusMark`).
   - Typo: Archivo; `type-wide` für Bereichsköpfe, `type-narrow` für dichte Daten; Code in IBM Plex Mono. Satzanfang groß, keine Versal-Labels.
   - Keine Karten-Raster aus identischen Boxen mit Schatten; Struktur über Linien, Abstände, Ausrichtung.
4. **Texte:** Deutsch, aus Sicht der Nutzer, aktiv, konkret. Buttons sagen, was passiert („Session teilen“, nicht „OK“). Fehler erklären Ursache und nächsten Schritt.
5. **Prämissen sichtbar machen:** Alles läuft lokal ohne externe Server (Ausnahme: Modell-Anbieter hinter den CLIs); Subscriptions über die offiziellen CLIs sind der Normalfall, API-Keys nur eine Option. Netzwerkzugriffe (Updates, Downloads, Telemetrie, Web-Push) sind opt-in und so beschriftet.
6. **Gemeinsame Dateien** (`src/app`, `src/proto`, `src/mock/data.ts`) nur ändern, wenn es nicht anders geht; eigene Beispieldaten und Hilfskomponenten in die eigene Gruppe legen.
