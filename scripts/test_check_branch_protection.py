"""Tests für scripts/check_branch_protection.py (QA-010 AC3).

  python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import os
import unittest

import check_branch_protection as cbp

CI = """name: CI
on: [push]
jobs:
  lint:
    name: fmt + clippy
    runs-on: ubuntu-latest
  vm:
    name: escape (linux-vm, ${{ matrix.image }})
    strategy:
      matrix:
        image: [debian-12, "ubuntu-22.04"]
  plain:
    runs-on: ubuntu-latest
"""


class BranchProtection(unittest.TestCase):
    def test_qa_010_ac3_job_names_expand_the_matrix(self):
        self.assertEqual(
            cbp.job_names_from_text(CI),
            ["escape (linux-vm, debian-12)", "escape (linux-vm, ubuntu-22.04)", "fmt + clippy", "plain"],
        )

    def test_qa_010_ac3_list_ignores_comments_and_blank_lines(self):
        text = "# Kopf\nfmt + clippy\n\n  plain  # Kommentar\n"
        self.assertEqual(cbp.parse_list(text), ["fmt + clippy", "plain"])

    def test_qa_010_ac3_list_must_match_the_ci_jobs_exactly(self):
        jobs = cbp.job_names_from_text(CI)
        self.assertEqual(cbp.check_list(jobs, list(jobs)), [])
        errors = cbp.check_list(jobs, ["fmt + clippy", "plain", "plain", "alt"])
        self.assertTrue(any("escape (linux-vm, debian-12)" in e for e in errors), errors)
        self.assertTrue(any("ohne CI-Job: alt" in e for e in errors), errors)
        self.assertTrue(any("mehrfach: plain" in e for e in errors), errors)

    def test_qa_010_ac3_on_main_every_listed_check_must_be_required(self):
        errors, notes = cbp.compare(["a", "b"], ["a"], True)
        self.assertEqual(errors, ["fehlt als Required Check: b"])
        self.assertEqual(notes, [])
        errors, _ = cbp.compare(["a"], ["a", "x"], True)
        self.assertEqual(errors, ["Required Check ohne CI-Job: x"])
        errors, _ = cbp.compare(["a"], ["a"], False)
        self.assertEqual(errors, ["main ist nicht geschützt"])

    def test_qa_010_ac3_pr_accepts_only_checks_it_introduces_or_removes(self):
        # Basis: a, x; PR: a, neu. `neu` ist zum Eintragen, `x` zum Austragen vorgesehen.
        errors, notes = cbp.compare(["a", "neu"], ["a", "x"], True, base=["a", "x"])
        self.assertEqual(errors, [])
        self.assertEqual(
            notes,
            [
                "zum Eintragen vorgesehen (neu in diesem PR): neu",
                "zum Austragen vorgesehen (entfällt mit diesem PR): x",
            ],
        )
        # Fehlt ein Check, den schon die Basis verlangt, bleibt es ein Fehler.
        errors, _ = cbp.compare(["a", "b"], ["x"], True, base=["a", "b"])
        self.assertEqual(
            errors,
            ["fehlt als Required Check: a", "fehlt als Required Check: b", "Required Check ohne CI-Job: x"],
        )

    def test_qa_010_ac3_repository_list_matches_ci_yml(self):
        root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
        jobs = cbp.job_names(os.path.join(root, cbp.CI))
        listed = cbp.parse_list(cbp.read(os.path.join(root, cbp.LIST)))
        self.assertEqual(cbp.check_list(jobs, listed), [])


if __name__ == "__main__":
    unittest.main()
