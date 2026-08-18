#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Package the Incan toolchain commands for one host target.

Usage:
  package_archive.sh <target> [--out-dir <dir>]

Environment:
  INCAN_BIN      Path to the built incan binary (default: target/release/incan)
  INCAN_LSP_BIN  Path to the built incan-lsp binary (default: target/release/incan-lsp)
  INCAN_SDK_PROVIDER_BUILDER_BIN
                 Host incan binary used to prepare the platform-neutral SDK provider seed (default: INCAN_BIN)
  INCAN_SDK_PROVIDER_SEED_DIR
                 Prebuilt SDK provider seed override used by packaging tests and controlled release staging
  INCAN_OVEN_LOAF_DIR
                 Prebuilt compiler-owned Oven Loafs used by packaging tests and controlled staging
  INCAN_SDK_DISTRIBUTION_PROFILE
                 SDK profile whose component payloads are packaged (default: full)
  TOOLCHAIN_RELEASE    Release name override (default: tag name or v<workspace version>)
USAGE
}

fail() {
  printf 'package_archive: %s\n' "$*" >&2
  exit 1
}

# Clear ambient Cargo/rustc-wrapper state before an internal Cargo invocation, mirroring
# `clear_inherited_cargo_environment` in src/oven/rustc.rs. This script's own `cargo metadata`
# calls are the release support workspace's authority; they must not inherit CARGO_* state
# (target dir, build jobs, an rustc wrapper meant for a different build, etc.) from an
# already-running, possibly nested Cargo/toolchain-managed parent process such as `cargo test`.
# Unlike the Rust helper, this deliberately keeps `CARGO_HOME` -- callers (CI, local testing) rely
# on it to name the prewarmed registry cache, and this script has no explicit replacement value to
# re-inject the way Rust call sites do immediately after clearing.
clear_inherited_cargo_environment() {
  local name
  for name in CARGO "${!CARGO_@}" RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER; do
    [ "$name" = "CARGO_HOME" ] && continue
    unset "$name"
  done
}

# Re-run one failed Cargo invocation under `strace`, filtered to syscalls that could plausibly
# explain an unusual, non-Cargo-typical exit status (network/socket address-family errors,
# process/fork failures) with no diagnostic text on stderr. Best-effort only: installs a local
# strace copy on demand when the host doesn't already have one, and silently produces no output
# when that isn't possible (no network, no package manager, unsupported platform) rather than
# masking the original failure. Prints at most a bounded tail so the caller's error stays legible.
describe_failure_via_strace() {
  if ! command -v strace >/dev/null 2>&1; then
    if command -v apt-get >/dev/null 2>&1; then
      apt-get install -y --no-install-recommends strace >/tmp/package_archive_strace_install.log 2>&1 || true
    fi
  fi
  if ! command -v strace >/dev/null 2>&1; then
    printf '<strace unavailable for follow-up diagnosis>'
    return 0
  fi
  local trace_log
  trace_log="$(mktemp)"
  ( clear_inherited_cargo_environment; strace -f -yy -tt -e trace=network,process -o "$trace_log" "$@" >/dev/null 2>&1 ) || true
  tail -c 6000 "$trace_log" 2>/dev/null
  rm -f "$trace_log"
}

if [ "$#" -lt 1 ]; then
  usage >&2
  exit 2
fi

target="$1"
shift
out_dir="."

[ -n "$target" ] || fail "target must not be empty"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --out-dir)
      [ "$#" -ge 2 ] || fail "--out-dir requires a value"
      out_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown option: $1"
      ;;
  esac
done

workspace_version() {
  awk '
    /^\[workspace.package\]/ { in_section=1; next }
    /^\[/ { in_section=0 }
    in_section && /^version = / {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
}

version="$(workspace_version)"
[ -n "$version" ] || fail "could not read workspace package version from Cargo.toml"

if [ -n "${TOOLCHAIN_RELEASE:-}" ]; then
  release="$TOOLCHAIN_RELEASE"
elif [[ "${GITHUB_REF:-}" == refs/tags/* ]]; then
  release="${GITHUB_REF_NAME}"
else
  release="v${version}"
fi

incan_bin="${INCAN_BIN:-target/release/incan}"
incan_lsp_bin="${INCAN_LSP_BIN:-target/release/incan-lsp}"
stdlib_dir="${INCAN_STDLIB_SOURCE_DIR:-crates/incan_stdlib/stdlib}"
distribution_profile="${INCAN_SDK_DISTRIBUTION_PROFILE:-full}"
[ -x "$incan_bin" ] || fail "incan binary is not executable: $incan_bin"
[ -x "$incan_lsp_bin" ] || fail "incan-lsp binary is not executable: $incan_lsp_bin"
[ -d "$stdlib_dir" ] || fail "stdlib source directory does not exist: $stdlib_dir"
[ -f "$stdlib_dir/testing.incn" ] || fail "stdlib source directory is missing testing.incn: $stdlib_dir"
for support_crate in incan_core incan_derive incan_stdlib incan_vocab incan_web_macros; do
  [ -f "crates/${support_crate}/Cargo.toml" ] || fail "support crate is missing: crates/${support_crate}"
done

archive_counter=0
stage_tracked_tree() {
  local source_tree="${1#./}"
  local destination="$2"
  archive_counter=$((archive_counter + 1))
  local source_archive="$package_dir/.tracked-source-${archive_counter}.tar"
  mkdir -p "$destination"
  git archive --format=tar --output="$source_archive" "HEAD:${source_tree}" \
    || fail "could not archive tracked source tree: ${source_tree}"
  tar -C "$destination" -xf "$source_archive" \
    || fail "could not extract tracked source tree into: ${destination}"
  rm "$source_archive"
}

validate_sdk_provider_seed() {
  local seed_dir="$1"
  [ -d "$seed_dir" ] || fail "SDK provider seed directory does not exist: $seed_dir"
  [ -f "$seed_dir/sdk-inventory.json" ] || fail "SDK provider seed is missing sdk-inventory.json: $seed_dir"
  [ -f "$seed_dir/Cargo.lock" ] || fail "SDK provider seed is missing its shared Cargo.lock: $seed_dir"
  [ ! -d "$seed_dir/.cargo-target" ] || fail "SDK provider seed contains a Cargo build target: $seed_dir"
  local required_components excluded_components
  case "$distribution_profile" in
    minimal)
      required_components="stdlib-core"
      excluded_components="stdlib-system stdlib-codecs stdlib-compression stdlib-data stdlib-async stdlib-observability stdlib-web stdlib-testing"
      ;;
    default|full)
      required_components="stdlib-core stdlib-system stdlib-codecs stdlib-compression stdlib-data stdlib-async stdlib-observability stdlib-web stdlib-testing"
      excluded_components=""
      ;;
    *)
      fail "unsupported SDK distribution profile: $distribution_profile"
      ;;
  esac
  local component component_dir manifest_count
  for component in $required_components
  do
    component_dir="$seed_dir/components/$component"
    [ -d "$component_dir" ] || fail "SDK provider seed is missing component $component"
    [ -f "$component_dir/Cargo.toml" ] || fail "SDK component $component is missing Cargo.toml"
    [ ! -f "$component_dir/Cargo.lock" ] || fail "SDK component $component duplicates the shared Cargo.lock"
    [ -f "$component_dir/src/lib.rs" ] || fail "SDK component $component is missing src/lib.rs"
    manifest_count="$(find "$component_dir" -maxdepth 1 -type f -name '*.incnlib' | wc -l | tr -d ' ')"
    [ "$manifest_count" = "1" ] || fail "SDK component $component must contain exactly one .incnlib manifest"
  done
  for component in $excluded_components; do
    [ ! -e "$seed_dir/components/$component" ] \
      || fail "SDK distribution profile $distribution_profile unexpectedly contains component $component"
  done
  if grep -R -E '(/Users/|/home/|/private/tmp/|/tmp/)' \
    "$seed_dir/sdk-inventory.json" "$seed_dir/components"/*/Cargo.toml >/dev/null 2>&1
  then
    fail "SDK provider seed contains a producer-specific absolute path"
  fi
}

prepare_sdk_provider_seed() {
  if [ -n "${INCAN_SDK_PROVIDER_SEED_DIR:-}" ]; then
    printf '%s\n' "$INCAN_SDK_PROVIDER_SEED_DIR"
    return
  fi

  local provider_builder="${INCAN_SDK_PROVIDER_BUILDER_BIN:-$incan_bin}"
  [ -x "$provider_builder" ] || fail "SDK provider builder is not executable: $provider_builder"
  provider_builder="$(cd "$(dirname "$provider_builder")" && pwd -P)/$(basename "$provider_builder")"
  local staged_stdlib="$package_dir/crates/incan_stdlib/stdlib"
  local probe="$package_dir/.incan-sdk-provider-seed-${target}-$$.incn"
  local path_file="$package_dir/.incan-sdk-provider-seed-${target}-$$.path"
  printf 'from std.result import map\n\ndef main() -> None:\n    pass\n' > "$probe"
  rm -f "$path_file"
  if (
    cd "$package_dir"
    INCAN_STDLIB="$staged_stdlib" \
      INCAN_TOOLCHAIN_CRATES_DIR="$package_dir/crates" \
      INCAN_INTERNAL_SDK_PROVIDER_STORE="$release_provider_store" \
      INCAN_INTERNAL_SDK_PROVIDER_PATH_FILE="$path_file" \
      INCAN_INTERNAL_SDK_DISTRIBUTION_PROFILE="$distribution_profile" \
      "$provider_builder" check "$probe" --sdk-profile "$distribution_profile" >/dev/null
  ); then
    :
  else
    rm -f "$probe" "$path_file"
    fail "could not prepare the release-compatible SDK provider seed"
  fi
  [ -s "$path_file" ] || fail "SDK provider builder did not report its seed path"
  local prepared_seed
  prepared_seed="$(sed -n '1p' "$path_file")"
  rm -f "$probe" "$path_file"
  printf '%s\n' "$prepared_seed"
}

mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd -P)"
package_dir="$out_dir/dist/incan-${release}-${target}"
archive="$out_dir/incan-${release}-${target}.tar.gz"
release_provider_store=""

cleanup_release_provider_store() {
  if [ -n "$release_provider_store" ]; then
    rm -rf "$release_provider_store"
  fi
}
trap cleanup_release_provider_store EXIT

rm -rf "$package_dir"
mkdir -p "$package_dir/bin" "$package_dir/crates"
cp "$incan_bin" "$package_dir/bin/incan"
cp "$incan_lsp_bin" "$package_dir/bin/incan-lsp"
for support_crate in incan_core incan_derive incan_stdlib incan_vocab incan_web_macros; do
  support_destination="$package_dir/crates/${support_crate}"
  stage_tracked_tree "crates/${support_crate}" "$support_destination"
done
if [ -z "${INCAN_SDK_PROVIDER_SEED_DIR:-}" ]; then
  release_provider_store="$package_dir/share/incan"
fi
cat > "$package_dir/crates/Cargo.toml" <<WORKSPACE
[workspace]
members = [
    "incan_core",
    "incan_derive",
    "incan_stdlib",
    "incan_vocab",
    "incan_web_macros",
]
default-members = [
    "incan_core",
    "incan_derive",
    "incan_stdlib",
    "incan_vocab",
    "incan_web_macros",
]
resolver = "2"

[workspace.package]
version = "${version}"
edition = "2024"
rust-version = "1.93"
license = "Apache-2.0"
authors = ["Danny Meijer <dannys.code.corner@gmail.com>"]
repository = "https://github.com/encero-systems/incan"
homepage = "https://github.com/encero-systems/incan"
keywords = ["programming-language", "compiler", "rust", "python"]
categories = ["compilers", "development-tools"]

# This non-default package keeps the locked registry-source authority used by the built-in release Loaf fixtures. It
# is metadata-only release infrastructure; normal support-workspace commands operate on the default members above.
[package]
name = "incan-release-inspection-authority"
version = "${version}"
edition = "2024"
publish = false

[lib]
path = "release-inspection-authority.rs"
WORKSPACE
printf '%s\n' '#![allow(dead_code)]' > "$package_dir/crates/release-inspection-authority.rs"
git show HEAD:src/oven/fixtures/release_stdlib.toml \
  | awk '
      /^\[rust-dependencies\]$/ {
        print "[dependencies]"
        copy_dependencies = 1
        next
      }
      copy_dependencies { print }
    ' >> "$package_dir/crates/Cargo.toml" \
  || fail "could not stage the checked release Loaf inspection dependency authority"
git show HEAD:Cargo.lock > "$package_dir/crates/Cargo.lock" \
  || fail "could not stage the verified workspace Cargo.lock"
# Oven's compiler-suite test roots deliberately poison `cargo` on `PATH` with a guard binary that
# rejects any unexpected invocation, to catch tests that should never touch Cargo; this script is
# a legitimate exception to that guard (its caller, tests/toolchain_installer_tests.rs, is the one
# test that genuinely needs to package a real release archive). Confirmed by direct CI diagnosis:
# the guard sandbox clears `CARGO_HOME` and redirects `HOME` to an isolated, per-root scratch
# directory, so neither can locate the real Cargo; the real Cargo is still on `PATH` (rustup's
# normal install location), just shadowed because the guard's own directory -- always somewhere
# under this repository's own `target/` tree -- is prepended in front of it. Resolve Cargo in a
# way that specific trick cannot intercept, preferring the most explicit source available:
#   1. `CARGO_BIN`, when the caller names a verified real Cargo directly.
#   2. The first `cargo` on `PATH` whose directory is NOT inside this repository's own `target/`
#      tree. A real, system-installed Cargo is never legitimately located there; only a guard or
#      other build-owned artifact would be.
#   3. `command -v cargo` outright, unchanged for every caller with no such guard (a real release
#      build, local manual packaging) and as a last-resort fallback otherwise.
if [ -n "${CARGO_BIN:-}" ]; then
  cargo_bin="$CARGO_BIN"
  [ -x "$cargo_bin" ] || fail "CARGO_BIN does not name an executable: $cargo_bin"
else
  cargo_bin=""
  saved_ifs="$IFS"
  IFS=':'
  for path_entry in $PATH; do
    case "$path_entry" in
      */target/*) continue ;;
    esac
    if [ -x "$path_entry/cargo" ]; then
      cargo_bin="$path_entry/cargo"
      break
    fi
  done
  IFS="$saved_ifs"
  if [ -z "$cargo_bin" ]; then
    cargo_bin="$(command -v cargo)" || fail "could not resolve Cargo for the release support workspace"
  fi
fi
# The resolved binary's own directory is the real Cargo home (`.cargo/bin/cargo`, whether reached
# via `CARGO_BIN` or the `PATH` walk above), and `.cargo`/`.rustup` are always installed as
# siblings under the same parent directory -- regardless of what `$HOME` is later set to at
# runtime. Capture that real Cargo home now, before `$HOME` gets in the way of anything else that
# needs it (the offline registry cache below).
cargo_home_dir="$(dirname "$(dirname "$cargo_bin")")"
# The resolved binary is very likely rustup's own multiplexer (a `cargo` symlink or shim next to
# `rustup` itself), which selects a toolchain at runtime by consulting `$RUSTUP_HOME` (default
# `$HOME/.rustup`) for a configured default. That lookup fails here even after finding the right
# Cargo: the guard's sandbox also redirects `HOME` to an isolated, per-root scratch directory with
# no rustup state at all. Route around rustup's own toolchain selection entirely by resolving
# directly to one real, installed toolchain's `cargo`.
#
# Prefer `$RUSTUP_HOME`/`$HOME/.rustup` first: that is rustup's own authoritative toolchains
# location regardless of where its `cargo`/`rustup` shim binary physically lives, so it also covers
# package-manager rustup installs (e.g. Homebrew's `rustup` formula, whose shims live under
# `<prefix>/opt/rustup/bin/` rather than `~/.cargo/bin/`) where `.cargo`/`.rustup` are not siblings
# of the resolved binary's directory. Fall back to the sibling-of-Cargo heuristic only when that
# lookup is empty, which covers the guarded/sandboxed case above where `$HOME` itself is redirected
# but the resolved Cargo binary's real location still has `.rustup` as a physical sibling.
direct_toolchain_cargo="$(
  find "${RUSTUP_HOME:-$HOME/.rustup}/toolchains" -mindepth 3 -maxdepth 3 \
    -type f -name cargo -path '*/bin/cargo' 2>/dev/null | head -1
)"
if [ -z "$direct_toolchain_cargo" ]; then
  direct_toolchain_cargo="$(
    find "$(dirname "$cargo_home_dir")/.rustup/toolchains" -mindepth 3 -maxdepth 3 \
      -type f -name cargo -path '*/bin/cargo' 2>/dev/null | head -1
  )"
fi
if [ -n "$direct_toolchain_cargo" ] && [ -x "$direct_toolchain_cargo" ]; then
  cargo_bin="$direct_toolchain_cargo"
fi
# `cargo metadata --offline` below resolves its registry cache from `$CARGO_HOME` (default
# `$HOME/.cargo`), which is equally a victim of the guard's `$HOME` redirect: the offline cache
# prewarmed into the real Cargo home would otherwise be invisible. `clear_inherited_cargo_environment`
# deliberately preserves `CARGO_HOME`, so exporting the real value here, once, is sufficient for
# every Cargo invocation this script makes afterward.
: "${CARGO_HOME:=$cargo_home_dir}"
export CARGO_HOME
printf 'package_archive: DEBUG cargo resolution: CARGO_BIN=%s CARGO_HOME=%s HOME=%s resolved=%s PATH=%s\n' \
  "${CARGO_BIN:-<unset>}" "${CARGO_HOME:-<unset>}" "${HOME:-<unset>}" "$cargo_bin" "$PATH" >&2
# The archive ships a deliberately reduced support workspace, so its lock must describe that workspace rather than the
# complete compiler repository. Seed resolution from the verified repository lock, reconcile only the removed workspace
# members without network access, then prove the shipped closure is stable under Cargo's locked mode.
#
# Each call runs in its own `( ... )` subshell so `clear_inherited_cargo_environment` only scopes that one Cargo
# invocation; the SDK component build later in this script still needs its own ambient Cargo/rustc-wrapper state.
set +e
metadata_error="$(
  clear_inherited_cargo_environment
  "$cargo_bin" metadata \
    --offline \
    --format-version 1 \
    --manifest-path "$package_dir/crates/Cargo.toml" 2>&1 >/dev/null
)"
metadata_exit=$?
set -e
if [ "$metadata_exit" -ne 0 ]; then
  strace_summary="$(describe_failure_via_strace "$cargo_bin" metadata --offline --format-version 1 --manifest-path "$package_dir/crates/Cargo.toml")"
  fail "could not derive the release support workspace lock from the verified repository lock (exit ${metadata_exit}): ${metadata_error}
strace (network/process syscalls, tail): ${strace_summary}"
fi
set +e
metadata_error="$(
  clear_inherited_cargo_environment
  "$cargo_bin" metadata \
    --locked \
    --offline \
    --format-version 1 \
    --manifest-path "$package_dir/crates/Cargo.toml" 2>&1 >/dev/null
)"
metadata_exit=$?
set -e
if [ "$metadata_exit" -ne 0 ]; then
  strace_summary="$(describe_failure_via_strace "$cargo_bin" metadata --locked --offline --format-version 1 --manifest-path "$package_dir/crates/Cargo.toml")"
  fail "release support workspace lock is not reproducible (exit ${metadata_exit}): ${metadata_error}
strace (network/process syscalls, tail): ${strace_summary}"
fi

# Ship one immutable component-aware SDK seed. The fixed `share/incan/sdk` location is relocation-stable and contains
# only checked manifests, generated Rust crates, and resolved locks; mutable cache identities and Cargo targets stay out.
sdk_provider_seed="$(prepare_sdk_provider_seed)"
validate_sdk_provider_seed "$sdk_provider_seed"
sdk_seed_root="$package_dir/share/incan/sdk"
if [ -n "${INCAN_SDK_PROVIDER_SEED_DIR:-}" ]; then
  rm -rf "$sdk_seed_root"
  mkdir -p "$(dirname "$sdk_seed_root")"
  cp -R "$sdk_provider_seed" "$sdk_seed_root"
elif [ "$sdk_provider_seed" != "$sdk_seed_root" ]; then
  rm -rf "$sdk_seed_root"
  mv "$sdk_provider_seed" "$sdk_seed_root"
fi
release_provider_store=""
rm -f "$package_dir/share/incan/.incan.lock"
# Provider source is a packaging input, not an installed SDK component. Remove it only after every checked provider
# artifact has been published; consumers receive the Rust support crate plus the relocatable provider payloads.
rm -rf "$package_dir/crates/incan_stdlib/stdlib"
[ ! -d "$package_dir/stdlib" ] || fail "legacy top-level stdlib source unexpectedly entered the package"
[ ! -d "$package_dir/crates/incan_stdlib/stdlib" ] \
  || fail "provider-owned stdlib source unexpectedly entered the package"

# Ship the typed release envelope through the same explicit baker used by local and CI preparation. The baker owns
# fixture source, identity, admission, accounting, and atomic publication; this packaging script only stages its output.
loaf_root="$package_dir/share/incan/oven/loafs"
if [ -n "${INCAN_OVEN_LOAF_DIR:-}" ]; then
  [ "${INCAN_OVEN_LOAF_OVERRIDE_TEST_ONLY:-0}" = "1" ] \
    || fail "INCAN_OVEN_LOAF_DIR is reserved for controlled packaging tests; production archives must invoke the baker"
  [ -d "$INCAN_OVEN_LOAF_DIR" ] \
    || fail "Oven Loaf override does not exist: $INCAN_OVEN_LOAF_DIR"
  mkdir -p "$(dirname "$loaf_root")"
  cp -R "$INCAN_OVEN_LOAF_DIR" "$loaf_root"
else
  if [ -n "${RUSTC:-}" ]; then
    rustc_bin="$RUSTC"
  elif command -v rustup >/dev/null 2>&1; then
    rustc_bin="$(rustup which rustc)"
  else
    rustc_bin="$(command -v rustc)"
  fi
  [ -x "$rustc_bin" ] || fail "could not resolve an executable rustc for the release-only Loaf publisher"
  "$package_dir/bin/incan" oven legacy-cargo bake-loafs \
    --compiler-root "$package_dir" \
    --output "$loaf_root" \
    --envelope release \
    --sdk-inventory "$sdk_seed_root/sdk-inventory.json" \
    --cargo "$cargo_bin" \
    --rustc "$rustc_bin" \
    --format json >/dev/null \
    || fail "could not bake the release Oven Loaf envelope"
fi
[ -d "$loaf_root" ] || fail "release package is missing Oven Loafs"
[ "$(find "$loaf_root" -name loaf.json -type f | wc -l | tr -d ' ')" = "2" ] \
  || fail "release package must contain one release core and one debug Oven foundation Loaf"

sdk_component_count="$(find "$sdk_seed_root/components" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')"
sdk_payload_bytes="$(find "$sdk_seed_root" -type f -exec wc -c {} + | awk '$2 != "total" { total += $1 } END { print total + 0 }')"
loaf_count="$(find "$loaf_root" -name loaf.json -type f | wc -l | tr -d ' ')"
loaf_payload_bytes="$(find "$loaf_root" -type f -exec wc -c {} + | awk '$2 != "total" { total += $1 } END { print total + 0 }')"
loaf_physical_bytes="$(du -sk "$loaf_root" | awk '{ print $1 * 1024 }')"

# The baker has already admitted the complete immutable closure under the central Oven policy. Packaging records both
# logical and host-physical measurements without redefining that policy.
tar -C "$package_dir" -czf "$archive" .
shasum -a 256 "$archive" | awk '{print $1}' > "${archive}.sha256"
archive_bytes="$(wc -c < "$archive" | tr -d ' ')"
cat > "${archive}.profile.json" <<PROFILE_EVIDENCE
{
  "schema_version": 1,
  "release": "${release}",
  "target": "${target}",
  "sdk_profile": "${distribution_profile}",
  "sdk_component_count": ${sdk_component_count},
  "sdk_payload_bytes": ${sdk_payload_bytes},
  "oven_loaf_count": ${loaf_count},
  "oven_loaf_logical_bytes": ${loaf_payload_bytes},
  "oven_loaf_physical_bytes": ${loaf_physical_bytes},
  "archive_bytes": ${archive_bytes}
}
PROFILE_EVIDENCE
printf '%s\n' "$version" > "$out_dir/toolchain-version.txt"
printf '%s\n' "$release" > "$out_dir/toolchain-release.txt"

printf 'Packaged %s\n' "$archive"
