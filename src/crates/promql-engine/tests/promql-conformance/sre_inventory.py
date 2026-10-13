#!/usr/bin/env python3
"""Build a private expression manifest from immutable infrastructure source objects."""
import argparse
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

import yaml

HERE = Path(__file__).resolve().parent
INFRA_COMMIT = "05c9aa6d5bf38e2c5064df83845b0cc16e7cb483"
ALERTS = "aws/common/eks-resources/monitoring/prometheus/alerts.hcl"
SLOS = "aws/common/eks-resources/monitoring/slos/output/thanos/ruler.yaml"
ANALYTICS = "aws/org/us-east-1/analytics-ml/kube-prometheus-stack/terragrunt.hcl"
ORG = "aws/org/org.hcl"
ENVIRONMENTS = ("production", "staging", "testing")
MODULE_COMMIT = "2b7489e60939561d9740573c6eefdeb63a479cb0"


@dataclass
class Token:
    kind: str
    value: str
    start: int
    line: int
    body_line: int = 0


def scan(source):
    """A bounded lexical extractor; it does not execute Terraform or interpret arbitrary HCL."""
    tokens, position = [], 0
    while position < len(source):
        start = position
        if source[position].isspace():
            position += 1
            continue
        if source.startswith("//", position) or source[position] == "#":
            end = source.find("\n", position)
            position = len(source) if end < 0 else end
            continue
        if source.startswith("/*", position):
            end = source.find("*/", position + 2)
            if end < 0:
                raise ValueError("unterminated HCL comment")
            position = end + 2
            continue
        line = source.count("\n", 0, start) + 1
        heredoc = re.match(r"<<(-?)([A-Za-z_][A-Za-z_0-9]*)[^\S\n]*\n", source[position:])
        if heredoc:
            body_start = position + heredoc.end()
            cursor = body_start
            while cursor < len(source):
                end = source.find("\n", cursor)
                end = len(source) if end < 0 else end
                delimiter = source[cursor:end]
                if delimiter.strip() == heredoc[2]:
                    body = source[body_start:cursor]
                    if heredoc[1]:
                        indents = [len(s) - len(s.lstrip(" \t")) for s in body.splitlines() if s.strip()]
                        indents.append(len(delimiter) - len(delimiter.lstrip(" \t")))
                        width = min(indents)
                        body = "".join(s[width:] if s.strip() else s for s in body.splitlines(keepends=True))
                    tokens.append(Token("heredoc", body, start, line, line + 1))
                    position = min(len(source), end + 1)
                    break
                cursor = end + 1
            else:
                raise ValueError("unterminated HCL heredoc")
            continue
        if source[position] == '"':
            position += 1
            while position < len(source):
                if source[position] == "\\":
                    position += 2
                elif source[position] == '"':
                    position += 1
                    tokens.append(Token("string", json.loads(source[start:position]), start, line))
                    break
                else:
                    position += 1
            else:
                raise ValueError("unterminated HCL string")
            continue
        match = re.match(r"[A-Za-z_][A-Za-z_0-9]*|[0-9]+(?:\.[0-9]+)?", source[position:])
        if match:
            value = match[0]
            kind = "number" if value[0].isdigit() else "name"
            tokens.append(Token(kind, value, start, line))
            position += len(value)
        else:
            tokens.append(Token("punctuation", source[position], start, line))
            position += 1
    return tokens


def pairs(tokens):
    stack, closed = [], {}
    for i, token in enumerate(tokens):
        if token.kind != "punctuation":
            continue
        if token.value in "[{(":
            stack.append(i)
        elif token.value in "]})":
            if not stack or "[{(".index(tokens[stack[-1]].value) != "]})".index(token.value):
                raise ValueError("unbalanced HCL delimiters")
            opening = stack.pop()
            closed[opening] = i
    if stack:
        raise ValueError("unclosed HCL delimiter")
    return closed


def assignment(tokens, name):
    matches = [i + 2 for i in range(len(tokens) - 2)
               if tokens[i].kind == "name" and tokens[i].value == name and tokens[i + 1].value == "="
               and tokens[i + 2].value != "="]
    if len(matches) != 1:
        raise ValueError(f"expected one assignment for {name}")
    return matches[0]


def literal_list(tokens, start, matching):
    if tokens[start].value != "[":
        raise ValueError("expected literal list")
    values = tokens[start + 1:matching[start]]
    if any(t.kind != "string" and t.value != "," for t in values):
        raise ValueError("nonliteral list requires an explicit renderer update")
    return [t.value for t in values if t.kind == "string"]


def render(template, bindings):
    def substitute(match):
        key = match[1]
        if key not in bindings:
            raise ValueError(f"unresolved HCL expression binding {key}")
        return str(bindings[key])
    result = re.sub(r"\$\{([^}]*)\}", substitute, template)
    if "${" in result or "%{" in result:
        raise ValueError("unresolved HCL template directive")
    return result


class Source:
    def __init__(self, root, commit=INFRA_COMMIT):
        self.root = Path(root).expanduser().resolve(strict=True)
        if not self.root.is_dir() or len(self.root.parts) < 4 or not re.fullmatch(r"[0-9a-f]{40}", commit):
            raise ValueError("invalid source root or revision")
        self.commit, self.files = commit, {}

    def read(self, path):
        if path.startswith("/") or ".." in Path(path).parts:
            raise ValueError("invalid repository-relative source path")
        raw = subprocess.check_output(["git", "--no-optional-locks", "show", f"{self.commit}:{path}"], cwd=self.root)
        self.files[path] = hashlib.sha256(raw).hexdigest()
        return raw.decode("utf-8")


def comprehension_bindings(header, environment, microservices, urls):
    values = [t.value for t in header]
    split = values.index("in")
    variables = [t.value for t in header[1:split] if t.kind == "name"]
    rhs = header[split + 1:]
    rhs_values = [t.value for t in rhs]
    matching = pairs(rhs)
    if rhs_values == ["local", ".", "http_checks_urls"]:
        return [{f"{variables[0]}.url": url} for url in urls]
    if rhs_values[0] == "[":
        return [{variables[0]: value} for value in literal_list(rhs, 0, matching)]
    if rhs_values[0] == "concat":
        list_end = matching[2]
        if rhs_values[:3] != ["concat", "(", "["] or rhs_values[list_end + 1:] != [",", "local", ".", "microservices", ")"]:
            raise ValueError("unexpected concat comprehension")
        fixed = literal_list(rhs, 2, matching)
        return [{variables[0]: value} for value in fixed + microservices]
    if rhs_values[0] == "{" and len(variables) == 2:
        fields, i = {}, 1
        while i < len(rhs) - 1:
            if rhs[i].value == ",":
                i += 1
                continue
            if rhs[i].kind not in {"name", "string"} or rhs[i + 1].value != "=" or rhs[i + 2].kind != "number":
                raise ValueError("unexpected map comprehension")
            fields[rhs[i].value] = rhs[i + 2].value
            i += 3
        return [{variables[0]: key, variables[1]: fields[key]} for key in sorted(fields)]
    raise ValueError(f"unsupported comprehension in {environment}")


def common_expressions(source):
    text = source.read(ALERTS)
    tokens = scan(text)
    matching = pairs(tokens)
    production_start = tokens[assignment(tokens, "production_prometheus_alerting_rules")].start
    namespace = tokens[assignment(tokens, "pulsar_namespace")].value
    comprehensions = []
    for i, token in enumerate(tokens):
        if token.value != "for" or token.kind != "name" or tokens[i + 1].kind != "name":
            continue
        k = i + 2
        if tokens[k].value == ",":
            k += 2
        if tokens[k].value != "in":
            continue
        array = max(opening for opening, end in matching.items() if opening < i < end and tokens[opening].value == "[")
        end = matching[array]
        j = k + 1
        while tokens[j].value != ":":
            j = matching[j] + 1 if j in matching else j + 1
        comprehensions.append((i, j, end))
    templates = []
    for i, token in enumerate(tokens[:-2]):
        if token.kind != "name" or token.value != "expr" or tokens[i + 1].value != "=":
            continue
        value = tokens[i + 2]
        if value.kind not in {"string", "heredoc"}:
            raise ValueError("nonliteral expression requires an explicit renderer update")
        contexts = [(start, colon, end) for start, colon, end in comprehensions if colon < i < end]
        if len(contexts) > 1:
            raise ValueError("nested comprehension needs an explicit renderer update")
        templates.append({"id": f"hcl:{token.line}", "source": ALERTS, "line": token.line,
            "query": value.value, "parameters": sorted(set(re.findall(r"\$\{([^}]*)\}", value.value))),
            "production_only": token.start > production_start,
            "header": tokens[contexts[0][0]:contexts[0][1]] if contexts else None})
    if len(templates) != 107 or len(comprehensions) != 6:
        raise ValueError("pinned HCL source closure changed")
    org = scan(source.read(ORG))
    microservices = literal_list(org, assignment(org, "microservices"), pairs(org))
    requests, counts = [], {}
    for environment in ENVIRONMENTS:
        path = f"aws/org/us-east-1/{environment}/env.hcl"
        env_tokens = scan(source.read(path))
        env_pairs = pairs(env_tokens)
        env_name = env_tokens[assignment(env_tokens, "environment")].value
        if env_name != environment:
            raise ValueError("environment reference mismatch")
        env_bindings = {"local.environment_vars.locals.environment": env_name}
        for key in ["public_dns_name", "public_mqtt_dns_name"]:
            env_bindings[f"local.{key}"] = env_tokens[assignment(env_tokens, key)].value
        url_start = assignment(env_tokens, "http_checks_urls")
        url_tokens = env_tokens[url_start:env_pairs[url_start] + 1]
        urls = [render(url_tokens[i + 2].value, env_bindings) for i in range(len(url_tokens) - 2)
                if url_tokens[i].value == "url" and url_tokens[i + 1].value == "=" and url_tokens[i + 2].kind == "string"]
        base = {"local.pulsar_namespace": render(namespace, env_bindings)}
        for key in ["max_consumers", "max_producers"]:
            value = env_tokens[assignment(env_tokens, key)]
            if value.kind != "number":
                raise ValueError("nonliteral numeric query binding")
            base[f"local.pulsar_cluster_limits.{key}"] = value.value
        before = len(requests)
        for template in templates:
            if template["production_only"] and environment != "production":
                continue
            bindings = comprehension_bindings(template["header"], environment, microservices, urls) if template["header"] else [{}]
            for index, binding in enumerate(bindings):
                for value in binding.values():
                    if any(char in str(value) for char in "\"'\\\r\n"):
                        raise ValueError("query binding needs an explicit escaping review")
                query = render(template["query"], base | binding)
                requests.append({"id": f"{template['id']}:{environment}:{index}", "query": query,
                    "profile": "server-default", "mode": "instant", "time_ms": 25200000,
                    "source_slot": template["id"], "environment": environment, "bindings": base | binding})
        counts[environment] = len(requests) - before
    if counts != {"production": 147, "staging": 140, "testing": 140}:
        raise ValueError("pinned expression expansion count changed")
    for template in templates:
        template.pop("header")
    return templates, requests, counts


def yaml_expressions(text, path, line_offset=0):
    document = yaml.compose(text)
    def fields(node):
        return {key.value: value for key, value in node.value}
    root = fields(document)
    if "spec" in root:
        root = fields(root["spec"])
    out = []
    for group in root["groups"].value:
        group_fields = fields(group)
        for index, rule in enumerate(group_fields["rules"].value):
            values = fields(rule)
            expression = values["expr"]
            kind = "recording" if "record" in values else "alert"
            line = expression.start_mark.line + line_offset + 1
            out.append({"id": f"yaml:{path}:{line}", "source": path, "line": line,
                        "query": expression.value, "kind": kind,
                        "rule_name": values["record" if kind == "recording" else "alert"].value,
                        "group": group_fields["name"].value, "group_index": index,
                        "labels": {key.value: value.value for key, value in values.get("labels", yaml.MappingNode("tag:yaml.org,2002:map", [])).value}})
    return out


def inventory(root, module_root):
    source = Source(root)
    templates, rendered, counts = common_expressions(source)
    slo_text = source.read(SLOS)
    slos = yaml_expressions(slo_text, SLOS)
    if len(slos) != 680 or sum(e["kind"] == "recording" for e in slos) != 608:
        raise ValueError("pinned SLO closure changed")
    explicit = []
    for token in scan(source.read(ANALYTICS)):
        if token.kind == "heredoc" and re.search(r"^kind:\s*PrometheusRule\s*$", token.value, re.M):
            explicit.extend(yaml_expressions(token.value, ANALYTICS, token.body_line - 1))
    if len(explicit) != 3:
        raise ValueError("pinned explicit rule closure changed")
    slots = templates + slos + explicit
    for expression in slos + explicit:
        rendered.append({"id": expression["id"], "query": expression["query"], "profile": "server-default",
                         "mode": "instant", "time_ms": 25200000, "source_slot": expression["id"]})
    covered = {expression["source_slot"] for expression in rendered}
    if covered != {slot["id"] for slot in slots} or len({e["id"] for e in rendered}) != len(rendered):
        raise ValueError("source slots are missing or rendered identities collide")
    for path in ["aws/common/eks-resources/monitoring.hcl", "aws/common/eks-resources/monitoring/slos/justfile"]:
        source.read(path)
    module = Source(module_root, MODULE_COMMIT)
    for path in ["monitoring/locals.tf", "monitoring/templates/prometheus.tpl", "monitoring/templates/thanos.tpl",
                 "kube-prometheus-stack/main.tf", "kube-prometheus-stack/templates/values.tpl"]:
        body = module.read(path)
        if path.endswith("values.tpl") and not re.search(r"^defaultRules:\s*\n\s+create:\s*false\b", body):
            raise ValueError("additional default rules require source closure")
    return {"schema_version": 1, "source": {"repository": "netdata/infra", "commit": source.commit, "files": source.files},
        "loader_source": {"repository": "netdata/infra-terraform-module-eks-resources", "commit": module.commit, "files": module.files},
        "expression_slots": slots, "rendered_expressions": rendered,
        "summary": {"slots": len(slots), "hcl_templates": len(templates), "slo_rules": len(slos),
            "slo_recording_rules": 608, "slo_alerts": 72, "explicit_rules": len(explicit),
            "hcl_expansions": counts, "rendered_requests": len(rendered),
            "distinct_slo_texts": len({e['query'] for e in slos})}}


def write_inventory(root, module_root):
    data = inventory(root, module_root)
    scratch = HERE / ".tmp"
    scratch.mkdir(exist_ok=True)
    scratch = scratch.resolve(strict=True)
    if not scratch.is_relative_to(HERE) or len(scratch.parts) < 4:
        raise ValueError("unsafe scratch directory")
    out = Path(tempfile.mkdtemp(prefix="sre-", dir=scratch))
    (out / "manifest.json").write_text(json.dumps(data, indent=2) + "\n")
    requests = [{k: value for k, value in e.items() if k in {"id", "query", "profile", "mode", "time_ms"}}
                for e in data["rendered_expressions"]]
    (out / "requests.json").write_text(json.dumps(requests, indent=2) + "\n")
    print(json.dumps({"summary": data["summary"], "private_output": str(out)}, indent=2))
    return out


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--infra", required=True, help="existing infra checkout containing the pinned commit")
    parser.add_argument("--eks-module", required=True, help="existing EKS resources module checkout containing the pinned commit")
    args = parser.parse_args()
    write_inventory(args.infra, args.eks_module)
