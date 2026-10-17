#!/usr/bin/env python3
"""QA-010 AC3: Alle CI-Jobs aus .github/workflows/ci.yml sind Required Checks auf `main`.

Die Liste der Required Checks steht in `scripts/required_checks.txt` (ein Name je Zeile, `#`
leitet einen Kommentar ein); genau diese Namen trägt der Maintainer in die Branch-Protection
von `main` ein. Geprüft wird zweierlei:

1. Die Datei enthält genau die Job-Namen aus ci.yml (Matrix-Werte eingesetzt), offline.
2. Die Required Status Checks, die der öffentliche Branch-Endpunkt der GitHub-API meldet,
   stimmen mit der Datei überein. In einem PR (`GITHUB_EVENT_NAME=pull_request`) gelten Namen,
   die der PR gegenüber dem Basis-Branch neu einführt bzw. entfernt, als „zum Eintragen“ bzw.
   „zum Austragen vorgesehen“ und lassen den Check nicht scheitern: Die Branch-Protection kann
   erst nach dem Merge angepasst werden (Maintainer). Auf `main` gilt der Vergleich streng.

  python3 scripts/check_branch_protection.py            # gegen die API (braucht `gh`)
  python3 scripts/check_branch_protection.py --jobs     # nur die erwarteten Namen ausgeben

Exit 1 bei Abweichung.
"""
import json, os, re, subprocess, sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
REPO = os.environ.get("GITHUB_REPOSITORY", "ifahrentholz/beton")
CI = ".github/workflows/ci.yml"
LIST = "scripts/required_checks.txt"


def job_names_from_text(text):
    """Job-Namen aus ci.yml; `${{ matrix.<k> }}` wird mit jedem Matrix-Wert ersetzt."""
    lines = text.split("\n")
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


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def job_names(path):
    return job_names_from_text(read(path))


def parse_list(text):
    """Namen aus required_checks.txt (ohne Kommentare und Leerzeilen)."""
    names = []
    for line in text.split("\n"):
        line = line.split("#", 1)[0].strip()
        if line:
            names.append(line)
    return names


def check_list(jobs, listed):
    """Die Datei enthält genau die Jobs aus ci.yml, jeden einmal."""
    errors = []
    for n in sorted(set(jobs) - set(listed)):
        errors.append(f"CI-Job fehlt in {LIST}: {n}")
    for n in sorted(set(listed) - set(jobs)):
        errors.append(f"{LIST} nennt einen Namen ohne CI-Job: {n}")
    for n in sorted({n for n in listed if listed.count(n) > 1}):
        errors.append(f"{LIST} nennt einen Namen mehrfach: {n}")
    return errors


def compare(listed, required, enabled, base=None):
    """Vergleich mit der Branch-Protection.

    `base` sind die Required Checks laut Basis-Branch (nur in PRs): Was der PR neu einführt bzw.
    entfernt, ist vorgesehen statt falsch. Liefert (Fehler, Hinweise).
    """
    errors, notes = [], []
    if not enabled:
        errors.append("main ist nicht geschützt")
    for n in sorted(set(listed) - set(required)):
        if base is not None and n not in base:
            notes.append(f"zum Eintragen vorgesehen (neu in diesem PR): {n}")
        else:
            errors.append(f"fehlt als Required Check: {n}")
    for n in sorted(set(required) - set(listed)):
        if base is not None and n in base:
            notes.append(f"zum Austragen vorgesehen (entfällt mit diesem PR): {n}")
        else:
            errors.append(f"Required Check ohne CI-Job: {n}")
    return errors, notes


def git_show(ref, path):
    out = subprocess.run(["git", "show", f"{ref}:{path}"], cwd=ROOT, capture_output=True, text=True)
    return out.stdout if out.returncode == 0 else None


def base_checks():
    """Required Checks laut Basis-Branch des PRs: dessen required_checks.txt bzw., wo es die
    Datei noch nicht gibt, die Jobs seiner ci.yml. `None` außerhalb von PRs."""
    if os.environ.get("GITHUB_EVENT_NAME") != "pull_request":
        return None
    ref = os.environ.get("GITHUB_BASE_REF") or "main"
    subprocess.run(["git", "fetch", "-q", "--depth=1", "origin", ref], cwd=ROOT, check=True)
    text = git_show("FETCH_HEAD", LIST)
    if text is not None:
        return parse_list(text)
    ci = git_show("FETCH_HEAD", CI)
    if ci is None:
        raise SystemExit(f"Basis-Branch {ref} ohne {CI}")
    return job_names_from_text(ci)


def main():
    jobs = job_names(os.path.join(ROOT, CI))
    if "--jobs" in sys.argv:
        print("\n".join(jobs))
        return 0
    listed = parse_list(read(os.path.join(ROOT, LIST)))
    errors = check_list(jobs, listed)
    out = subprocess.run(["gh", "api", f"repos/{REPO}/branches/main"], capture_output=True, text=True, check=True).stdout
    prot = json.loads(out).get("protection", {})
    required = sorted(prot.get("required_status_checks", {}).get("contexts", []))
    more, notes = compare(listed, required, prot.get("enabled"), base_checks())
    errors += more
    print(f"CI-Jobs: {len(jobs)} · {LIST}: {len(listed)} · Required Checks: {len(required)} · geschützt: {prot.get('enabled')}")
    for n in notes:
        print(f"  {n}")
    for e in errors:
        print(f"  {e}")
    if errors:
        print(f"Abhilfe: {LIST} an ci.yml angleichen bzw. Branch-Protection von main anpassen (Settings → Branches) – Maintainer.")
        return 1
    if notes:
        print(f"Nach dem Merge: Required Checks von main laut {LIST} eintragen (Maintainer).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
