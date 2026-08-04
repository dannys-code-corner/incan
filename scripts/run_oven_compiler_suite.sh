#!/usr/bin/env bash

# Publish and execute the complete receipt-bound compiler suite through Oven.
#
# Cargo is permitted only in the first command, the explicit legacy publisher that translates the compiler's
# resolved unit graph into immutable direct-Rustc inputs. The runner installs a PATH-first Cargo tripwire, then
# proves that every stored root executes without Cargo. This shared script keeps the local `make test` contract and
# the hosted CI contract identical.

set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  bash scripts/run_oven_compiler_suite.sh \
    --incan PATH --compiler-root PATH --store PATH --output PATH \
    --sdk-provider-path-file PATH --sdk-provider-store PATH \
    --toolchain-data-root PATH --generated-cargo-target-dir PATH --domain NAME \
    --max-physical-bytes BYTES --max-domain-physical-bytes BYTES \
    --max-domain-logical-bytes BYTES [--cargo PATH] [--feature NAME] [--temp-root PATH]

Publishes the full compiler test suite through the named legacy Cargo publisher, then runs every stored direct-Rustc
root under a Cargo guard. The store inspection reports logical artifact bytes separately from physical disk use.
EOF
}

incan=""
compiler_root=""
cargo_override=""
store=""
output=""
sdk_provider_path_file=""
sdk_provider_store=""
toolchain_data_root=""
generated_cargo_target_dir=""
domain=""
max_physical_bytes=""
max_domain_physical_bytes=""
max_domain_logical_bytes=""
feature=""
temp_root=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --incan) incan=${2:?--incan requires a path}; shift 2 ;;
        --compiler-root) compiler_root=${2:?--compiler-root requires a path}; shift 2 ;;
        --cargo) cargo_override=${2:?--cargo requires a path}; shift 2 ;;
        --store) store=${2:?--store requires a path}; shift 2 ;;
        --output) output=${2:?--output requires a path}; shift 2 ;;
        --sdk-provider-path-file) sdk_provider_path_file=${2:?--sdk-provider-path-file requires a path}; shift 2 ;;
        --sdk-provider-store) sdk_provider_store=${2:?--sdk-provider-store requires a path}; shift 2 ;;
        --toolchain-data-root) toolchain_data_root=${2:?--toolchain-data-root requires a path}; shift 2 ;;
        --generated-cargo-target-dir) generated_cargo_target_dir=${2:?--generated-cargo-target-dir requires a path}; shift 2 ;;
        --domain) domain=${2:?--domain requires a name}; shift 2 ;;
        --max-physical-bytes) max_physical_bytes=${2:?--max-physical-bytes requires a value}; shift 2 ;;
        --max-domain-physical-bytes) max_domain_physical_bytes=${2:?--max-domain-physical-bytes requires a value}; shift 2 ;;
        --max-domain-logical-bytes) max_domain_logical_bytes=${2:?--max-domain-logical-bytes requires a value}; shift 2 ;;
        --feature) feature=${2:?--feature requires a name}; shift 2 ;;
        --temp-root) temp_root=${2:?--temp-root requires a path}; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

for required in incan compiler_root store output sdk_provider_path_file sdk_provider_store toolchain_data_root generated_cargo_target_dir domain max_physical_bytes max_domain_physical_bytes max_domain_logical_bytes; do
    if [ -z "${!required}" ]; then
        echo "--${required//_/-} is required" >&2
        usage >&2
        exit 2
    fi
done

test -x "$incan"
test -d "$compiler_root"
test -f "$sdk_provider_path_file"
test -d "$sdk_provider_store"
test -d "$toolchain_data_root"
provider_root="$(cat "$sdk_provider_path_file")"
provider_inventory="$provider_root/sdk-inventory.json"
test -f "$provider_inventory"
command -v python3 >/dev/null 2>&1 || { echo "python3 is required to record suite phase timings" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "jq is required to write compiler-suite evidence" >&2; exit 2; }

if [ -n "$cargo_override" ]; then
    cargo_path="$cargo_override"
else
    cargo_path="$(rustup which cargo)"
fi
rustc_path="$(rustup which rustc)"
test -f "$cargo_path"
test -f "$rustc_path"
mkdir -p "$store" "$output"
phase_tsv="$output/phases.tsv"
: > "$phase_tsv"

now_ms() {
    python3 -c 'import time; print(time.monotonic_ns() // 1_000_000)'
}

record_phase() {
    # The explicit publisher and the Cargo-free replay are deliberately timed separately. A successful replay is
    # the prepared-suite result; the named legacy publisher is the attributable cold/preparation cost.
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$phase_tsv"
}

if [ -z "$temp_root" ]; then
    temp_root="$output/tmp"
fi
case "$temp_root" in
    */) echo "--temp-root must not end in a slash" >&2; exit 2 ;;
esac
temp_root_name="${temp_root##*/}"
if [[ ! "$temp_root_name" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
    echo "--temp-root must end in a Rust-identifier-safe directory name, got $temp_root_name" >&2
    exit 2
fi
mkdir -p "$temp_root"

feature_args=()
if [ -n "$feature" ]; then
    feature_args=(--feature "$feature")
fi

run_named_legacy_publisher() {
    local started finished status
    started="$(now_ms)"
    set +e
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
    CARGO_NET_OFFLINE=true \
    INCAN_SOURCE_ROOT="$compiler_root" \
    INCAN_STDLIB="$compiler_root/crates/incan_stdlib/stdlib" \
    INCAN_STDLIB_DIR="$compiler_root/crates/incan_stdlib/stdlib" \
    INCAN_TOOLCHAIN_CRATES_DIR="$compiler_root/crates" \
    INCAN_SDK_INVENTORY="$provider_inventory" \
    INCAN_INTERNAL_SDK_PROVIDER_PATH_FILE="$sdk_provider_path_file" \
    INCAN_INTERNAL_SDK_PROVIDER_STORE="$sdk_provider_store" \
    INCAN_INTERNAL_TOOLCHAIN_DATA_ROOT="$toolchain_data_root" \
    INCAN_GENERATED_CARGO_TARGET_DIR="$generated_cargo_target_dir" \
    "$incan" oven legacy-cargo prepare-compiler-libtests \
        --compiler-root "$compiler_root" \
        --cargo "$cargo_path" \
        --rustc "$rustc_path" \
        "${feature_args[@]}" \
        --domain "$domain" \
        --store "$store" \
        --max-physical-bytes "$max_physical_bytes" \
        --max-domain-physical-bytes "$max_domain_physical_bytes" \
        --max-domain-logical-bytes "$max_domain_logical_bytes" \
        --format json > "$store/publish.json"
    status=$?
    set -e
    finished="$(now_ms)"
    record_phase named_legacy_publisher "$((finished - started))" "$status"
    return "$status"
}

if ! run_named_legacy_publisher; then
    echo "named legacy Cargo publisher failed; retained timing evidence at $phase_tsv" >&2
    exit 1
fi

guard="$output/cargo-guard"
mkdir -p "$guard"
cat > "$guard/cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "unexpected Cargo invocation during Oven compiler-suite execution: $*" >> "$CARGO_GUARD_LOG"
exit 97
EOF
chmod +x "$guard/cargo"
: > "$guard/invocations.log"

runner_stderr="$output/compiler-suite-runner.stderr.log"
run_cargo_free_replay() {
    local started finished status
    started="$(now_ms)"
    set +e
    PATH="$guard:$PATH" \
        CARGO_GUARD_LOG="$guard/invocations.log" \
        TMPDIR="$temp_root" \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_STDLIB="$compiler_root/crates/incan_stdlib/stdlib" \
        INCAN_STDLIB_DIR="$compiler_root/crates/incan_stdlib/stdlib" \
        INCAN_TOOLCHAIN_CRATES_DIR="$compiler_root/crates" \
        INCAN_SDK_INVENTORY="$provider_inventory" \
        INCAN_INTERNAL_SDK_PROVIDER_PATH_FILE="$sdk_provider_path_file" \
        INCAN_INTERNAL_SDK_PROVIDER_STORE="$sdk_provider_store" \
        INCAN_INTERNAL_TOOLCHAIN_DATA_ROOT="$toolchain_data_root" \
        INCAN_GENERATED_CARGO_TARGET_DIR="$generated_cargo_target_dir" \
        "$incan" oven compiler-libtests \
            --compiler-root "$compiler_root" \
            --rustc "$rustc_path" \
            "${feature_args[@]}" \
            --output "$output" \
            --store "$store" \
            --max-physical-bytes "$max_physical_bytes" \
            --max-domain-physical-bytes "$max_domain_physical_bytes" \
            --max-domain-logical-bytes "$max_domain_logical_bytes" \
            --format json > "$output/compiler-suite-report.json" 2> "$runner_stderr"
    status=$?
    set -e
    finished="$(now_ms)"
    record_phase cargo_free_direct_rustc_replay "$((finished - started))" "$status"
    return "$status"
}

if ! run_cargo_free_replay; then
    cat "$runner_stderr" >&2
    exit 1
fi

if [ -s "$guard/invocations.log" ]; then
    cat "$guard/invocations.log" >&2
    exit 1
fi

jq '{
        success,
        cargo_process_started,
        native_test_count,
        native_test_case_totals,
        native_test_root_count: (.native_test_roots | length),
        test_target_count: (.test_targets | length),
        binary_target_count: (.binary_targets | length),
        doctest_target_count: (.doctest_targets | length),
        failures
    }' "$output/compiler-suite-report.json"

run_store_inspection() {
    local started finished status
    started="$(now_ms)"
    set +e
    "$incan" oven store inspect \
        --store "$store" \
        --max-physical-bytes "$max_physical_bytes" \
        --max-domain-physical-bytes "$max_domain_physical_bytes" \
        --max-domain-logical-bytes "$max_domain_logical_bytes" \
        --format json > "$store/inspection.json"
    status=$?
    set -e
    finished="$(now_ms)"
    record_phase store_inspection "$((finished - started))" "$status"
    return "$status"
}

if ! run_store_inspection; then
    echo "Oven store inspection failed; retained timing evidence at $phase_tsv" >&2
    exit 1
fi

du -sk "$store" "$output" > "$output/disk-usage-kib.tsv"
guard_invocation_count="$(wc -l < "$guard/invocations.log" | tr -d '[:space:]')"
jq -n \
    --rawfile phases "$phase_tsv" \
    --rawfile disk_usage "$output/disk-usage-kib.tsv" \
    --slurpfile suite "$output/compiler-suite-report.json" \
    --slurpfile inspection "$store/inspection.json" \
    --arg domain "$domain" \
    --arg guard "$guard/cargo" \
    --arg store "$store" \
    --argjson guard_invocation_count "$guard_invocation_count" \
    '{
        schema_version: 1,
        purpose: "bounded Oven compiler-suite preparation and Cargo-free replay evidence",
        compatibility_domain: $domain,
        phases: [
            $phases | split("\n")[] | select(length > 0) | split("\t") |
            {name: .[0], duration_ms: (.[1] | tonumber), exit_code: (.[2] | tonumber)}
        ],
        cargo_guard: {
            executable: $guard,
            invocation_count: $guard_invocation_count,
            verified: ($guard_invocation_count == 0),
            verdict: "a successful direct-rustc replay with zero guard invocations did not launch Cargo"
        },
        suite: ($suite[0] | {
            success,
            cargo_process_started,
            native_test_count,
            native_test_case_totals,
            native_test_root_count: (.native_test_roots | length),
            doctest_target_count: (.doctest_targets | length),
            test_target_count: (.test_targets | length),
            binary_target_count: (.binary_targets | length),
            shard_count,
            failures
        }),
        store: ($inspection[0] | {
            logical_bytes,
            physical_bytes,
            reclaimable_physical_bytes,
            active_lease_physical_bytes,
            limits,
            entry_count: (.entries | length),
            domains: ([.entries[].manifest.domain] | unique)
        }),
        raw_disk_usage_kib: [
            $disk_usage | split("\n")[] | select(length > 0) |
            capture("^(?<kib>[0-9]+)[[:space:]]+(?<path>.*)$") |
            {path, kib: (.kib | tonumber), bytes: ((.kib | tonumber) * 1024)}
        ],
        reports: {
            publisher: ($store + "/publish.json"),
            suite: "compiler-suite-report.json",
            store_inspection: ($store + "/inspection.json"),
            phase_tsv: "phases.tsv"
        }
    }' > "$output/suite-evidence.json"

jq '{phases, cargo_guard, suite, store, raw_disk_usage_kib}' "$output/suite-evidence.json"
jq '{logical_bytes, physical_bytes, reclaimable_physical_bytes, active_lease_physical_bytes, limits, entry_count: (.entries | length), domains: ([.entries[].manifest.domain] | unique)}' "$store/inspection.json"
du -sh "$store" "$output"
