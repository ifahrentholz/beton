"""Tests für scripts/escape_vm.py (SBX-011, QA-005 AC3): Auswertung der VM-Ergebnisse.

  python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import json
import os
import tempfile
import unittest

import escape_vm


def write_out(root, files):
    out = os.path.join(root, "out")
    os.makedirs(out, exist_ok=True)
    for name, text in files.items():
        with open(os.path.join(out, name), "w", encoding="utf-8") as f:
            f.write(text)


def probe(abi, available):
    return json.dumps(
        {
            "providers": [
                {"id": i, "available": available, "landlock_abi": abi, "kernel": "k",
                 **({} if available else {"unavailable_reason": "ABI 1"})}
                for i in ("landlock", "bwrap")
            ]
        }
    )


class EscapeVm(unittest.TestCase):
    def check(self, image, files):
        with tempfile.TemporaryDirectory() as d:
            write_out(d, files)
            return escape_vm.check(type("A", (), {"image": image, "out": d})())

    def test_sbx_011_every_image_is_pinned_by_checksum(self):
        for name, spec in escape_vm.IMAGES.items():
            self.assertTrue(spec["url"].startswith("https://"), name)
            self.assertTrue("sha256" in spec or "sha512" in spec, name)
            self.assertTrue("abi" in spec or "min_abi" in spec, name)

    def test_qa_005_ac3_debian_passes_with_abi_2_and_green_suite(self):
        ok = {"done": "fertig", "probe.json": probe(2, True), "sandbox-test.status": "0",
              "suite.status": "0", "junit.xml": "<testsuites/>"}
        self.assertEqual(self.check("debian-12", ok), 0)

    def test_qa_005_ac3_wrong_abi_or_red_suite_fails(self):
        base = {"done": "fertig", "probe.json": probe(2, True), "sandbox-test.status": "0",
                "suite.status": "0", "junit.xml": "<testsuites/>"}
        self.assertEqual(self.check("debian-12", {**base, "probe.json": probe(3, True)}), 1)
        self.assertEqual(self.check("debian-12", {**base, "suite.status": "100"}), 1)
        self.assertEqual(self.check("debian-12", {k: v for k, v in base.items() if k != "junit.xml"}), 1)
        self.assertEqual(self.check("debian-12", {k: v for k, v in base.items() if k != "done"}), 1)

    def test_qa_005_ac3_ubuntu_22_04_must_report_unavailable_and_fail_sandbox_test(self):
        ok = {"done": "fertig", "probe.json": probe(1, False), "sandbox-test.status": "1"}
        self.assertEqual(self.check("ubuntu-22.04", ok), 0)
        self.assertEqual(self.check("ubuntu-22.04", {**ok, "sandbox-test.status": "0"}), 1)
        self.assertEqual(self.check("ubuntu-22.04", {**ok, "probe.json": probe(1, True)}), 1)

    def test_sbx_011_provisioning_is_valid_cloud_config(self):
        text = escape_vm.user_data(1001)
        self.assertTrue(text.startswith("#cloud-config\n"))
        config = json.loads(text.split("\n", 1)[1])
        runner = [u for u in config["users"] if isinstance(u, dict)][0]
        self.assertEqual((runner["name"], runner["uid"]), ("runner", 1001))
        self.assertIn("bubblewrap", config["packages"])


if __name__ == "__main__":
    unittest.main()
