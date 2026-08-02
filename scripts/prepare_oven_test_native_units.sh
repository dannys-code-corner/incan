#!/usr/bin/env bash

# Stage debug and release foundation native units that let the Rust integration suite exercise normal Oven build/run/test commands.
#
# This is test-fixture preparation, not a normal command fallback: each probe intentionally records its receipt on an
# initial Oven-native miss, then this script invokes the hidden publisher only here. The foundation covers
# the compiler-owned standard-library envelope exercised by normal build/run/test. Its provider superset covers
# narrower compiler-owned requests without treating dependency resolution as a fallback. A corresponding release
# seed is required because normal `incan build --lib` deliberately produces both debug and release caller-owned
# libraries. The guards below prove those normal envelopes use the installed seeds without Cargo.

set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  bash scripts/prepare_oven_test_native_units.sh \
    --incan PATH --output PATH [--max-bytes BYTES]

Builds broad compiler-owned standard-library and encoding debug/release foundations, then verifies normal core,
testing, hashing, utility, encoding, and library Alpha envelopes. The output must be under a compiler toolchain layout (the
regular test target uses target/share/incan/oven/native-units). `--max-bytes` is the aggregate physical policy;
each native-unit domain is independently limited to 768 MiB logical bytes and 1 GiB physical bytes.
EOF
}

incan=""
output=""
max_bytes=2147483648
domain_max_logical_bytes=805306368
domain_max_physical_bytes=1073741824
compiler_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --incan) incan=${2:?--incan requires a path}; shift 2 ;;
        --output) output=${2:?--output requires a path}; shift 2 ;;
        --max-bytes) max_bytes=${2:?--max-bytes requires a value}; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[ -n "$incan" ] || { echo "--incan is required" >&2; exit 2; }
[ -n "$output" ] || { echo "--output is required" >&2; exit 2; }
[ -x "$incan" ] || { echo "--incan is not executable: $incan" >&2; exit 2; }
[ -n "${INCAN_SDK_INVENTORY:-}" ] \
    || { echo "INCAN_SDK_INVENTORY must name the SDK inventory prepared for Oven native-unit seeds" >&2; exit 2; }
[ -f "$INCAN_SDK_INVENTORY" ] \
    || { echo "INCAN_SDK_INVENTORY is not a regular file: $INCAN_SDK_INVENTORY" >&2; exit 2; }
case "$max_bytes" in
    ''|*[!0-9]*) echo "--max-bytes must be a positive whole-byte value" >&2; exit 2 ;;
esac
[ "$max_bytes" -gt 0 ] || { echo "--max-bytes must be greater than zero" >&2; exit 2; }

cargo_bin="$(command -v cargo)" || { echo "cargo is required for the explicit test-fixture publisher" >&2; exit 2; }
if [ -n "${RUSTC:-}" ]; then
    rustc_bin="$RUSTC"
elif command -v rustup >/dev/null 2>&1; then
    rustc_bin="$(rustup which rustc)"
else
    rustc_bin="$(command -v rustc)"
fi
[ -x "$rustc_bin" ] || { echo "rustc is not executable: $rustc_bin" >&2; exit 2; }

output_parent="$(dirname "$output")"
mkdir -p "$output_parent"
output_parent="$(cd "$output_parent" && pwd -P)"
output="$output_parent/$(basename "$output")"
visible_output="$compiler_root/target/share/incan/oven/native-units"
[ "$output" = "$visible_output" ] || {
    echo "--output must be the compiler-visible native-unit layout $visible_output, got $output" >&2
    exit 2
}
scratch="$(mktemp -d "${TMPDIR:-/tmp}/incan-oven-test-seeds.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

# Test preparation owns this exact ignored fixture directory. A stale unit must never be reused across a rebuilt
# compiler because receipt build-unit identity includes the active runtime sources and lockfile.
rm -rf "$output"
mkdir -p "$output"

prepare_seed() {
    local name=$1
    local profile=$2
    local source="$scratch/oven_${name}_seed.incn"
    local receipt="$scratch/.incan/oven/receipt.json"
    local generated_project="$scratch/target/incan/oven_${name}_seed"
    local oven_home="$scratch/oven-${name}-home"
    local log="$scratch/oven_${name}_seed.probe.log"

    case "$name" in
        testing|core)
            # This is the complete compiler-owned module envelope currently exercised by the normal CLI integration
            # suite. It deliberately imports one checked symbol per module so the receipt records an explicit
            # provider capability. The compiler suite also directly exercises the exact registry dependency
            # `bitflags = "=1.3.2"`; it is compiled here only by the named publisher into the immutable seed catalog.
            # This is a bounded test compatibility envelope, not a normal-command Cargo or cache fallback.
            printf '%s\n' \
                'from std.async import spawn' \
                'from std.async.channel import Sender' \
                'from std.async.race import RaceArm' \
                'from std.async.sync import Mutex' \
                'from std.async.task import spawn_blocking' \
                'from std.async.time import Duration' \
                'from std.collections import OrdinalKey' \
                'from std.derives.collection import FallibleIterator' \
                'from std.environ import get' \
                'from std.fs import Path' \
                'from std.fs.glob import matches' \
                'from std.fs.locking import FileLock' \
                'from std.fs.path import _as_path' \
                'from std.io import BytesIO' \
                'from std.hash import sha1' \
                'from std.json import JsonValue' \
                'from std.registry import Registry' \
                'from std.serde import json' \
                'from std.serde.json import Serialize' \
                'from std.testing import assert_true' \
                'from std.traits import TryFrom' \
                'from std.traits.callable import Callable1' \
                'from std.traits.convert import Into' \
                'from std.traits.indexing import Index' \
                'from std.web import App, Response' \
                'from std.web.request import Query' \
                'from std.web.routing import GET, route' \
                'import std.async' \
                'from rust::bitflags import bitflags' \
                '' \
                '@route("/oven-seed", methods=[GET])' \
                'async def oven_seed_route() -> Response:' \
                '    return Response.ok()' \
                '' \
                'def main() -> None:' \
                '    pass' > "$source"
            printf '%s\n' \
                '[project]' \
                "name = \"oven_${name}_seed\"" \
                'version = "0.1.0"' \
                '' \
                '[rust-dependencies]' \
                'bitflags = "=1.3.2"' \
                > "$scratch/incan.toml"
            ;;
        encoding)
            # Encoding is a separately bounded family rather than an oversized extension of the broad foundation.
            # Its source imports every public encoding module plus the standard-library modules used by its checked
            # implementation. The native-unit selector chooses the narrowest authorized family for a request.
            printf '%s\n' \
                'from std.encoding._shared import EncodingError' \
                'from std.encoding.base32 import b32encode' \
                'from std.encoding.base58 import b58encode' \
                'from std.encoding.base64 import b64encode' \
                'from std.encoding.base85 import b85encode' \
                'from std.encoding.bech32 import bech32_encode' \
                'from std.encoding.hex import encode as hex_encode' \
                'from std.fs import Path' \
                'from std.io import BytesIO' \
                'from std.traits.error import Error' \
                '' \
                'def main() -> None:' \
                '    pass' > "$source"
            printf '%s\n' \
                '[project]' \
                "name = \"oven_${name}_seed\"" \
                'version = "0.1.0"' > "$scratch/incan.toml"
            ;;
        *)
            echo "unknown Oven test native-unit seed: $name" >&2
            exit 2
            ;;
    esac

    rm -rf "$scratch/.incan" "$scratch/target" "$oven_home"
    # The ordinary Rust integration children use the compiler checkout that owns this fixture. Never inherit an
    # unrelated developer source root: runtime source digests are part of native-unit compatibility, so that would
    # publish seeds the stored suite can never select. The explicit publisher still removes ambient stdlib overrides
    # so runtime lookup follows the checked compiler layout rather than a caller-selected stdlib directory.
    local -a run_args=(run "$source")
    if [ "$profile" = "release" ]; then
        run_args=(run --release "$source")
    fi
    if env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_OVEN_NATIVE_UNIT_SEED=1 INCAN_HOME="$oven_home" \
        "$incan" "${run_args[@]}" >"$log" 2>&1; then
        echo "expected initial Oven-native miss while preparing $name" >&2
        exit 1
    fi
    grep -F 'Oven Alpha has no compatible native' "$log" >/dev/null \
        || { sed -n '1,80p' "$log" >&2; echo "probe failed before its expected Oven-native miss" >&2; exit 1; }
    [ -f "$receipt" ] || { echo "probe did not create receipt: $receipt" >&2; exit 1; }
    [ -d "$generated_project" ] || { echo "probe did not create generated project: $generated_project" >&2; exit 1; }

    "$incan" oven legacy-cargo prepare-native-unit-seed \
        --output "$output" \
        --receipt "$receipt" \
        --generated-project "$generated_project" \
        --cargo "$cargo_bin" \
        --rustc "$rustc_bin" \
        --format json >/dev/null
}

verify_core_consumer() {
    local source="$scratch/oven_core_consumer.incn"
    local oven_home="$scratch/core-consumer-home"
    local cargo_guard="$scratch/core-cargo-guard"

    printf 'def main() -> None:\n    pass\n' > "$source"
    mkdir -p "$cargo_guard"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" run "$source" >/dev/null
}

verify_std_testing_consumer() {
    local source="$scratch/test_oven_testing_consumer.incn"
    local oven_home="$scratch/testing-consumer-home"
    local cargo_guard="$scratch/cargo-guard"

    printf 'from std.testing import assert_true\n\ndef test_oven_seed_reuses_testing_unit() -> None:\n    assert_true(True)\n' > "$source"
    mkdir -p "$cargo_guard"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" test "$source" >/dev/null
}

verify_std_hash_test_consumer() {
    local source="$scratch/test_oven_hash_consumer.incn"
    local oven_home="$scratch/hash-consumer-home"
    local cargo_guard="$scratch/hash-cargo-guard"

    printf 'from std.hash import sha1\n\ndef test_oven_seed_reuses_hash_unit() -> None:\n    assert len(sha1.digest(b"abc")) == 20\n' > "$source"
    mkdir -p "$cargo_guard"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" test "$source" >/dev/null
}

verify_web_route_consumer() {
    local source="$scratch/oven_web_route_consumer.incn"
    local oven_home="$scratch/web-route-consumer-home"
    local cargo_guard="$scratch/web-route-cargo-guard"

    printf 'from std.web import GET, Response, route\nimport std.async\n\n@route("/oven-consumer", methods=[GET])\nasync def oven_consumer_route() -> Response:\n    return Response.ok()\n\ndef main() -> None:\n    pass\n' > "$source"
    mkdir -p "$cargo_guard"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" build "$source" >/dev/null
}

verify_utility_consumer() {
    local source="$scratch/oven_utility_consumer.incn"
    local oven_home="$scratch/utility-consumer-home"
    local cargo_guard="$scratch/utility-cargo-guard"

    printf 'from std.fs import Path\nfrom std.json import JsonValue\n\ndef main() -> None:\n    _ = Path(".").exists()\n' > "$source"
    mkdir -p "$cargo_guard"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" run "$source" >/dev/null
}

verify_encoding_consumer() {
    local source="$scratch/oven_encoding_consumer.incn"
    local oven_home="$scratch/encoding-consumer-home"
    local cargo_guard="$scratch/encoding-cargo-guard"

    printf 'from std.encoding._shared import EncodingError\nfrom std.encoding.base32 import b32encode\nfrom std.encoding.base58 import b58encode\nfrom std.encoding.base64 import b64encode\nfrom std.encoding.base85 import b85encode\nfrom std.encoding.bech32 import bech32_encode\nfrom std.encoding.hex import encode as hex_encode\n\ndef main() -> None:\n    assert b64encode(b"hello") == "aGVsbG8="\n' > "$source"
    mkdir -p "$cargo_guard"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" run "$source" >/dev/null
}

verify_library_consumer() {
    local project="$scratch/oven_library_consumer"
    local oven_home="$scratch/library-consumer-home"
    local cargo_guard="$scratch/library-cargo-guard"

    mkdir -p "$project/src" "$cargo_guard"
    printf '[project]\nname = "oven_library_consumer"\nversion = "0.1.0"\n' > "$project/incan.toml"
    printf 'pub def value() -> int:\n    return 1\n' > "$project/src/lib.incn"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" build --lib "$project" >/dev/null
}

verify_registry_leaf_consumer() {
    local project="$scratch/oven_registry_leaf_consumer"
    local oven_home="$scratch/registry-leaf-consumer-home"
    local cargo_guard="$scratch/registry-leaf-cargo-guard"

    mkdir -p "$project/src" "$cargo_guard"
    printf '%s\n' \
        '[project]' \
        'name = "oven_registry_leaf_consumer"' \
        'version = "0.1.0"' \
        '' \
        '[project.scripts]' \
        'library = "src/lib.incn"' \
        '' \
        '[rust-dependencies]' \
        'bitflags = "=1.3.2"' > "$project/incan.toml"
    printf 'from rust::bitflags import bitflags\n\npub def value() -> int:\n    return 1\n' > "$project/src/lib.incn"
    printf '#!/bin/sh\nexit 97\n' > "$cargo_guard/cargo"
    chmod +x "$cargo_guard/cargo"
    env -u INCAN_STDLIB -u INCAN_STDLIB_DIR \
        INCAN_SOURCE_ROOT="$compiler_root" \
        INCAN_HOME="$oven_home" PATH="$cargo_guard:$PATH" \
        "$incan" build --lib "$project" >/dev/null
}

prepare_seed testing debug
prepare_seed core release
prepare_seed encoding debug
prepare_seed encoding release
verify_core_consumer
verify_std_testing_consumer
verify_std_hash_test_consumer
verify_web_route_consumer
verify_utility_consumer
verify_encoding_consumer
verify_library_consumer
verify_registry_leaf_consumer

seed_count="$(find "$output" -name seed.json -type f | wc -l | tr -d ' ')"
[ "$seed_count" = 4 ] || { echo "expected debug and release broad and encoding Oven foundation seeds, found $seed_count" >&2; exit 1; }
aggregate_physical_bytes=0
aggregate_logical_bytes=0
while IFS= read -r seed; do
    seed_root="$(dirname "$seed")"
    logical_bytes="$(find "$seed_root" -type f -exec wc -c {} + | awk '{ total += $1 } END { print total + 0 }')"
    physical_bytes="$(du -sk "$seed_root" | awk '{ print $1 * 1024 }')"
    [ "$logical_bytes" -le "$domain_max_logical_bytes" ] \
        || { echo "Oven native-unit domain $seed_root has $logical_bytes logical bytes; policy is $domain_max_logical_bytes" >&2; exit 1; }
    [ "$physical_bytes" -le "$domain_max_physical_bytes" ] \
        || { echo "Oven native-unit domain $seed_root has $physical_bytes physical bytes; policy is $domain_max_physical_bytes" >&2; exit 1; }
    aggregate_logical_bytes=$((aggregate_logical_bytes + logical_bytes))
    aggregate_physical_bytes=$((aggregate_physical_bytes + physical_bytes))
done < <(find "$output" -name seed.json -type f | sort)
[ "$aggregate_physical_bytes" -le "$max_bytes" ] \
    || { echo "Oven native-unit aggregate has $aggregate_physical_bytes physical bytes; policy is $max_bytes" >&2; exit 1; }
printf 'Prepared %s Oven foundation seeds (%s logical bytes, %s physical bytes, aggregate physical policy %s bytes)\n' \
    "$seed_count" "$aggregate_logical_bytes" "$aggregate_physical_bytes" "$max_bytes"
