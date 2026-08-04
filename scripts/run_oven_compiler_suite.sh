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
    --max-domain-logical-bytes BYTES [--feature NAME] [--temp-root PATH]

Publishes the full compiler test suite through the named legacy Cargo publisher, then runs every stored direct-Rustc
root under a Cargo guard. The store inspection reports logical artifact bytes separately from physical disk use.
EOF
}

incan=""
compiler_root=""
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

cargo_path="$(rustup which cargo)"
rustc_path="$(rustup which rustc)"
test -f "$cargo_path"
test -f "$rustc_path"
mkdir -p "$store" "$output"
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
if ! PATH="$guard:$PATH" \
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
        --format json > "$output/compiler-suite-report.json" 2> "$runner_stderr"; then
    cat "$runner_stderr" >&2
    exit 1
fi

if [ -s "$guard/invocations.log" ]; then
    cat "$guard/invocations.log" >&2
    exit 1
fi

jq '{success, cargo_process_started, passed, failed, green_roots, reported_roots, unreported_roots, failures}' \
    "$output/compiler-suite-report.json"

"$incan" oven store inspect \
    --store "$store" \
    --max-physical-bytes "$max_physical_bytes" \
    --max-domain-physical-bytes "$max_domain_physical_bytes" \
    --max-domain-logical-bytes "$max_domain_logical_bytes" \
    --format json > "$store/inspection.json"
jq '{logical_bytes, physical_bytes, reclaimable_physical_bytes, active_lease_physical_bytes, limits, entry_count: (.entries | length), domains: ([.entries[].manifest.domain] | unique)}' "$store/inspection.json"
du -sh "$store" "$output"
