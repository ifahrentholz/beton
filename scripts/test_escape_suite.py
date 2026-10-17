"""QA-005 AC2: Die Escape-Suite kann nicht still grün werden (ADR-0042).

Die Suite ist das nextest-Profil `escape` in `.config/nextest.toml`. Jede Testdatei, die es nennt,
muss existieren, und kein Test darin trägt `#[ignore]`: Escape-Tests werden nie übersprungen
(AGENTS.md §4).

  python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import os
import re
import unittest

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))


def escape_files(config):
    """Testdateien der Suite aus dem `default-filter` des Profils `escape`."""
    m = re.search(r"\[profile\.escape\]\s*default-filter\s*=\s*'''(.*?)'''", config, re.S)
    if not m:
        raise ValueError("Profil `escape` ohne default-filter")
    ids = re.findall(r"binary_id\(([a-z0-9-]+)::([a-z0-9_]+)\)", m.group(1))
    return [os.path.join("crates", krate, "tests", f"{name}.rs") for krate, name in ids]


class EscapeSuite(unittest.TestCase):
    def setUp(self):
        with open(os.path.join(ROOT, ".config", "nextest.toml"), encoding="utf-8") as f:
            self.files = escape_files(f.read())

    def test_qa_005_ac2_suite_names_the_escape_test_files(self):
        self.assertEqual(
            sorted(self.files),
            sorted(
                [
                    "crates/beton-sandbox/tests/seatbelt.rs",
                    "crates/beton-sandbox/tests/linux.rs",
                    "crates/beton-sandbox/tests/broker.rs",
                    "crates/beton-runner/tests/stages.rs",
                    "crates/beton-cli/tests/m2_demo.rs",
                ]
            ),
        )
        for path in self.files:
            self.assertTrue(os.path.isfile(os.path.join(ROOT, path)), path)

    def test_qa_005_ac2_escape_tests_are_never_ignored(self):
        for path in self.files:
            with open(os.path.join(ROOT, path), encoding="utf-8") as f:
                text = f.read()
            self.assertNotRegex(text, r"#\[\s*ignore", f"{path}: Escape-Tests werden nie übersprungen")


if __name__ == "__main__":
    unittest.main()
