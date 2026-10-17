#!/usr/bin/env python3
"""SBX-011 AC3: Fixture-Secrets tauchen in keinem Testartefakt der CI auf.

Alle Fixture-Secrets der Tests enthalten per Konvention das Wort `FIXTURE` in Großbuchstaben
(z. B. die Fixture-Homes der Escape-Suite, `crates/beton-sandbox/tests/support/mod.rs`). Das
Skript durchsucht JUnit-Berichte (nextest-Profil `ci`, mit den Ausgaben auch bestandener Tests)
und Logs danach und nennt je Treffer Datei und Testfall bzw. Zeile, nie den Inhalt.

  python3 scripts/check_fixture_secrets.py target/nextest/ci/junit.xml [weitere Dateien …]

Exit 1 bei einem Treffer und, damit nichts still durchrutscht, bei einer fehlenden, leeren oder
unlesbaren Datei bzw. einem JUnit-Bericht ohne Testfälle.
"""
import sys
import xml.etree.ElementTree as ET

MARKER = "FIXTURE"


def scan_junit(text):
    """Treffer als `Testfall`-Namen; ein Bericht ohne Testfälle ist ein Fehler."""
    root = ET.fromstring(text)
    cases = root.findall(".//testcase")
    if not cases:
        raise ValueError("JUnit-Bericht ohne Testfälle")
    def contains(el):
        return any(MARKER in (e.text or "") or MARKER in " ".join(e.attrib.values()) for e in el.iter())

    hits = [f"{c.get('classname', '')}::{c.get('name', '')}" for c in cases if contains(c)]
    # Sonst irgendwo im Bericht (z. B. Ausgaben einer ganzen Suite).
    if not hits and MARKER in text:
        hits.append("(außerhalb eines Testfalls)")
    return hits


def scan_text(text):
    return [f"Zeile {n}" for n, line in enumerate(text.splitlines(), 1) if MARKER in line]


def scan(path):
    """Liste der Fundstellen; wirft bei fehlender, leerer oder kaputter Datei."""
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    if not text.strip():
        raise ValueError("Datei ist leer")
    if path.endswith(".xml"):
        return scan_junit(text)
    return scan_text(text)


def main(paths):
    if not paths:
        print("Aufruf: check_fixture_secrets.py <junit.xml|log> …", file=sys.stderr)
        return 2
    failed = False
    for path in paths:
        try:
            hits = scan(path)
        except (OSError, ValueError, ET.ParseError) as e:
            print(f"::error title=Fixture-Secret-Scan::{path}: {e}")
            failed = True
            continue
        for hit in hits:
            print(f"::error title=Fixture-Secret im Testartefakt::{path}: {hit}")
        failed |= bool(hits)
        if not hits:
            print(f"{path}: keine Fixture-Secrets")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
