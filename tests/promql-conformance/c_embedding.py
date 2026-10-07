#!/usr/bin/env python3
"""Qualify the standalone C interface with actual C/C++ callers; no Agent integration."""
import argparse
import hashlib
import json
import os
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

from build import directory

HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes(root, crate):
    files = [crate / "Cargo.toml", crate / "Cargo.lock", HERE / "c_embedding.c", Path(__file__).resolve()]
    files += list((crate / "src").rglob("*.rs"))
    files += list((crate / "include").glob("*.h"))
    return {str(path.relative_to(root)): digest(path) for path in sorted(files)}


def command(task, name, args, root, env, expect_failure=False):
    print(f"{root} > {shlex.join(map(str, args))}", flush=True)
    result = subprocess.run(list(map(str, args)), cwd=root, env=env, capture_output=True, timeout=600, check=False)
    (task / (name + ".stdout")).write_bytes(result.stdout)
    (task / (name + ".stderr")).write_bytes(result.stderr)
    (task / (name + ".command.json")).write_text(json.dumps({"args": list(map(str, args)), "exit": result.returncode}) + "\n")
    if (result.returncode != 0) != expect_failure:
        raise RuntimeError(f"{name}: exit {result.returncode}; see {task / (name + '.stderr')}")
    return result


def snake(name):
    first = re.sub(r"(.)([A-Z][a-z]+)", r"\1_\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", first).lower()


def layouts(crate):
    rust = (crate / "src/c_abi/types.rs").read_text()
    header = (crate / "include/netdata-promql.h").read_text()
    shapes = {}
    for name, body in re.findall(r"pub (?:struct|union) (\w+) \{([^}]+)\}", rust):
        shapes[snake(name)] = ("types::" + name, re.findall(r"pub (\w+):", body))
    shapes["span"] = ("types::Span", ["offset", "length"])
    c_shapes = {name: re.findall(r"\b(\w+)\s*(?:\[[^]]+\])?\s*;", body)
                for name, body in re.findall(r"typedef (?:struct|union) netdata_promql_(\w+)_t \{([^}]+)\}", header)}
    if {name: fields for name, (_, fields) in shapes.items()} != c_shapes:
        raise ValueError("Rust/header structure and field rosters differ")
    callbacks = {snake(name): "Option<types::" + name + ">"
                 for name in re.findall(r"pub type (\w+)", rust) if name != "Span"}
    if set(callbacks) != set(re.findall(r"\(\*netdata_promql_(\w+)_t\)", header)):
        raise ValueError("Rust/header callback rosters differ")
    constants = {name: "types::" + name for name in re.findall(r"pub const (\w+): u32", rust)}
    constants["MEMORY_KINDS"] = "netdata_promql_engine::accounting::MEMORY_KINDS"
    if not all("#define NETDATA_PROMQL_" + name + " " in header for name in constants):
        raise ValueError("missing shared C constant")
    return shapes, callbacks, constants, header


def probes(task, shapes, callbacks, constants):
    rust = ['use netdata_promql_engine::c_abi::types;',
            'fn layout<T>(name: &str, fields: &[(&str, usize)]) {',
            ' print!("{{\\\"name\\\":\\\"{}\\\",\\\"size\\\":{},\\\"align\\\":{},\\\"fields\\\":{{", name, size_of::<T>(), align_of::<T>());',
            ' for (index, (field, offset)) in fields.iter().enumerate() {',
            '  if index != 0 { print!(","); } print!("\\\"{}\\\":{}", field, offset);',
            ' } println!("}}}}");', '}', 'fn main() {']
    c = ['#include "netdata-promql.h"', '#include <stdio.h>', '#include <inttypes.h>',
         '#ifdef __cplusplus', '#define ABI_ALIGN(T) alignof(T)', '#else',
         '#define ABI_ALIGN(T) _Alignof(T)', '#endif', 'int main(void) {']
    for name, (kind, fields) in {**shapes, **{name: (kind, []) for name, kind in callbacks.items()}}.items():
        offsets = ', '.join(f'("{field}", std::mem::offset_of!({kind}, {field}))' for field in fields)
        rust.append(f'layout::<{kind}>("{name}", &[{offsets}]);')
        c_kind = 'netdata_promql_' + name + '_t'
        c.append(f'printf("{{\\\"name\\\":\\\"{name}\\\",\\\"size\\\":%zu,\\\"align\\\":%zu,\\\"fields\\\":{{", sizeof({c_kind}), ABI_ALIGN({c_kind}));')
        for index, field in enumerate(fields):
            c.append(f'printf("{"," if index else ""}\\\"{field}\\\":%zu", offsetof({c_kind}, {field}));')
        c.append('puts("}}");')
    for name, expression in constants.items():
        rust.append(f'println!("{{{{\\\"constant\\\":\\\"{name}\\\",\\\"value\\\":{{}}}}}}", {expression});')
        c.append(f'printf("{{\\\"constant\\\":\\\"{name}\\\",\\\"value\\\":%" PRIu64 "}}\\n", (uint64_t)NETDATA_PROMQL_{name});')
    rust.append('}')
    c.append('return 0; }')
    (task / 'layout.rs').write_text('\n'.join(rust) + '\n')
    (task / 'layout.c').write_text('\n'.join(c) + '\n')


def records(output):
    return [json.loads(line) for line in output.splitlines()]


def qualify(profiles, sanitizers, abort_check):
    root = directory(HERE.parent.parent)
    crate = directory(root / "src/crates/promql-engine")
    scratch = HERE / ".tmp"
    scratch.mkdir(exist_ok=True)
    directory(scratch)
    task = directory(tempfile.mkdtemp(prefix="c-embedding-", dir=scratch))
    print(json.dumps({'evidence': str(task)}), flush=True)
    sources = source_hashes(root, crate)
    (task / 'source-sha256.json').write_text(json.dumps(sources, indent=2) + '\n')
    shapes, callbacks, constants, header = layouts(crate)
    probes(task, shapes, callbacks, constants)
    target = scratch / "core-target"
    target.mkdir(exist_ok=True)
    directory(target)
    env = dict(os.environ, CARGO_NET_OFFLINE="true", CARGO_TARGET_DIR=str(target))
    common = ['--manifest-path', crate / 'Cargo.toml', '--offline', '--locked', '--lib']
    witnesses = {}
    for profile in profiles:
        built = command(task, profile + '-static', ['cargo', 'rustc', *common, '--profile', profile,
                                                  '--', '--print', 'native-static-libs'], root, env)
        match = re.search(r'native-static-libs: (.+)', built.stderr.decode())
        if match is None:
            raise ValueError("rustc did not report native-static-libs")
        libs = shlex.split(match[1])
        output = directory(target / profile)
        archive = output / 'libnetdata_promql_engine.a'
        symbols = command(task, profile + '-symbols', ['nm', '--defined-only', '-g', archive], root, env)
        actual = {fields[-1] for line in symbols.stdout.decode().splitlines() if len(fields := line.split()) >= 3
                  and len(fields[-2]) == 1 and fields[-2].isalpha() and fields[-1].startswith('netdata_promql_')}
        expected = set(re.findall(r'uint32_t (netdata_promql_\w+)\(', header))
        if actual != expected:
            raise ValueError(f"export mismatch: missing={expected-actual}, extra={actual-expected}")
        rust_probe = task / (profile + '-rust-layout')
        command(task, profile + '-rust-layout-build', ['rustc', '--edition', '2024', task / 'layout.rs',
                '--extern', 'netdata_promql_engine=' + str(output / 'libnetdata_promql_engine.rlib'),
                '-L', 'dependency=' + str(directory(output / 'deps')), '-o', rust_probe], root, env)
        rust_layout = records(command(task, profile + '-rust-layout', [rust_probe], root, env).stdout)
        for compiler, language, standard in [('gcc', 'c', 'c11'), ('g++', 'c++', 'c++17'),
                                             ('clang', 'c', 'c11'), ('clang++', 'c++', 'c++17')]:
            name = profile + '-' + compiler.replace('+', 'p')
            flags = [compiler, '-x', language, '-std=' + standard, '-O2', '-Wall', '-Wextra', '-Werror',
                     '-I', crate / 'include']
            layout_binary = task / (name + '-layout')
            command(task, name + '-layout-build', [*flags, task / 'layout.c', '-o', layout_binary], root, env)
            c_layout = records(command(task, name + '-layout', [layout_binary], root, env).stdout)
            if c_layout != rust_layout:
                raise ValueError(name + ': C/Rust layout or shared constant mismatch')
            binary = task / name
            command(task, name + '-build', [*flags, HERE / 'c_embedding.c', '-x', 'none', archive,
                                          *libs, '-o', binary], root, env)
            values = records(command(task, name, [binary], root, env).stdout)
            if (len(values) != 8 or values[-2] != {'label_field_boundary': 'pass', 'cases': 6}
                    or values[-1] != {'native_embedding': 'pass'}):
                raise ValueError(name + ': incomplete native witnesses')
            witnesses[name] = values
        if sanitizers:
            name = profile + '-clang-sanitized'
            binary = task / name
            command(task, name + '-build', ['clang', '-std=c11', '-O1', '-g', '-Wall', '-Wextra', '-Werror',
                    '-fsanitize=address,undefined', '-fno-omit-frame-pointer', '-I', crate / 'include',
                    HERE / 'c_embedding.c', archive, *libs, '-o', binary], root, env)
            sanitizer_env = dict(env, ASAN_OPTIONS='detect_leaks=1:halt_on_error=1', UBSAN_OPTIONS='halt_on_error=1')
            witnesses[name] = records(command(task, name, [binary], root, sanitizer_env).stdout)
    if abort_check:
        abort_target = scratch / 'c-embedding-abort-target'
        abort_target.mkdir(exist_ok=True)
        directory(abort_target)
        abort_env = dict(env, CARGO_TARGET_DIR=str(abort_target), CARGO_PROFILE_RELEASE_PANIC='abort')
        negative = command(task, 'abort-negative', ['cargo', 'check', *common, '--release'], root, abort_env, True)
        if b'requires panic=unwind' not in negative.stderr:
            raise ValueError('abort-negative failed without the required panic guard diagnostic')
    if sources != source_hashes(root, crate):
        raise ValueError('source changed during C embedding qualification')
    (task / 'witnesses.json').write_text(json.dumps(witnesses, indent=2) + '\n')
    artifacts = {str(path.relative_to(task)): digest(path) for path in sorted(task.rglob('*')) if path.is_file()}
    (task / 'manifest.json').write_text(json.dumps({'sources': sources, 'artifacts': artifacts,
            'profiles': profiles, 'structures': len(shapes), 'callbacks': len(callbacks),
            'shared_constants': len(constants), 'sanitizers': sanitizers,
            'sanitizer_scope': 'foreign caller; Rust core is not instrumented',
            'abort_negative': abort_check}, indent=2) + '\n')
    print(json.dumps({'native_embedding': 'pass', 'evidence': str(task), 'structures': len(shapes),
                     'callbacks': len(callbacks), 'shared_constants': len(constants)}), flush=True)
    return task


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profiles', nargs='+', choices=['release', 'release-min'], default=['release', 'release-min'])
    parser.add_argument('--sanitizers', action='store_true', help='instrument foreign C caller with ASan/UBSan')
    parser.add_argument('--abort-check', action='store_true', help='also require panic=abort compilation rejection')
    args = parser.parse_args()
    qualify(args.profiles, args.sanitizers, args.abort_check)
