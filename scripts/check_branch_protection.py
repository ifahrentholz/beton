#!/usr/bin/env python3
"""QA-010 AC3: Alle CI-Jobs aus .github/workflows/ci.yml sind Required Checks auf `main`.

Liest die Job-Namen (Matrix-Werte eingesetzt) und vergleicht sie mit den Required Status
Checks, die der öffentliche Branch-Endpunkt der GitHub-API meldet. Exit 1 bei Abweichung.

  python3 scripts/check_branch_protection.py            # gegen die API (braucht `gh`)
  python3 scripts/check_branch_protection.py --jobs     # nur die erwarteten Namen ausgeben
"""
import json, os, re, subprocess, sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
REPO = os.environ.get("GITHUB_REPOSITORY", "ifahrentholz/beton")


def job_names(path):
    """Job-Namen aus ci.yml; `${{ matrix.<k> }}` wird mit jedem Matrix-Wert ersetzt."""
    lines = open(path, encoding="utf-8").read().split("\n")
    jobs, cur, in_jobs = [], None, False
    for line in lines:
        if line.startswith("jobs:"):
            in_jobs = True
            continue
        if not in_jobs:
            continue
        if re.match(r"^  [A-Za-z0-9_-]+:\s*$", line):
            cur = {"id": line.strip()[:-1], "name": None, "matrix": {}}
            jobs.append(cur)
        elif cur and (m := re.match(r"^    name:\s*(.+?)\s*$", line)):
            cur["name"] = m.group(1).strip("'\"")
        elif cur and (m := re.match(r"^        ([A-Za-z0-9_-]+):\s*\[(.*)\]\s*$", line)):
            cur["matrix"][m.group(1)] = [v.strip().strip("'\"") for v in m.group(2).split(",")]
    names = []
    for j in jobs:
        name = j["name"] or j["id"]
        keys = re.findall(r"\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}", name)
        variants = [name]
        for k in keys:
            variants = [re.sub(r"\$\{\{\s*matrix\." + k + r"\s*\}\}", v, n) for n in variants for v in j["matrix"].get(k, [])]
        names += variants
    return sorted(names)


def main():
    expected = job_names(os.path.join(ROOT, ".github/workflows/ci.yml"))
    if "--jobs" in sys.argv:
        print("\n".join(expected))
        return 0
    out = subprocess.run(["gh", "api", f"repos/{REPO}/branches/main"], capture_output=True, text=True, check=True).stdout
    prot = json.loads(out).get("protection", {})
    required = sorted(prot.get("required_status_checks", {}).get("contexts", []))
    missing = [n for n in expected if n not in required]
    extra = [n for n in required if n not in expected]
    print(f"CI-Jobs: {len(expected)} · Required Checks: {len(required)} · geschützt: {prot.get('enabled')}")
    if not prot.get("enabled") or missing or extra:
        for n in missing:
            print(f"  fehlt als Required Check: {n}")
        for n in extra:
            print(f"  Required Check ohne CI-Job: {n}")
        print("Abhilfe: Branch-Protection von main anpassen (Settings → Branches) – Maintainer.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
