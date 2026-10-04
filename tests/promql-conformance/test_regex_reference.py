"""Falsify lost regex cases/fields, changed profiles and corrupted source facades."""
import base64
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import regex_reference as reference
from regex_capture import verify_extras


def b(value):
    return base64.b64encode(value).decode()


def find_case():
    return {"id": "test", "mode": "find", "profile": "leftmost-first", "pattern": b(b"(?P<x>a)(?P<x>b)?"),
            "subject": b(b"a"), "lookups": [b(b"x"), b(b"missing")], "templates": [b(b"$x")],
            "expected": {"error": None, "num_subexp": 2, "names": ["", b(b"x"), b(b"x")], "indices": [1, -1],
                "match_string": True, "first": [0, 1, 0, 1, -1, -1], "matches": [[0, 1, 0, 1, -1, -1]],
                "expansions": [[b(b"a")]], "replacements": [b(b"a")]}}


class RegexReferenceTests(unittest.TestCase):
    def test_complete_capture_shape_and_nil_are_required(self):
        case = find_case()
        reference.validate_case(case)
        for field in case["expected"]:
            with self.subTest(field=field):
                changed = deepcopy(case)
                del changed["expected"][field]
                with self.assertRaises(ValueError):
                    reference.validate_case(changed)
        changed = deepcopy(case)
        changed["expected"].update(match_string=False, first=None, matches=None, expansions=[[]])
        reference.validate_case(changed)
        changed["expected"]["matches"] = []
        with self.assertRaises(ValueError):
            reference.validate_case(changed)

    def test_capture_bounds_and_dimensions_cannot_change(self):
        mutations = [("first", [0, 2, 0, 1, -1, -1]), ("first", [0, 1, -1, 0, -1, -1]),
                     ("matches", [[0, 1]]), ("names", ["", b(b"x")]), ("indices", [3, -1]),
                     ("indices", [True, -1]), ("expansions", [[]]), ("expansions", [["!"]]),
                     ("replacements", []), ("match_string", 1), ("num_subexp", 2.0)]
        for field, value in mutations:
            with self.subTest(field=field, value=value):
                case = find_case()
                case["expected"][field] = value
                with self.assertRaises(ValueError):
                    reference.validate_case(case)

    def test_original_roster_and_input_binding_are_required(self):
        bindings = reference.bindings([])["syntax"]
        cases = [{**row, "expected": {"error": {"code": b(b"e"), "expression": "", "message": b(b"e")}}}
                 for row in bindings.values()]
        data = {"schema_version": 1, "commit": reference.GRAFANA_COMMIT, "go_version": "go1.27.1",
                "unicode_version": "17.0.0", "original_assertions_passed": True,
                "syntax_origin": "github.com/grafana/regexp/syntax", "cases": cases}
        reference.validate_cases("syntax", data, [])
        for mutation in ("missing", "invented", "duplicate", "pattern", "flags", "origin", "profile", "mode", "field"):
            with self.subTest(mutation=mutation):
                changed = deepcopy(data)
                if mutation == "missing":
                    changed["cases"].pop()
                elif mutation == "duplicate":
                    changed["cases"][1] = deepcopy(changed["cases"][0])
                elif mutation == "invented":
                    changed["cases"][0]["id"] = "invented"
                elif mutation == "field":
                    del changed["cases"][0]["pattern"]
                else:
                    changed["cases"][0][mutation] = 212 if mutation == "flags" else "changed"
                with self.assertRaises(ValueError):
                    reference.validate_cases("syntax", changed, [])
        for key, value in (("original_assertions_passed", 1), ("go_version", "wrong"), ("commit", "wrong")):
            changed = deepcopy(data)
            changed[key] = value
            with self.assertRaises(ValueError):
                reference.validate_cases("syntax", changed, [])

    def test_targeted_bindings_include_templates_subjects_and_profiles(self):
        request = {"id": "test", "pattern": b(b"a"), "flags": [212], "subjects": [b(b"a")],
                   "templates": [b(b"$0")], "lookups": [b(b"x")]}
        bindings = reference.bindings([request])
        case = deepcopy(bindings["regexp"]["targeted:find:test:000"])
        self.assertEqual(case["templates"], request["templates"])
        self.assertEqual(case["subject"], request["subjects"][0])
        self.assertEqual(bindings["syntax"]["std:syntax:targeted:test:212"]["origin"], "Go 1.27.1 regexp/syntax")
        request["flags"].append(212)
        with self.assertRaises(ValueError):
            reference.bindings([request])

    def test_source_digests_bytes_hooks_and_extra_files_are_required(self):
        parent = reference.HERE / ".tmp"
        parent.mkdir(exist_ok=True)
        task = Path(tempfile.mkdtemp(prefix="regex-validator-test-", dir=parent))
        manifests, baseline, hooks = {}, {}, {}
        for name, entries in reference.HOOKS.items():
            facade = task / name
            facade.mkdir()
            raw = b"public source fixture\n"
            (facade / "source.go").write_bytes(raw)
            manifest = {"source.go": hashlib.sha256(raw).hexdigest()}
            manifests[name] = manifest
            baseline[name] = {"files": 1, "digest": hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()}
            for relative, source_name in entries.items():
                destination = facade / relative
                destination.parent.mkdir(exist_ok=True, parents=True)
                destination.write_bytes(source_name.encode())
                original = task / "capture" / source_name
                original.parent.mkdir(exist_ok=True, parents=True)
                original.write_bytes(source_name.encode())
                hooks[destination.name+":"+destination.parent.name] = hashlib.sha256(source_name.encode()).hexdigest()
        (task / "regex-original-inputs.json").write_text(json.dumps({"sources": baseline}))
        (task / "source-manifests.json").write_text(json.dumps(manifests))
        metadata = {"source_file_counts": {name: 1 for name in manifests},
                    "source_digests": {name: row["digest"] for name, row in baseline.items()}, "hook_digests": hooks}
        with patch.object(reference, "HERE", task):
            reference.validate_sources(task, metadata)
            for field in metadata:
                changed = deepcopy(metadata)
                changed[field][next(iter(changed[field]))] = "changed"
                with self.assertRaises(ValueError):
                    reference.validate_sources(task, changed)
            (task / "grafana/source.go").write_bytes(b"changed source")
            with self.assertRaises(ValueError):
                reference.validate_sources(task, metadata)
        verify_extras(task / "prometheus", manifests["prometheus"], reference.HOOKS["prometheus"])
        (task / "prometheus/unlisted.go").write_bytes(b"unlisted source")
        with self.assertRaises(ValueError):
            verify_extras(task / "prometheus", manifests["prometheus"], reference.HOOKS["prometheus"])


if __name__ == "__main__":
    unittest.main()
