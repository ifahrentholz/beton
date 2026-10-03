# ADR-0020: Frontend-Stack – React 19 + TypeScript + Vite + TanStack + Tailwind/shadcn + Zustand

- **Status:** Akzeptiert
- **Datum:** 2026-10-03
- **Entscheider:** Ingo Fahrentholz

## Kontext
Eine Frontend-Codebasis wird in Tauri (ADR-0004) und als Web-UI/PWA vom Server ausgeliefert. Benötigt werden u. a. Code-Editor (Monaco), Terminal (xterm.js), Syntax-Highlighting, Diff-Ansicht, Sub-Agent-Graph und performantes Streaming langer Transkripte. Omnigent nutzt Vite + React + TS, TanStack Query, Zustand, Tailwind/shadcn, Monaco, xterm, Shiki, xyflow. Entwicklung erfolgt überwiegend durch Coding-Agents, die mit verbreiteten Stacks am zuverlässigsten arbeiten.

## Betrachtete Optionen
1. **A — React 19 + TS + Vite + TanStack Router/Query + Tailwind/shadcn + Zustand** — größtes Ökosystem, beste Agent-Vertrautheit, alle Komponenten verfügbar; Re-Render-Performance muss aktiv gemanagt werden.
2. **B — SolidJS** — feingranulare Reaktivität, sehr performant; kleineres Ökosystem.
3. **C — Svelte 5** — schlank, gute DX; weniger Komponentenbibliotheken für Editor/Terminal-Integrationen.
4. **D — Rust-WASM-Frontend (Leptos/Dioxus)** — eine Sprache; unreifes Ökosystem, Monaco/xterm nur über JS-Interop, Agents weniger sicher.

## Entscheidung
Option **A**: **React 19 + TypeScript + Vite + TanStack Router/Query + Tailwind + shadcn/ui + Zustand**, ergänzt um Monaco, xterm.js, Shiki und xyflow (Sub-Agent-Graph). Läuft innerhalb von Tauri und als Web-UI/PWA aus derselben Codebasis (`apps/web`). Streaming-Performance über **Virtualisierung + Event-Batching**. Typen kommen generiert aus Rust (ADR-0019).

## Konsequenzen
- Positiv: Maximale Komponentenauswahl und Agent-Produktivität; konsistente Typen zwischen Rust und TS.
- Negativ / Risiken: Bundle-Größe (Monaco) → Code-Splitting/Lazy-Loading; Rendering-Kosten bei langen Sessions.
- Folgearbeiten: Design-Tokens/Themes (ADR-0021), Vitest + Playwright (ADR-0031), PWA-Manifest und Service Worker für Web-Push.

## Bezug
- Spec: docs/spec/08-clients.md (Prefix WEB/DESK)
