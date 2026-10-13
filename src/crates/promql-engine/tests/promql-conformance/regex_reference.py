"""Validate independent regex capture coverage, attribution and complete byte schemas."""
import base64
import hashlib
import json
from pathlib import Path

from build import directory
from parser import difference

HERE = Path(__file__).resolve().parent
GRAFANA_COMMIT = "f7b3be9d18538c56fb03caa6088db55404e784e8"
PROMETHEUS_COMMIT = "5241a27fe3c6983549fccc32f6e65917408c63cd"
HOOKS = {"grafana": {"syntax/netdata_capture_test.go": "regex_syntax_test.go",
                      "netdata_capture_test.go": "regex_test.go"},
         "prometheus": {"model/labels/netdata_capture_test.go": "regex_selector_test.go"}}


def byte_string(value):
    if type(value) is not str:
        raise ValueError("regex byte string has incorrect type")
    try:
        raw = base64.b64decode(value, validate=True)
    except (ValueError, TypeError) as error:
        raise ValueError("invalid regex base64") from error
    if base64.b64encode(raw).decode() != value:
        raise ValueError("noncanonical regex base64")
    return raw


def integer(value, minimum=0):
    if type(value) is not int or value < minimum:
        raise ValueError("incorrect regex integer")


def boolean(value):
    if type(value) is not bool:
        raise ValueError("incorrect regex boolean")


def keys(value, expected):
    if type(value) is not dict or value.keys() != set(expected):
        raise ValueError("missing or extra regex fields")


def byte_list(values):
    if type(values) is not list:
        raise ValueError("incorrect regex byte-list type")
    return [byte_string(value) for value in values]


def bindings(requests):
    baseline = json.loads((HERE / "regex-original-inputs.json").read_bytes())
    if (baseline["schema_version"] != 1 or baseline["commits"] != {
            "grafana": GRAFANA_COMMIT, "prometheus": PROMETHEUS_COMMIT}
            or len(baseline["syntax"]) != 610 or len(baseline["regexp"]) != 226
            or len(baseline["selectors"]["patterns"]) != 72 or len(baseline["selectors"]["subjects"]) != 95):
        raise ValueError("original regex input inventory changed")
    result = {"syntax": {row["id"]: row for row in baseline["syntax"]},
              "regexp": {row["id"]: row for row in baseline["regexp"]}, "selectors": {}}
    for i, pattern in enumerate(baseline["selectors"]["patterns"]):
        for j, subject in enumerate(baseline["selectors"]["subjects"]):
            identity = f"selector:{i:03d}:{j:03d}"
            result["selectors"][identity] = {"id": identity, "mode": "selector", "pattern": pattern,
                "subject": subject, "profile": "Prometheus FastRegexMatcher"}
    seen_ids, seen_subjects = set(), set()
    for request in requests:
        keys(request, ("id", "pattern", "flags", "subjects", "templates", "lookups"))
        if type(request["id"]) is not str or not request["id"] or request["id"] in seen_ids:
            raise ValueError("duplicate or incorrect targeted regex identity")
        seen_ids.add(request["id"])
        byte_string(request["pattern"])
        for field in ("subjects", "templates", "lookups"):
            byte_list(request[field])
        if type(request["flags"]) is not list or len(set(request["flags"])) != len(request["flags"]):
            raise ValueError("invalid regex syntax profiles")
        for flags in request["flags"]:
            integer(flags)
            if flags > 65535:
                raise ValueError("invalid regex syntax flags")
            for prefix, mode, origin in (("", "syntax", "github.com/grafana/regexp/syntax"),
                    ("std:", "standard-syntax", "Go 1.27.1 regexp/syntax")):
                identity = f'{prefix}syntax:targeted:{request["id"]}:{flags:03d}'
                result["syntax"][identity] = {"id": identity, "mode": mode, "profile": "targeted",
                    "origin": origin, "pattern": request["pattern"], "flags": flags}
        for mode in ("find", "label_replace"):
            identity = f'program:targeted:{mode}:{request["id"]}'
            pattern = byte_string(request["pattern"])
            if mode == "label_replace":
                pattern = b"^(?s:"+pattern+b")$"
            result["regexp"][identity] = {"id": identity, "mode": "program", "profile": "Go compileOnePass",
                "pattern": base64.b64encode(pattern).decode()}
        for index, subject in enumerate(request["subjects"]):
            for mode in ("find", "label_replace"):
                identity = f'targeted:{mode}:{request["id"]}:{index:03d}'
                result["regexp"][identity] = {"id": identity, "mode": mode, "profile": "leftmost-first",
                    "pattern": request["pattern"], "subject": subject, "templates": request["templates"],
                    "lookups": request["lookups"]}
            identity = f'targeted:selector:{request["id"]}:{index:03d}'
            result["selectors"][identity] = {"id": identity, "mode": "selector",
                "profile": "targeted Prometheus FastRegexMatcher", "pattern": request["pattern"], "subject": subject}
            if subject not in seen_subjects:
                identity = f"normalization:{len(seen_subjects):04d}"
                seen_subjects.add(subject)
                result["selectors"][identity] = {"id": identity, "mode": "normalize", "profile": "Go NFKD and ToLower",
                    "pattern": "", "subject": subject}
    return result


def validate_case(case):
    mode, expected = case["mode"], case["expected"]
    byte_string(case["pattern"])
    if mode in ("syntax", "standard-syntax"):
        if expected.get("error") is not None:
            keys(expected, ("error",))
            keys(expected["error"], ("code", "expression", "message"))
            for value in expected["error"].values():
                byte_string(value)
        else:
            keys(expected, ("error", "canonical", "names"))
            byte_string(expected["canonical"])
            names = byte_list(expected["names"])
            if not names or names[0] != b"":
                raise ValueError("invalid syntax capture names")
        return
    if mode == "normalize":
        keys(expected, ("error", "normalized", "lower"))
        if expected["error"] is not None:
            raise ValueError("normalization cannot fail")
        for field in ("normalized", "lower"):
            byte_string(expected[field])
        byte_string(case["subject"])
        return
    if expected.get("error") is not None:
        keys(expected, ("error",))
        byte_string(expected["error"])
        return
    if mode == "selector":
        keys(expected, ("error", "matches"))
        boolean(expected["matches"])
        byte_string(case["subject"])
        return
    if mode == "program":
        keys(expected, ("error", "start", "num_cap", "one_pass", "prefix", "prefix_end", "prefix_complete", "instructions"))
        for field in ("start", "num_cap", "prefix_end"):
            integer(expected[field])
        for field in ("one_pass", "prefix_complete"):
            boolean(expected[field])
        byte_string(expected["prefix"])
        if type(expected["instructions"]) is not list or not expected["instructions"]:
            raise ValueError("missing Go instructions")
        if max(expected["start"], expected["prefix_end"]) >= len(expected["instructions"]) or expected["num_cap"] < 2:
            raise ValueError("invalid Go program indices")
        for instruction in expected["instructions"]:
            keys(instruction, ("op", "out", "arg", "runes"))
            if instruction["op"] not in ("InstFail", "InstNop", "InstMatch", "InstCapture", "InstEmptyWidth", "InstAlt",
                    "InstRune", "InstRune1", "InstRuneAny", "InstRuneAnyNotNL"):
                raise ValueError("invalid Go instruction")
            integer(instruction["out"])
            integer(instruction["arg"])
            if instruction["runes"] is not None:
                if type(instruction["runes"]) is not list:
                    raise ValueError("invalid Go runes")
                for rune in instruction["runes"]:
                    integer(rune)
                    if rune > 0x10ffff:
                        raise ValueError("invalid Go rune value")
        return
    required = {"error", "num_subexp", "names", "indices"}
    if mode in ("find", "label_replace"):
        required.update(("match_string", "first", "matches", "expansions", "replacements"))
    elif mode != "compile":
        raise ValueError("unknown regex reference mode")
    keys(expected, required)
    integer(expected["num_subexp"])
    names = byte_list(expected["names"])
    lookups, templates = byte_list(case["lookups"]), byte_list(case["templates"])
    if len(names) != expected["num_subexp"]+1 or names[0] != b"" or type(expected["indices"]) is not list:
        raise ValueError("incorrect capture metadata")
    if len(expected["indices"]) != len(lookups):
        raise ValueError("incorrect capture-name lookup count")
    for index in expected["indices"]:
        integer(index, -1)
        if index >= len(names):
            raise ValueError("invalid capture-name index")
    if mode == "compile":
        return
    subject = byte_string(case["subject"])
    boolean(expected["match_string"])
    def offsets(value):
        if type(value) is not list or len(value) != 2*len(names):
            raise ValueError("incorrect capture width")
        for start, end in zip(value[::2], value[1::2]):
            integer(start, -1)
            integer(end, -1)
            if (start, end) != (-1, -1) and not 0 <= start <= end <= len(subject):
                raise ValueError("invalid capture offsets")
        if value[0] < 0:
            raise ValueError("successful capture has no whole-match offset")
    matches = expected["matches"]
    if expected["match_string"]:
        offsets(expected["first"])
        if type(matches) is not list or not matches or difference(matches[0], expected["first"]):
            raise ValueError("first/all capture disagreement")
        for match in matches:
            offsets(match)
    elif matches is not None or expected["first"] is not None:
        raise ValueError("unmatched capture must be nil")
    if type(expected["expansions"]) is not list or len(expected["expansions"]) != len(templates):
        raise ValueError("incorrect expansion dimension")
    for expansions in expected["expansions"]:
        if len(byte_list(expansions)) != len(matches or []):
            raise ValueError("incorrect expansion match dimension")
    if len(byte_list(expected["replacements"])) != len(templates):
        raise ValueError("incorrect replacement dimension")


def validate_cases(name, data, requests):
    expected = bindings(requests)[name]
    commit = PROMETHEUS_COMMIT if name == "selectors" else GRAFANA_COMMIT
    header = {"schema_version": 1, "commit": commit, "go_version": "go1.27.1", "unicode_version": "17.0.0",
              "original_assertions_passed": True}
    if name == "syntax":
        header["syntax_origin"] = "github.com/grafana/regexp/syntax"
    elif name == "regexp":
        header["syntax_origin"] = "Go 1.27.1 regexp/syntax"
    else:
        header.update(pattern_count=72, subject_count=95)
    keys(data, (*header, "cases"))
    if difference(header, {key: data[key] for key in header}):
        raise ValueError("regex source/profile attribution changed")
    if type(data["cases"]) is not list or len(data["cases"]) != len(expected):
        raise ValueError("regex reference coverage changed")
    seen = set()
    for case in data["cases"]:
        if type(case) is not dict or case.get("id") not in expected or case["id"] in seen:
            raise ValueError("missing, duplicate or invented regex identity")
        seen.add(case["id"])
        binding = expected[case["id"]]
        keys(case, (*binding, "expected"))
        if difference(binding, {key: case[key] for key in binding}):
            raise ValueError("regex input/profile binding changed")
        validate_case(case)
    if seen != expected.keys():
        raise ValueError("missing required regex identity")
    return data["cases"]


def validate_sources(reference, metadata):
    baseline = json.loads((HERE / "regex-original-inputs.json").read_bytes())["sources"]
    manifests = json.loads((reference / "source-manifests.json").read_bytes())
    keys(manifests, baseline)
    keys(metadata["source_file_counts"], baseline)
    keys(metadata["source_digests"], baseline)
    expected_hooks = {}
    for source, expected in baseline.items():
        manifest = manifests[source]
        digest = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
        if (len(manifest) != expected["files"] or digest != expected["digest"]
                or metadata["source_file_counts"][source] != expected["files"]
                or metadata["source_digests"][source] != digest):
            raise ValueError("regex reference source identity changed")
        facade = directory(reference / source)
        files = set()
        for path in facade.rglob("*"):
            if path.is_symlink():
                raise ValueError("regex facade contains a symlink")
            if path.is_file():
                files.add(path.relative_to(facade).as_posix())
        if files != manifest.keys() | HOOKS[source].keys():
            raise ValueError("unlisted or missing regex facade file")
        for relative, digest in manifest.items():
            if hashlib.sha256((facade / relative).read_bytes()).hexdigest() != digest:
                raise ValueError("regex facade source bytes changed")
        for relative, hook in HOOKS[source].items():
            raw = (HERE / "capture" / hook).read_bytes()
            if (facade / relative).read_bytes() != raw:
                raise ValueError("regex hook bytes changed")
            key = Path(relative).name+":"+(facade / relative).parent.name
            expected_hooks[key] = hashlib.sha256(raw).hexdigest()
    if difference(expected_hooks, metadata["hook_digests"]):
        raise ValueError("regex hook digest changed")


def load(reference):
    reference = directory(reference)
    metadata = json.loads((reference / "metadata.json").read_bytes())
    keys(metadata, ("schema_version", "source_file_counts", "source_digests", "hook_digests", "requests_sha256", "outputs"))
    if type(metadata["schema_version"]) is not int or metadata["schema_version"] != 1:
        raise ValueError("invalid regex metadata version")
    raw_requests = (HERE / "regex-requests.json").read_bytes()
    if metadata["requests_sha256"] != hashlib.sha256(raw_requests).hexdigest():
        raise ValueError("targeted regex requests changed")
    requests = json.loads(raw_requests)
    validate_sources(reference, metadata)
    expected = bindings(requests)
    keys(metadata["outputs"], expected)
    cases = []
    for name in expected:
        raw = (reference / f"{name}.json").read_bytes()
        if difference(metadata["outputs"][name], {"sha256": hashlib.sha256(raw).hexdigest(), "cases": len(expected[name])}):
            raise ValueError("regex capture output digest or count changed")
        cases.extend(validate_cases(name, json.loads(raw), requests))
    if len({case["id"] for case in cases}) != len(cases):
        raise ValueError("regex identity overlaps source families")
    return cases
