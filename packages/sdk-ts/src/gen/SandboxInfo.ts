// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
/**
 * Sandbox-Status des Hosts für Clients (`GET /v1/info`, SBX-006, HAR-027).
 */
export type SandboxInfo = { 
/**
 * Konfiguriertes Backend: `auto` (Default), `seatbelt`, `landlock`, `bwrap`, … oder
 * `none` (ausdrücklich ohne Isolation, je Session zu bestätigen).
 */
backend: string, 
/**
 * Gewähltes Backend dieses Rechners (bei `auto`); fehlt bei `none`.
 */
provider?: string, 
/**
 * Sessions starten mit Sandbox: Das Backend ist auf diesem Rechner nutzbar.
 */
active: boolean, 
/**
 * Netz der Tools nur über den Egress-Proxy (oder gar keins, `allow_network: false`).
 */
proxy: boolean, 
/**
 * `yolo` ist startbar (für Harnesses, deren Tools in Stufe 2 laufen).
 */
yolo: boolean, 
/**
 * Was dem Backend fehlt, wenn es nicht nutzbar ist (z. B. `net`).
 */
missing?: Array<string>, 
/**
 * Hinweis zur Einrichtung, wenn keine Sandbox aktiv ist.
 */
hint?: string, };
