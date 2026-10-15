#!/usr/bin/env python3
"""Qualify the standalone C interface with actual C/C++ callers; no Agent integration."""

import argparse
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from build import directory

HERE = Path(__file__).resolve().parent
PREVIOUS_HEADER_COMMIT = "f14c541028a9047da34bed7eb41b705abe9bd741"
PREVIOUS_HEADER_SHA256 = "5e8227777044dfc75b3fa5a7da1a7e1edf57ba512ffd6a8ab0324b0f2b00bb00"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes(root, crate):
    files = [
        crate / "Cargo.toml",
        crate / "Cargo.lock",
        HERE / "c_embedding.c",
        HERE / "abi3-header.h",
        HERE / "abi3-rejection.c",
        Path(__file__).resolve(),
    ]
    files += list((crate / "src").rglob("*.rs"))
    files += list((crate / "include").glob("*.h"))
    if digest(HERE / "abi3-header.h") != PREVIOUS_HEADER_SHA256:
        raise ValueError("previous interface header differs from its pinned source")
    return {str(path.relative_to(root)): digest(path) for path in sorted(files)}


def command(task, name, args, root, env, expect_failure=False):
    print(f"{root} > {shlex.join(map(str, args))}", flush=True)
    args = list(map(str, args))
    started = time.time_ns()
    stdout = task / (name + ".stdout")
    stderr = task / (name + ".stderr")
    with stdout.open("xb") as out, stderr.open("xb") as err:
        process = subprocess.Popen(args, cwd=root, env=env, stdout=out, stderr=err)
        (task / (name + ".process.json")).write_text(
            json.dumps(
                {
                    "pid": process.pid,
                    "args": args,
                    "cwd": str(root),
                    "started_unix_ns": started,
                    "automatic_signal": False,
                }
            )
            + "\n"
        )
        returned = process.wait()
    result = subprocess.CompletedProcess(
        args, returned, stdout.read_bytes(), stderr.read_bytes()
    )
    (task / (name + ".command.json")).write_text(
        json.dumps(
            {
                "args": args,
                "pid": process.pid,
                "exit": returned,
                "elapsed_seconds": (time.time_ns() - started) / 1e9,
            }
        )
        + "\n"
    )
    if (result.returncode != 0) != expect_failure:
        raise RuntimeError(
            f"{name}: exit {result.returncode}; see {task / (name + '.stderr')}"
        )
    return result


def link_command(task, name, args, root, env, inputs, expected):
    def hashes():
        paths = inputs() if callable(inputs) else inputs
        return {str(path.resolve(strict=True)): digest(path) for path in paths}

    files = hashes()
    if files != expected:
        raise ValueError(name + ": link input or roster changed before command")
    result = command(task, name, args, root, env)
    if files != hashes():
        raise ValueError(name + ": link input or roster changed during command")
    (task / (name + ".link-inputs.json")).write_text(
        json.dumps(
            {
                "files": files,
                "unchanged": True,
            },
            indent=2,
        )
        + "\n"
    )
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
    c_shapes = {
        name: re.findall(r"\b(\w+)\s*(?:\[[^]]+\])?\s*;", body)
        for name, body in re.findall(
            r"typedef (?:struct|union) netdata_promql_(\w+)_t \{([^}]+)\}", header
        )
    }
    if {name: fields for name, (_, fields) in shapes.items()} != c_shapes:
        raise ValueError("Rust/header structure and field rosters differ")
    callbacks = {
        snake(name): "Option<types::" + name + ">"
        for name in re.findall(r"pub type (\w+)", rust)
        if name != "Span"
    }
    if set(callbacks) != set(re.findall(r"\(\*netdata_promql_(\w+)_t\)", header)):
        raise ValueError("Rust/header callback rosters differ")
    constants = {
        name: "types::" + name for name in re.findall(r"pub const (\w+): u32", rust)
    }
    constants["MEMORY_KINDS"] = "netdata_promql_engine::accounting::MEMORY_KINDS"
    if not all("#define NETDATA_PROMQL_" + name + " " in header for name in constants):
        raise ValueError("missing shared C constant")
    return shapes, callbacks, constants, header


def probes(task, shapes, callbacks, constants):
    rust = [
        "use netdata_promql_engine::c_abi::types;",
        "use netdata_promql_engine::storage::{AdaptedPage, PageSamples, SamplePage};",
        "fn copy_traits<T: Copy + Clone + std::fmt::Debug + Send + Sync + std::panic::RefUnwindSafe + std::panic::UnwindSafe>() {}",
        "fn page_traits<T: std::fmt::Debug + Send + Sync + std::panic::RefUnwindSafe + std::panic::UnwindSafe>() {}",
        "fn narrow<'short, 'long: 'short>(page: PageSamples<'long>) -> PageSamples<'short> {",
        " match page { PageSamples::Views(v) => PageSamples::Views(v), PageSamples::Samples(v) => PageSamples::Samples(v), PageSamples::Adapted(v) => PageSamples::Adapted(v) }",
        "}",
        "fn narrow_adapted<'short, 'long: 'short>(page: AdaptedPage<'long>) -> AdaptedPage<'short> { page }",
        "fn narrow_page<'short, 'long: 'short>(page: SamplePage<'long>) -> SamplePage<'short> { page }",
        "fn layout<T>(name: &str, fields: &[(&str, usize)]) {",
        ' print!("{{\\"name\\":\\"{}\\",\\"size\\":{},\\"align\\":{},\\"fields\\":{{", name, size_of::<T>(), align_of::<T>());',
        " for (index, (field, offset)) in fields.iter().enumerate() {",
        '  if index != 0 { print!(","); } print!("\\"{}\\":{}", field, offset);',
        ' } println!("}}}}");',
        "}",
        "fn main() {",
        " copy_traits::<PageSamples<'_>>(); copy_traits::<AdaptedPage<'_>>(); page_traits::<SamplePage<'_>>();",
        " let samples = narrow(PageSamples::Views(&[])); assert!(samples.is_empty());",
        " let _ = (narrow_adapted, narrow_page);",
    ]
    c = [
        '#include "netdata-promql.h"',
        "#include <stdio.h>",
        "#include <inttypes.h>",
        "#ifdef __cplusplus",
        "#define ABI_ALIGN(T) alignof(T)",
        "#else",
        "#define ABI_ALIGN(T) _Alignof(T)",
        "#endif",
        "int main(void) {",
    ]
    for name, (kind, fields) in {
        **shapes,
        **{name: (kind, []) for name, kind in callbacks.items()},
    }.items():
        offsets = ", ".join(
            f'("{field}", std::mem::offset_of!({kind}, {field}))' for field in fields
        )
        rust.append(f'layout::<{kind}>("{name}", &[{offsets}]);')
        c_kind = "netdata_promql_" + name + "_t"
        c.append(
            f'printf("{{\\"name\\":\\"{name}\\",\\"size\\":%zu,\\"align\\":%zu,\\"fields\\":{{", sizeof({c_kind}), ABI_ALIGN({c_kind}));'
        )
        for index, field in enumerate(fields):
            c.append(
                f'printf("{"," if index else ""}\\"{field}\\":%zu", offsetof({c_kind}, {field}));'
            )
        c.append('puts("}}");')
    for name, expression in constants.items():
        rust.append(
            f'println!("{{{{\\"constant\\":\\"{name}\\",\\"value\\":{{}}}}}}", {expression});'
        )
        c.append(
            f'printf("{{\\"constant\\":\\"{name}\\",\\"value\\":%" PRIu64 "}}\\n", (uint64_t)NETDATA_PROMQL_{name});'
        )
    rust.append("}")
    c.append("return 0; }")
    (task / "layout.rs").write_text("\n".join(rust) + "\n")
    (task / "layout.c").write_text("\n".join(c) + "\n")


def records(output):
    return [json.loads(line) for line in output.splitlines()]


def native_records(name, output):
    values = records(output)
    tail = [
        {"temporary_failures": "pass", "cases": 11, "callback_phases": 5},
        {"basic_statistics": "pass", "cases": 21, "page_sizes": 3},
        {
            "foreign_accounting": "pass",
            "callback_phases": 9,
            "overlap_bytes": 7340032,
            "retained_bytes": 4096,
        },
        {"foreign_early_abort": "pass", "cases": 11},
        {"histogram_buckets": "pass", "cases": 7},
        {"index_sort": "pass", "cases": 6},
        {"precise_requests": "pass", "cases": 2},
        {"protocol_primitives": "pass"},
        {"metadata_discovery": "pass", "cases": 6},
        {"label_field_boundary": "pass", "cases": 6},
        {"unique_binary_replay": "pass", "cases": 9, "page_sizes": 3},
        {"instant_numeric_folds": "pass", "cases": 6, "page_sizes": 3},
        {"scheduled_quantile_rank": "pass", "cases": 6, "page_sizes": 3},
        {"direct_rate_replay": "pass", "cases": 12, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "abs(m) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "m / on(zone) abs(m)", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "abs(m) / on(zone) abs(m)", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "ceil(m) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "m / on(zone) round(m)", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "sqrt(m) / on(zone) sqrt(m)", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "year(m) / on(zone) year(m)", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "round(m,.5) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "clamp_min(m,0) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "clamp_max(m,4) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "clamp(m,0,4) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "round(abs(m),.5) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "abs((m*1)) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "abs(timestamp(+m)) / on(zone) abs(timestamp(+m))", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "m / on(zone) round(abs(m),.5)", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "(m*1) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "m / on(zone) (m*1)", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "(+m) / on(zone) m", "cases": 9, "page_sizes": 3},
        {"direct_float_map_replay": "pass", "expression": "label_join(m, \"zone\", \"\", \"zone\") / on(zone) m", "cases": 9, "page_sizes": 3},
        {"average_read_reuse": "pass", "cases": 15, "page_sizes": 3},
        {"foreign_allocation_lifetime": "pass", "bytes": 16384, "abort_cases": 3},
        {"native_embedding": "pass"},
    ]
    expected = [(rows, size) for size in (1, 7, 1024) for rows in (8192, 65536)]
    actual = [(value.get("rows"), value.get("page_size")) for value in values[:6]]
    temporary = [
        (value.get("temporary_rows"), value.get("page_size")) for value in values[6:12]
    ]
    temporary_expected = [
        (rows, size) for size in (1, 17, 1024) for rows in (2048, 100000)
    ]
    if (
        len(values) != 12 + len(tail)
        or actual != expected
        or temporary != temporary_expected
        or values[12:] != tail
    ):
        raise ValueError(name + ": incomplete native witnesses")
    return values


def qualify(profiles, sanitizers, abort_check, cross_target=None):
    targets = {
        "aarch64-unknown-linux-gnu": (
            "aarch64-linux-gnu",
            "qemu-aarch64",
            "ld-linux-aarch64.so.1",
        ),
        "armv7-unknown-linux-gnueabihf": (
            "arm-linux-gnueabihf",
            "qemu-arm",
            "ld-linux-armhf.so.3",
        ),
    }
    if cross_target and (cross_target not in targets or sanitizers):
        raise ValueError(
            "cross qualification uses an installed GNU target without caller instrumentation"
        )
    root = directory(HERE.parent.parent)
    crate = root
    scratch = HERE / ".tmp"
    scratch.mkdir(exist_ok=True)
    directory(scratch)
    task = directory(tempfile.mkdtemp(prefix="c-embedding-", dir=scratch))
    print(json.dumps({"evidence": str(task)}), flush=True)
    sources = source_hashes(root, crate)
    (task / "source-sha256.json").write_text(json.dumps(sources, indent=2) + "\n")
    shapes, callbacks, constants, header = layouts(crate)
    probes(task, shapes, callbacks, constants)
    target = scratch / (
        "core-target-" + cross_target if cross_target else "core-target"
    )
    target.mkdir(exist_ok=True)
    directory(target)
    env = dict(
        os.environ,
        CARGO_NET_OFFLINE="true",
        CARGO_TARGET_DIR=str(target),
        NETDATA_PROMQL_FIXTURE_SCRATCH=str(task),
        CARGO_BUILD_JOBS="4",
    )
    common = ["--manifest-path", crate / "Cargo.toml", "--offline", "--locked", "--lib"]
    runner = []
    rust_target = []
    nm = "nm"
    compilers = [
        ("gcc", "c", "c11"),
        ("g++", "c++", "c++17"),
        ("clang", "c", "c11"),
        ("clang++", "c++", "c++17"),
    ]
    tools = {}
    if cross_target:
        triplet, qemu, loader = targets[cross_target]
        cc = triplet + "-gcc-14"
        compilers = [(cc, "c", "c11"), (triplet + "-g++-14", "c++", "c++17")]
        nm = triplet + "-nm"
        env["CARGO_TARGET_" + cross_target.replace("-", "_").upper() + "_LINKER"] = cc
        common += ["--target", cross_target]
        rust_target = ["--target", cross_target, "-Clinker=" + cc]
        selected = (
            command(
                task, "target-loader", [cc, "-print-file-name=" + loader], root, env
            )
            .stdout.decode()
            .strip()
        )
        prefix = Path(selected).resolve(strict=True).parent.parent
        if (
            not prefix.is_absolute()
            or not prefix.is_dir()
            or prefix.name != triplet
            or not (prefix / "lib" / loader).is_file()
        ):
            raise ValueError(
                "target compiler did not supply the expected loader prefix"
            )
        runner = [qemu, "-L", str(prefix)]
        for tool in [cc, triplet + "-g++-14", nm, qemu, "cargo", "rustc"]:
            executable = shutil.which(tool, path=env["PATH"])
            if executable is None:
                raise ValueError("missing installed target tool: " + tool)
            executable = Path(executable).resolve(strict=True)
            tools[str(executable)] = digest(executable)
            command(
                task,
                "version-" + tool.replace("+", "p"),
                [tool, "--version"],
                root,
                env,
            )
        sysroot = Path(
            command(task, "rust-sysroot", ["rustc", "--print", "sysroot"], root, env)
            .stdout.decode()
            .strip()
        ).resolve(strict=True)
        for name in ["cargo", "rustc"]:
            executable = sysroot / "bin" / name
            if not executable.is_file():
                raise ValueError("selected Rust toolchain executable is missing")
            tools[str(executable)] = digest(executable)
        tools[str((prefix / "lib" / loader).resolve(strict=True))] = digest(
            (prefix / "lib" / loader).resolve(strict=True)
        )

    def execute(name, binary, execution_env=env):
        executable_inputs = [binary, *[Path(path) for path in tools]]
        hashes = {
            str(path.resolve(strict=True)): digest(path) for path in executable_inputs
        }
        return link_command(
            task,
            name,
            [*runner, binary],
            root,
            execution_env,
            executable_inputs,
            hashes,
        )

    witnesses = {}
    link_inputs = {}
    for profile in profiles:
        built = command(
            task,
            profile + "-static",
            [
                "cargo",
                "rustc",
                *common,
                "--profile",
                profile,
                "--",
                "--print",
                "native-static-libs",
            ],
            root,
            env,
        )
        match = re.search(r"native-static-libs: (.+)", built.stderr.decode())
        if match is None:
            raise ValueError("rustc did not report native-static-libs")
        libs = shlex.split(match[1])
        output = directory(target / (cross_target or "") / profile)
        archive = output / "libnetdata_promql_engine.a"
        archive_inputs = {str(archive.resolve(strict=True)): digest(archive)}
        rlib = output / "libnetdata_promql_engine.rlib"

        def rust_inputs(rlib=rlib, output=output):
            return [
                rlib,
                *sorted((output / "deps").glob("*.rlib")),
                *sorted((output / "deps").glob("*.so")),
            ]

        rust_hashes = {
            str(path.resolve(strict=True)): digest(path) for path in rust_inputs()
        }
        link_inputs[profile] = {
            "static_archive": archive_inputs,
            "rust_layout": rust_hashes,
        }
        symbols = link_command(
            task,
            profile + "-symbols",
            [nm, "--defined-only", "-g", archive],
            root,
            env,
            [archive],
            archive_inputs,
        )
        actual = {
            fields[-1]
            for line in symbols.stdout.decode().splitlines()
            if len(fields := line.split()) >= 3
            and len(fields[-2]) == 1
            and fields[-2].isalpha()
            and fields[-1].startswith("netdata_promql_")
        }
        expected = set(re.findall(r"uint32_t (netdata_promql_\w+)\(", header))
        if actual != expected:
            raise ValueError(
                f"export mismatch: missing={expected - actual}, extra={actual - expected}"
            )
        rust_probe = task / (profile + "-rust-layout")
        link_command(
            task,
            profile + "-rust-layout-build",
            [
                "rustc",
                "--edition",
                "2024",
                *rust_target,
                task / "layout.rs",
                "--extern",
                "netdata_promql_engine=" + str(rlib),
                "-L",
                "dependency=" + str(directory(output / "deps")),
                "-o",
                rust_probe,
            ],
            root,
            env,
            rust_inputs,
            rust_hashes,
        )
        rust_layout = records(execute(profile + "-rust-layout", rust_probe).stdout)
        if len(rust_layout) != len(shapes) + len(callbacks) + len(constants):
            raise ValueError("incomplete Rust layout and constant roster")
        for compiler, language, standard in compilers:
            name = profile + "-" + compiler.replace("+", "p")
            flags = [
                compiler,
                "-x",
                language,
                "-std=" + standard,
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-I",
                crate / "include",
            ]
            layout_binary = task / (name + "-layout")
            command(
                task,
                name + "-layout-build",
                [*flags, task / "layout.c", "-o", layout_binary],
                root,
                env,
            )
            c_layout = records(execute(name + "-layout", layout_binary).stdout)
            if c_layout != rust_layout:
                raise ValueError(name + ": C/Rust layout or shared constant mismatch")
            binary = task / name
            link_command(
                task,
                name + "-build",
                [
                    *flags,
                    HERE / "c_embedding.c",
                    "-x",
                    "none",
                    archive,
                    *libs,
                    "-o",
                    binary,
                ],
                root,
                env,
                [archive],
                archive_inputs,
            )
            witnesses[name] = native_records(name, execute(name, binary).stdout)
            previous_name = name + "-abi3"
            previous_binary = task / previous_name
            previous_inputs = [archive, HERE / "abi3-header.h", HERE / "abi3-rejection.c"]
            previous_hashes = {
                str(path.resolve(strict=True)): digest(path) for path in previous_inputs
            }
            link_command(
                task,
                previous_name + "-build",
                [
                    *flags,
                    HERE / "abi3-rejection.c",
                    "-x",
                    "none",
                    archive,
                    *libs,
                    "-o",
                    previous_binary,
                ],
                root,
                env,
                previous_inputs,
                previous_hashes,
            )
            previous = records(execute(previous_name, previous_binary).stdout)
            if previous != [{
                "old_header_rejected": "pass",
                "caller_abi": 3,
                "library_abi": 4,
                "engine_options": "rejected",
            }]:
                raise ValueError(previous_name + ": incomplete previous-header rejection")
            witnesses[previous_name] = previous
        if sanitizers:
            name = profile + "-clang-sanitized"
            binary = task / name
            link_command(
                task,
                name + "-build",
                [
                    "clang",
                    "-std=c11",
                    "-O1",
                    "-g",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-fsanitize=address,undefined",
                    "-fno-omit-frame-pointer",
                    "-I",
                    crate / "include",
                    HERE / "c_embedding.c",
                    archive,
                    *libs,
                    "-o",
                    binary,
                ],
                root,
                env,
                [archive],
                archive_inputs,
            )
            sanitizer_env = dict(
                env,
                ASAN_OPTIONS="detect_leaks=1:halt_on_error=1",
                UBSAN_OPTIONS="halt_on_error=1",
            )
            witnesses[name] = native_records(
                name, execute(name, binary, sanitizer_env).stdout
            )
    if abort_check:
        abort_target = scratch / "c-embedding-abort-target"
        abort_target.mkdir(exist_ok=True)
        directory(abort_target)
        abort_env = dict(
            env, CARGO_TARGET_DIR=str(abort_target), CARGO_PROFILE_RELEASE_PANIC="abort"
        )
        negative = command(
            task,
            "abort-negative",
            ["cargo", "check", *common, "--release"],
            root,
            abort_env,
            True,
        )
        if b"requires panic=unwind" not in negative.stderr:
            raise ValueError(
                "abort-negative failed without the required panic guard diagnostic"
            )
    if sources != source_hashes(root, crate):
        raise ValueError("source changed during C embedding qualification")
    if any(digest(Path(path)) != value for path, value in tools.items()):
        raise ValueError(
            "target tool or loader changed during C embedding qualification"
        )
    (task / "witnesses.json").write_text(json.dumps(witnesses, indent=2) + "\n")
    artifacts = {
        str(path.relative_to(task)): digest(path)
        for path in sorted(task.rglob("*"))
        if path.is_file()
    }
    (task / "manifest.json").write_text(
        json.dumps(
            {
                "sources": sources,
                "artifacts": artifacts,
                "link_inputs": link_inputs,
                "profiles": profiles,
                "target": cross_target,
                "runner": runner,
                "tools": tools,
                "structures": len(shapes),
                "callbacks": len(callbacks),
                "shared_constants": len(constants),
                "sanitizers": sanitizers,
                "sanitizer_scope": "foreign caller; Rust core is not instrumented",
                "abort_negative": abort_check,
                "previous_interface": {
                    "version": 3,
                    "header_commit": PREVIOUS_HEADER_COMMIT,
                    "header_sha256": PREVIOUS_HEADER_SHA256,
                    "rejection": "version handshake and otherwise valid engine options",
                },
            },
            indent=2,
        )
        + "\n"
    )
    print(
        json.dumps(
            {
                "native_embedding": "pass",
                "target": cross_target,
                "evidence": str(task),
                "structures": len(shapes),
                "callbacks": len(callbacks),
                "shared_constants": len(constants),
            }
        ),
        flush=True,
    )
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--profiles",
        nargs="+",
        choices=["release", "release-min"],
        default=["release", "release-min"],
    )
    parser.add_argument(
        "--target",
        choices=["aarch64-unknown-linux-gnu", "armv7-unknown-linux-gnueabihf"],
        help="compile and execute GNU target callers and layouts with the installed QEMU loader",
    )
    parser.add_argument(
        "--sanitizers",
        action="store_true",
        help="instrument foreign C caller with ASan/UBSan",
    )
    parser.add_argument(
        "--abort-check",
        action="store_true",
        help="also require panic=abort compilation rejection",
    )
    args = parser.parse_args()
    qualify(args.profiles, args.sanitizers, args.abort_check, args.target)
