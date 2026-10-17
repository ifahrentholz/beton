"""Tests für scripts/check_fixture_secrets.py (SBX-011 AC3).

  python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import os
import tempfile
import unittest

import check_fixture_secrets as cfs

JUNIT = """<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="2" failures="0" errors="0">
  <testsuite name="beton-sandbox::linux" tests="2">
    <testcase name="sbx_004_ac1_home_grant_masks_ssh_key_in_both_stages" classname="beton-sandbox::linux">
      <system-out>{out1}</system-out>
    </testcase>
    <testcase name="sbx_008_ac3_namespaces_mount_ptrace_and_kill_fail" classname="beton-sandbox::linux">
      <system-out>{out2}</system-out>
    </testcase>
  </testsuite>
</testsuites>
"""


class FixtureSecretScan(unittest.TestCase):
    def write(self, name, text):
        path = os.path.join(self.dir.name, name)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        return path

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.dir.cleanup()

    def test_sbx_011_ac3_clean_report_passes(self):
        path = self.write("junit.xml", JUNIT.format(out1="ok", out2="running 1 test"))
        self.assertEqual(cfs.scan(path), [])
        self.assertEqual(cfs.main([path]), 0)

    def test_sbx_011_ac3_fixture_in_output_names_the_test_case(self):
        path = self.write("junit.xml", JUNIT.format(out1="cat: FIXTURE-SSH-PRIVATE-KEY", out2="ok"))
        self.assertEqual(
            cfs.scan(path),
            ["beton-sandbox::linux::sbx_004_ac1_home_grant_masks_ssh_key_in_both_stages"],
        )
        self.assertEqual(cfs.main([path]), 1)

    def test_sbx_011_ac3_lowercase_fixture_in_test_names_is_no_hit(self):
        report = JUNIT.format(out1="ok", out2="ok").replace(
            "sbx_008_ac3_namespaces", "sbx_004_fixture_home_namespaces"
        )
        self.assertEqual(cfs.scan(self.write("junit.xml", report)), [])

    def test_sbx_011_ac3_logs_are_scanned_by_line(self):
        path = self.write("suite.log", "PASS a\nPASS b\nleak ghp_FIXTUREvalue\n")
        self.assertEqual(cfs.scan(path), ["Zeile 3"])
        self.assertEqual(cfs.main([path]), 1)

    def test_sbx_011_ac3_missing_empty_or_broken_artifacts_fail_closed(self):
        missing = os.path.join(self.dir.name, "fehlt.xml")
        empty = self.write("leer.xml", "")
        broken = self.write("kaputt.xml", "<testsuites>")
        no_cases = self.write("ohne.xml", "<testsuites></testsuites>")
        for path in (missing, empty, broken, no_cases):
            self.assertEqual(cfs.main([path]), 1, path)
        self.assertEqual(cfs.main([]), 2)


if __name__ == "__main__":
    unittest.main()
