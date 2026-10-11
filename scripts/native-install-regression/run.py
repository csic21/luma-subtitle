#!/usr/bin/env python3
"""Build and run the feature-only real Wry IPC installer regressions.

Every case is a new child process. Native faults/stack overflow cannot abort the
controller or hide later cases. No administrator/debugger configuration is changed.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
MARKER = b"LUMA_NATIVE_INSTALL_REGRESSION_NOT_FOR_DISTRIBUTION"
COMMON_CASES = [
    "metrics", "engine-cold", "engine-cached", "engine-stale", "engine-missing-dll",
    "engine-extraction-failure", "engine-activation-failure", "engine-wrong-hash",
    "engine-network-rejection", "engine-concurrent", "engine-nested-model-failure",
    "model-cold", "model-cached", "model-corrupt-cached", "model-truncated",
    "model-wrong-hash", "model-network-rejection", "model-activation-failure", "model-concurrent",
]
AB_CASES = ["metrics", "engine-cold", "engine-cached", "model-cold", "model-cached"]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pe_stack(path: Path) -> dict | None:
    data = path.read_bytes()
    if data[:2] != b"MZ":
        return None
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Invalid PE signature")
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    if magic == 0x20B:  # PE32+
        reserve, commit = struct.unpack_from("<QQ", data, optional + 72)
    elif magic == 0x10B:
        reserve, commit = struct.unpack_from("<II", data, optional + 72)
    else:
        raise ValueError(f"Unknown PE optional-header magic {magic:#x}")
    return {"SizeOfStackReserve": reserve, "SizeOfStackCommit": commit,
            "machine": hex(struct.unpack_from("<H", data, pe + 4)[0])}


def command_text(args: list[str]) -> str:
    result = subprocess.run(args, cwd=REPO, text=True, capture_output=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else f"unavailable: {result.stderr.strip()}"


def check_fixtures() -> dict:
    root = REPO / "src-tauri/native-install-regression/fixtures"
    manifest = json.loads((root / "manifest.json").read_text())
    for name, expected in manifest.items():
        path = root / name
        assert path.stat().st_size == expected["bytes"], f"Fixture size mismatch: {name}"
        assert digest(path) == expected["sha256"], f"Fixture hash mismatch: {name}"
    return manifest


def build(variant: str, output: Path) -> tuple[Path, dict]:
    # Share dependency compilation across feature variants, never with production.
    target = REPO / "src-tauri/target/native-install-regression"
    feature = "native-install-regression" + ("-inline-buffer" if variant == "inline" else "")
    env = os.environ.copy()
    env.update({"CARGO_TARGET_DIR": str(target), "LUMA_NATIVE_REGRESSION_BUILD": "1"})
    # Do not alter RUSTFLAGS/linker stack options. Record inherited build inputs.
    args = ["cargo", "build", "--manifest-path", "src-tauri/Cargo.toml", "--locked", "--release", "--features", feature]
    log = output / f"build-{variant}.log"
    print(f"Building {variant} native fixture application", flush=True)
    with log.open("w", encoding="utf-8") as stream:
        result = subprocess.run(args, cwd=REPO, env=env, stdout=stream, stderr=subprocess.STDOUT, check=False)
    if result.returncode:
        print(log.read_text(encoding="utf-8", errors="replace")[-18000:], file=sys.stderr)
        raise RuntimeError(f"{variant} diagnostic build failed; see {log}")
    built = target / "release" / ("luma-subtitle.exe" if os.name == "nt" else "luma-subtitle")
    if not built.is_file() or MARKER not in built.read_bytes():
        raise RuntimeError("Expected feature-only fixture executable was not produced")
    variant_dir = output / variant
    variant_dir.mkdir(exist_ok=True)
    executable = variant_dir / built.name
    shutil.copy2(built, executable)
    return executable, {"command": args, "target_dir": str(target), "sha256": digest(executable),
                        "pe_stack": pe_stack(executable), "bytes": executable.stat().st_size,
                        "preserved_executable": str(executable)}


def collect_application_errors(start: float, output: Path) -> None:
    if os.name != "nt":
        return
    # Read already available Application Error/WER events only. No registry, crash
    # dump or permission settings are changed. Absence is reported, not a pass.
    script = (
        f"$start = [DateTimeOffset]::FromUnixTimeSeconds({int(start)}).LocalDateTime; "
        "$events = Get-WinEvent -FilterHashtable @{LogName='Application';StartTime=$start;Id=1000,1001} -ErrorAction SilentlyContinue | "
        "Where-Object {$_.Message -like '*luma-subtitle*'} | "
        "Select-Object TimeCreated,Id,ProviderName,Message; "
        "ConvertTo-Json -InputObject @($events) -Depth 4"
    )
    result = subprocess.run(["powershell", "-NoProfile", "-Command", script], capture_output=True, text=True, check=False)
    (output / "windows-application-errors.json").write_text(result.stdout.strip() or "[]", encoding="utf-8")
    (output / "windows-application-errors.stderr.txt").write_text(result.stderr, encoding="utf-8")


def run_case(executable: Path, case: str, variant: str, output: Path, timeout: int) -> dict:
    directory = output / variant
    directory.mkdir(exist_ok=True)
    report_path = directory / f"{case}.json"
    started = time.monotonic()
    # Each native runner creates its own UUID temp root. The working directory is
    # also temporary and empty, excluding development-resource discovery.
    work = directory / f"{case}-cwd"
    work.mkdir()
    env = os.environ.copy()
    for key in ("RUST_MIN_STACK",):
        env.pop(key, None)
    with (directory / f"{case}.stdout.txt").open("wb") as stdout, (directory / f"{case}.stderr.txt").open("wb") as stderr:
        process = subprocess.Popen([str(executable), "--case", case, "--report", str(report_path)],
                                   cwd=work, env=env, stdout=stdout, stderr=stderr)
        timed_out = False
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True, check=False)
            else:
                process.kill()
            code = process.wait(timeout=30)
    report = json.loads(report_path.read_text(encoding="utf-8")) if report_path.exists() else None
    result = {"case": case, "exit_code": code, "exit_code_hex": f"0x{code & 0xFFFFFFFF:08X}",
              "timed_out": timed_out, "seconds": round(time.monotonic() - started, 3),
              "report_present": report is not None,
              "passed": not timed_out and code == 0 and report is not None and report.get("passed") is True,
              "assertion_error": report.get("error") if report else None}
    print(f"{variant}/{case}: {'PASS' if result['passed'] else 'FAIL'} exit={result['exit_code_hex']} {result['assertion_error'] or ''}", flush=True)
    return result


def verify_production(binary: Path, output: Path) -> None:
    if MARKER in binary.read_bytes():
        raise RuntimeError("REFUSING DISTRIBUTION: production artifact contains the diagnostic fixture marker")
    value = {"path": str(binary), "sha256": digest(binary), "pe_stack": pe_stack(binary), "diagnostic_marker_absent": True}
    summary_path = output / "summary.json"
    if summary_path.exists():
        summary = json.loads(summary_path.read_text())
        if os.name == "nt":
            expected = summary["variants"]["heap"]["build"]["pe_stack"]
            if value["pe_stack"] != expected:
                raise RuntimeError(f"Diagnostic and production PE stack settings differ: {expected} vs {value['pe_stack']}")
        value["diagnostic_build_is_separate"] = True
    output.mkdir(parents=True, exist_ok=True)
    (output / "production-binary.json").write_text(json.dumps(value, indent=2) + "\n")
    print(json.dumps(value, indent=2))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=REPO / "native-install-regression-results")
    parser.add_argument("--ab", action="store_true", help="Also observe original inline buffer on the same candidate source; failures are evidence, not required")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--verify-production", type=Path)
    parser.add_argument("--check-fixtures", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    if args.check_fixtures:
        print(json.dumps(check_fixtures(), indent=2)); return 0
    if args.verify_production:
        verify_production(args.verify_production.resolve(), output); return 0
    if platform.system() not in ("Windows", "Darwin"):
        parser.error("Real Wry regression must run natively on Windows or macOS Apple Silicon")
    if output.exists() and any(output.iterdir()):
        parser.error("Use an empty output directory so stale reports cannot satisfy assertions")
    output.mkdir(parents=True, exist_ok=True)
    started = time.time()
    summary = {"source_sha": command_text(["git", "rev-parse", "HEAD"]),
        "source_status": command_text(["git", "status", "--porcelain"]),
        "rustc": command_text(["rustc", "-vV"]), "cargo": command_text(["cargo", "--version"]),
        "host": platform.platform(), "cargo_lock_sha256": digest(REPO / "src-tauri/Cargo.lock"),
        "build_environment": {key: os.environ.get(key) for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUST_MIN_STACK", "CARGO_BUILD_TARGET")},
        "fixtures": check_fixtures(), "variants": {},
        "scope": "Release-mode feature-built application; real Wry JS IPC and generated production command wrappers. The normal release artifact is built and checked separately.",
        "ab_scope": "Candidate source with only hash-buffer representation changed. NOT original v1.2.2. If both pass, this is NOT a reproduction or proof of causality.",
        "dump_collection": "Only existing Windows Application Error/WER log entries. No minidump or native call-stack collection is configured by this runner."}
    failed = False
    try:
        for variant in (["heap", "inline"] if args.ab else ["heap"]):
            executable, metadata = build(variant, output)
            cases = COMMON_CASES + (["engine-rename-failure", "model-rename-failure"] if os.name == "nt" else []) if variant == "heap" else AB_CASES
            results = [run_case(executable, case, variant, output, args.timeout) for case in cases]
            summary["variants"][variant] = {"build": metadata, "cases": results}
            if variant == "heap" and not all(case["passed"] for case in results):
                failed = True
            (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    except Exception as error:
        summary["controller_error"] = str(error)
        failed = True
        print(str(error), file=sys.stderr)
    finally:
        collect_application_errors(started, output)
        summary["fixed_suite_passed"] = not failed
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
