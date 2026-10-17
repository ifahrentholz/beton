"""Tests für scripts/check_landlock_abi.py (QA-005 AC3).

  python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import json
import os
import tempfile
import unittest

import check_landlock_abi as cla


def probe(abi, landlock=True, bwrap=True):
    def p(name, available):
        out = {"id": name, "available": available, "landlock_abi": abi, "kernel": "6.1.0-40-cloud-amd64"}
        if not available:
            out["unavailable_reason"] = "Landlock-ABI 1: Kernel ≥ 5.19 nötig"
        return out

    return {"auto": "landlock", "providers": [p("bwrap", bwrap), p("landlock", landlock), {"id": "none", "available": True}]}


class LandlockAbi(unittest.TestCase):
    def test_qa_005_ac3_expected_abi_passes(self):
        self.assertEqual(cla.check(probe(2), abi=2), [])
        self.assertEqual(cla.check(probe(7), min_abi=4), [])
        self.assertEqual(cla.check(probe(4), min_abi=4), [])

    def test_qa_005_ac3_other_abi_fails_instead_of_skipping(self):
        self.assertEqual(len(cla.check(probe(3), abi=2)), 1)
        self.assertEqual(len(cla.check(probe(3), min_abi=4)), 1)
        self.assertEqual(len(cla.check(probe(None), min_abi=4)), 1)
        self.assertEqual(len(cla.check({"providers": []}, abi=2)), 1)

    def test_qa_005_ac3_landlock_must_be_available_unless_expected_otherwise(self):
        errors = cla.check(probe(2, landlock=False), abi=2)
        self.assertEqual(len(errors), 1)
        self.assertIn("nicht verfügbar", errors[0])

    def test_qa_005_ac3_old_kernel_must_report_backends_unavailable(self):
        old = probe(1, landlock=False, bwrap=False)
        self.assertEqual(cla.check(old, abi=1, unavailable=["landlock", "bwrap"]), [])
        errors = cla.check(probe(1, landlock=False, bwrap=True), abi=1, unavailable=["landlock", "bwrap"])
        self.assertEqual(errors, ["`bwrap` meldet sich als verfügbar, erwartet: nicht verfügbar"])

    def test_qa_005_ac3_cli_exit_codes(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "probe.json")
            with open(path, "w", encoding="utf-8") as f:
                json.dump(probe(2), f)
            self.assertEqual(cla.main([path, "--abi", "2"]), 0)
            self.assertEqual(cla.main([path, "--min-abi", "4"]), 1)
            self.assertEqual(cla.main([os.path.join(d, "fehlt.json"), "--abi", "2"]), 1)


if __name__ == "__main__":
    unittest.main()
