# ADR-0016: Eingebetteter Browser – Chromium via CDP-Screencast, Umschalter auf sichtbares Fenster, Inspect-Mode → Agent

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Agents sollen einen Browser steuern (navigate, snapshot mit A11y-Tree-Refs, click, type, screenshot), und Nutzer sollen per Element-Picker ("Inspect-Mode") UI-Elemente an den Agent schicken können. Omnigent nutzt dafür Electron (Chromium überall). beton nutzt Tauri mit System-WebViews (WKWebView/WebView2/WebKitGTK), die kein einheitliches CDP bieten (ADR-0004).

## Betrachtete Optionen
1. **A — Tauri-Child-WebView** — nativ eingebettet; je OS andere Engine, keine einheitliche Automatisierung, kein Inspector-API.
2. **B — Separates, sichtbares Chromium-Fenster (CDP)** — volle Chromium-Fähigkeiten, echte DevTools, Login/OAuth problemlos; nicht im App-Layout eingebettet.
3. **C — Hybrid: Headless-Chromium mit CDP-Screencast ins App-Panel** — wirkt eingebettet, auf allen OS identisch; Latenz und Bildqualität begrenzt, DevTools nur indirekt.
4. **D — CEF einbetten** — echtes Chromium im Fenster; großer Build-/Distributionsaufwand.

## Entscheidung
**C mit Umschalter auf B** (Crate `beton-browser`, `chromiumoxide`):
- Chromium headless via CDP; Bild per `Page.startScreencast` in ein App-Panel gestreamt; Input per `Input.dispatch*` zurück.
- **Umschalter** auf ein sichtbares Chromium-Fenster für Login/OAuth und echte DevTools.
- **Inspect-Mode/Picker** via CDP `Overlay.setInspectMode`; an den Agent gehen: stabiler Selektor, `outerHTML`-Ausschnitt, Computed Styles, Bounding-Box, markierter Screenshot-Ausschnitt, Nutzerkommentar. Stretch-Goal v1: Quelldatei-Mapping (React/Vue-Komponente + `datei:zeile` via Sourcemaps/Framework-Hooks).
- **Chromium-Bezug:** installiertes Chrome/Edge/Chromium autodetektieren, sonst **Chrome for Testing** auf Nachfrage laden.
- **Agent-Tools:** navigate, snapshot (A11y-Tree mit refs), click, type, screenshot, …
- **Sicherheit:** Traffic durch den Egress-Proxy, Policies für Browser-Aktionen (Navigations-Allowlist, `ask` vor Formular-Submit), isoliertes Profil pro Session.

## Konsequenzen
- Positiv: Identisches Verhalten auf allen OS, auch im Web-/PWA-Client nutzbar (Frames über Binärkanal, ADR-0019).
- Negativ / Risiken: Screencast-Latenz; Abhängigkeit von einem Chromium außerhalb des App-Bundles; CDP-Änderungen.
- Folgearbeiten: Binärkanal für `browser.frame`, Policy-Hooks für Navigation, Download von Chrome for Testing nur nach Zustimmung.
- Verschärft durch ADR-0033: Chrome for Testing nur auf Klick; Offline-Alternative installiertes Chrome/Chromium bzw. lokale Datei.

## Bezug
- Spec: docs/spec/09-browser.md (Prefix BRW)
