#!/usr/bin/env bash

# Install Incan into throwaway containers and prove a first-time user can build immediately.
#
# Clean rooms that contain no Rust are not sufficient evidence. The v0.5 release candidates passed exactly that
# check and still failed for real users, because a machine that already has Rust at a different version than the
# release was built with hit a compiler-identity wall on the first `incan build`. Both states are therefore
# checked here: no Rust at all, and Rust deliberately pinned to a version the release was not built with.
#
# This is a local gate. It downloads a toolchain and provisions Rust twice, which is far too slow for per-pull-
# request CI and belongs in front of a release instead.

set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  bash scripts/gate_cleanroom.sh [options]

Runs two container scenarios against one installer + manifest, and fails unless a new project builds and runs in
both. Requires a working Docker (or Colima) daemon.

Options:
  --manifest REF     Manifest URL or path handed to the installer
                     (default: the latest published release manifest)
  --dist PATH        Local dist directory holding install.sh, manifest.json and archives; served to the
                     containers instead of downloading a published release
  --mismatched-rust VERSION
                     Rust version installed as the container default in the second scenario
                     (default: 1.92.0)
  --image NAME       Base image (default: debian:bookworm-slim)
  --platform NAME    Docker platform (default: linux/amd64)
  --scenario NAME    Run only "no-rust" or "mismatched-rust"
  -h, --help         Show this help
EOF
}

fail() {
    printf 'gate_cleanroom: %s\n' "$*" >&2
    exit 1
}

manifest_ref="https://github.com/encero-systems/incan/releases/latest/download/manifest.json"
dist=""
mismatched_rust="1.92.0"
image="debian:bookworm-slim"
platform="linux/amd64"
scenario="all"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --manifest) manifest_ref="${2:-}"; shift 2 ;;
        --dist) dist="${2:-}"; shift 2 ;;
        --mismatched-rust) mismatched_rust="${2:-}"; shift 2 ;;
        --image) image="${2:-}"; shift 2 ;;
        --platform) platform="${2:-}"; shift 2 ;;
        --scenario) scenario="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "unknown argument: $1" ;;
    esac
done

command -v docker >/dev/null 2>&1 || fail "docker is required (start Colima or Docker Desktop first)"
docker info >/dev/null 2>&1 || fail "docker daemon is not reachable (try: colima start)"

mount_args=()
if [ -n "$dist" ]; then
    [ -d "$dist" ] || fail "--dist is not a directory: $dist"
    dist="$(cd "$dist" && pwd)"
    [ -f "$dist/install.sh" ] || fail "--dist has no install.sh: $dist"
    [ -f "$dist/manifest.json" ] || fail "--dist has no manifest.json: $dist"
    # Colima turns single-file bind mounts into directories, so mount the whole directory instead.
    mount_args=(-v "$dist:/dist:ro")
    manifest_ref="/dist/manifest.json"
fi

# The container script is passed base64-encoded through the environment: single-file bind mounts are unreliable
# under Colima, and encoding avoids every quoting hazard of inlining a script into `docker run sh -c`.
container_script() {
    local install_rust="$1"
    cat <<EOF
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq curl ca-certificates build-essential jq bash >/dev/null

if [ -n "${install_rust}" ]; then
  echo "== Pre-installing Rust ${install_rust} as the machine default =="
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain "${install_rust}" >/dev/null
  . "\$HOME/.cargo/env"
  echo "   machine default: \$(rustc --version)"
fi

echo "== Installing Incan =="
if [ -d /dist ]; then
  bash /dist/install.sh --manifest "${manifest_ref}"
else
  curl -fsSL "${manifest_ref%/manifest.json}/install.sh" | bash -s -- --manifest "${manifest_ref}"
fi
export PATH="\$HOME/.local/bin:\$PATH"

echo "== Verifying the installed toolchain =="
incan --version

if [ -n "${install_rust}" ]; then
  echo "   machine default is still: \$(rustc --version)"
fi
if [ -d "\$HOME/.incan/rust/toolchains" ]; then
  echo "   Incan-owned toolchain: \$(ls "\$HOME/.incan/rust/toolchains")"
else
  echo "   WARNING: no Incan-owned Rust toolchain was provisioned" >&2
  exit 1
fi

echo "== Building and running a new project =="
cd /tmp
incan new hello --yes
cd hello
incan run
EOF
}

run_scenario() {
    local name="$1"
    local install_rust="$2"
    printf '\n======== Clean room: %s ========\n' "$name"
    local encoded
    encoded="$(container_script "$install_rust" | base64 | tr -d '\n')"
    docker run --rm --platform "$platform" "${mount_args[@]}" \
        -e "GATE_SCRIPT=$encoded" \
        "$image" \
        sh -c 'echo "$GATE_SCRIPT" | base64 -d | bash' \
        || fail "clean-room scenario failed: $name"
    printf '\n-------- %s: passed --------\n' "$name"
}

case "$scenario" in
    all)
        run_scenario "no Rust installed" ""
        run_scenario "Rust ${mismatched_rust} already the default" "$mismatched_rust"
        ;;
    no-rust) run_scenario "no Rust installed" "" ;;
    mismatched-rust) run_scenario "Rust ${mismatched_rust} already the default" "$mismatched_rust" ;;
    *) fail "unknown scenario: $scenario (expected all, no-rust, or mismatched-rust)" ;;
esac

printf '\nClean-room gate passed.\n'
