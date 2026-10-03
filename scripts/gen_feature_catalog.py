#!/usr/bin/env python3
"""Erzeugt design/prototype/src/catalog/features.json aus den Feature-Einträgen der Spec.

Der Design-Prototyp (ADR-0032) ordnet jede Feature-ID einem Screen zu oder markiert sie
als „kein UI“. Diese Datei ist die Grundlage dafür; sie wird nie von Hand geändert.
Aufruf: python3 scripts/gen_feature_catalog.py
"""
import glob
import json
import os
import re

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
SPEC = os.path.join(ROOT, "docs", "spec")
OUT = os.path.join(ROOT, "design", "prototype", "src", "catalog", "features.json")


def slug(heading: str) -> str:
    s = heading.strip().lower()
    s = re.sub(r"[^\w\- ]", "", s, flags=re.UNICODE)
    return s.replace(" ", "-")


def clean(text: str) -> str:
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)  # Markdown-Links
    text = text.replace("**", "")
    return text.strip()


features = []
for path in sorted(glob.glob(os.path.join(SPEC, "[0-9][0-9]-*.md"))):
    name = os.path.basename(path)
    lines = open(path, encoding="utf-8").read().splitlines()
    chapter = lines[0].lstrip("# ").strip()
    current = None
    for line in lines:
        m = re.match(r"###\s+([A-Z]+)-(\d{3})\s+—\s+(.*)", line)
        if m:
            current = {
                "id": f"{m.group(1)}-{m.group(2)}",
                "prefix": m.group(1),
                "title": clean(m.group(3)),
                "milestone": None,
                "priority": None,
                "chapter": name,
                "chapterTitle": chapter,
                "anchor": slug(line[4:]),
                "summary": "",
                "acs": [],
            }
            features.append(current)
            continue
        if line.startswith("## ") or (line.startswith("### ") and current):
            current = None
            continue
        if not current:
            continue
        mm = re.search(r"\*\*Meilenstein:\*\*\s*(M\d|v2)\s*·\s*\*\*Priorität:\*\*\s*(\w+)", line)
        if mm and current["milestone"] is None:
            current["milestone"], current["priority"] = mm.group(1), mm.group(2)
            continue
        if line.startswith("- **Beschreibung:**") and not current["summary"]:
            current["summary"] = clean(line.split(":**", 1)[1])
            continue
        ma = re.match(r"\s*- \[[ xX]\] AC(\d+) — (.*)", line)
        if ma:
            current["acs"].append(clean(ma.group(2)))

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "w", encoding="utf-8") as fh:
    json.dump(features, fh, ensure_ascii=False, indent=1)
    fh.write("\n")
print(f"{len(features)} Features → {os.path.relpath(OUT, ROOT)}")
