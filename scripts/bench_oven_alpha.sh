#!/usr/bin/env bash

# Measure one compiler-shipped Oven Alpha workload without making Cargo part of the normal-command benchmark.
#
# A release archive supplies the supported native units. This harness starts with an empty Oven store, records the
# first normal command (seed materialization plus its caller-owned native bake), then records unchanged warm commands.
# An optional failing-Cargo directory may be prepended to PATH to make a consumer-side Cargo launch fail the evidence.

set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  bash scripts/bench_oven_alpha.sh \
    --incan PATH --workload build|run|test --source PATH \
    --incan-home PATH --output PATH [options]

The selected source must be in the documented compiler-shipped Oven Alpha envelope. The harness requires an empty
INCAN_HOME, records the first normal command separately from unchanged warm repeats, and never invokes Cargo.

Options:
  --repetitions N                 Unchanged warm repeats after first materialization (default: 2; minimum: 1)
  --cargo-guard-dir PATH          Directory containing a failing `cargo` executable to prepend to PATH
  --max-physical-bytes BYTES      Aggregate Oven physical-store policy (default: 2147483648)
  --max-domain-physical-bytes N   Per-domain Oven physical-store policy (default: 1073741824)
  --max-domain-logical-bytes N    Per-domain Oven logical-artifact policy (default: 805306368)
  -h, --help                      Show this help
EOF
}

incan=""
workload=""
source_path=""
incan_home=""
output_dir=""
repetitions=2
cargo_guard_dir=""
max_physical_bytes=2147483648
max_domain_physical_bytes=1073741824
max_domain_logical_bytes=805306368

while [ "$#" -gt 0 ]; do
    case "$1" in
        --incan) incan=${2:?--incan requires a path}; shift 2 ;;
        --workload) workload=${2:?--workload requires build, run, or test}; shift 2 ;;
        --source) source_path=${2:?--source requires a path}; shift 2 ;;
        --incan-home) incan_home=${2:?--incan-home requires a path}; shift 2 ;;
        --output) output_dir=${2:?--output requires a path}; shift 2 ;;
        --repetitions) repetitions=${2:?--repetitions requires a number}; shift 2 ;;
        --cargo-guard-dir) cargo_guard_dir=${2:?--cargo-guard-dir requires a directory}; shift 2 ;;
        --max-physical-bytes) max_physical_bytes=${2:?--max-physical-bytes requires bytes}; shift 2 ;;
        --max-domain-physical-bytes) max_domain_physical_bytes=${2:?--max-domain-physical-bytes requires bytes}; shift 2 ;;
        --max-domain-logical-bytes) max_domain_logical_bytes=${2:?--max-domain-logical-bytes requires bytes}; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

for required in incan workload source_path incan_home output_dir; do
    if [ -z "${!required}" ]; then
        echo "missing required --${required//_/-}" >&2
        usage >&2
        exit 2
    fi
done

case "$workload" in
    build|run|test) ;;
    *) echo "--workload must be build, run, or test" >&2; exit 2 ;;
esac

case "$repetitions" in
    ''|*[!0-9]*) echo "--repetitions must be an integer of at least 1" >&2; exit 2 ;;
esac
if [ "$repetitions" -lt 1 ]; then
    echo "--repetitions must be at least 1 to prove unchanged reuse" >&2
    exit 2
fi

[ -x "$incan" ] || { echo "--incan is not executable: $incan" >&2; exit 2; }
[ -e "$source_path" ] || { echo "--source does not exist: $source_path" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "required executable is unavailable: python3" >&2; exit 2; }
command -v uname >/dev/null 2>&1 || { echo "required executable is unavailable: uname" >&2; exit 2; }
if [ -n "$cargo_guard_dir" ]; then
    [ -d "$cargo_guard_dir" ] || { echo "--cargo-guard-dir is not a directory: $cargo_guard_dir" >&2; exit 2; }
    [ -x "$cargo_guard_dir/cargo" ] || { echo "--cargo-guard-dir must contain an executable cargo guard" >&2; exit 2; }
fi

case "$max_physical_bytes:$max_domain_physical_bytes:$max_domain_logical_bytes" in
    *[!0-9:]*|::*|:*|*:) echo "storage limits must be whole-byte integers" >&2; exit 2 ;;
esac
if [ "$max_physical_bytes" -eq 0 ] || [ "$max_domain_physical_bytes" -eq 0 ] || [ "$max_domain_logical_bytes" -eq 0 ]; then
    echo "storage limits must be greater than zero" >&2
    exit 2
fi
if [ "$max_domain_physical_bytes" -gt "$max_physical_bytes" ]; then
    echo "per-domain physical policy cannot exceed aggregate policy" >&2
    exit 2
fi

store_root="$incan_home/oven/store/v1"
if [ -d "$store_root/entries" ] && find "$store_root/entries" -mindepth 1 -maxdepth 1 -type d | grep -q .; then
    echo "--incan-home must start with an empty Oven store so first materialization is attributable: $store_root" >&2
    exit 2
fi

mkdir -p "$output_dir"
phase_tsv="$output_dir/phases.tsv"
: > "$phase_tsv"

now_ms() {
    python3 -c 'import time; print(time.monotonic_ns() // 1_000_000)'
}

run_stage() {
    local stage=$1
    shift
    local started finished status
    started=$(now_ms)
    set +e
    INCAN_HOME="$incan_home" \
        INCAN_OVEN_MAX_PHYSICAL_BYTES="$max_physical_bytes" \
        INCAN_OVEN_MAX_DOMAIN_PHYSICAL_BYTES="$max_domain_physical_bytes" \
        INCAN_OVEN_MAX_DOMAIN_LOGICAL_BYTES="$max_domain_logical_bytes" \
        PATH="${cargo_guard_dir:+$cargo_guard_dir:}$PATH" \
        "$@" >"$output_dir/$stage.log" 2>&1
    status=$?
    set -e
    finished=$(now_ms)
    printf '%s\t%s\t%s\n' "$stage" "$((finished - started))" "$status" >> "$phase_tsv"
    return "$status"
}

case "$workload" in
    build) normal_args=(build "$source_path" --report json) ;;
    run) normal_args=(run "$source_path") ;;
    test) normal_args=(test --verbose "$source_path") ;;
esac

if ! run_stage first_materialization "$incan" "${normal_args[@]}"; then
    echo "the first normal command failed; the source is unsupported or the release archive is incomplete" >&2
    sed -n '1,80p' "$output_dir/first_materialization.log" >&2
    exit 1
fi
for run_index in $(seq 1 "$repetitions"); do
    if ! run_stage "warm_repeat_$run_index" "$incan" "${normal_args[@]}"; then
        echo "unchanged warm command $run_index failed" >&2
        sed -n '1,80p' "$output_dir/warm_repeat_$run_index.log" >&2
        exit 1
    fi
done
run_stage store_inspect "$incan" oven store inspect --store "$store_root" --format json

"$incan" --version >"$output_dir/incan-version.txt"
uname -a >"$output_dir/uname.txt"

python3 - "$output_dir" "$workload" "$source_path" "$store_root" "$cargo_guard_dir" \
    "$max_physical_bytes" "$max_domain_physical_bytes" "$max_domain_logical_bytes" <<'PY'
import json
import pathlib
import sys

output = pathlib.Path(sys.argv[1])
phases = []
for line in (output / "phases.tsv").read_text().splitlines():
    name, duration_ms, exit_code = line.split("\t")
    phases.append({"name": name, "duration_ms": int(duration_ms), "exit_code": int(exit_code)})

report = {
    "schema_version": 2,
    "purpose": "Oven Alpha packaged-unit first-materialization and warm normal-command evidence",
    "machine": {"uname": (output / "uname.txt").read_text().strip()},
    "toolchain": {"incan": (output / "incan-version.txt").read_text().strip()},
    "workload": {"kind": sys.argv[2], "source": sys.argv[3]},
    "cargo_guard": {"enabled": bool(sys.argv[5]), "directory": sys.argv[5] or None},
    "store": {
        "root": sys.argv[4],
        "max_physical_bytes": int(sys.argv[6]),
        "max_domain_physical_bytes": int(sys.argv[7]),
        "max_domain_logical_bytes": int(sys.argv[8]),
        "inspection": json.loads((output / "store_inspect.log").read_text()),
    },
    "phases": phases,
    "logs": {phase["name"]: f"{phase['name']}.log" for phase in phases},
}
(output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
PY

echo "Oven Alpha benchmark evidence: $output_dir/report.json"
