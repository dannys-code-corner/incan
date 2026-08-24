#!/bin/sh

# Exercise the renderer against a small, checked-in GitHub fixture. This keeps
# docs builds offline while verifying hierarchy, blocker edges, completion
# state, and deterministic output for the public roadmap component.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fixture_dir="$script_dir/fixtures"
check_dir=$(mktemp -d "${TMPDIR:-/tmp}/incan-v06-roadmap-check.XXXXXX")

cleanup() {
  rm -rf "$check_dir"
}

trap cleanup 0 1 2 15

render() {
  output_path=$1
  jq --null-input \
    --raw-output \
    --arg snapshot_date "2026-08-24" \
    --slurpfile issues "$fixture_dir/github_issues.json" \
    --slurpfile blocked_by "$fixture_dir/github_blocked_by.json" \
    --from-file "$script_dir/render_roadmap.jq" > "$output_path"
}

first_render="$check_dir/first.md"
second_render="$check_dir/second.md"
render "$first_render"
render "$second_render"
cmp -s "$first_render" "$second_render"

grep -Fq 's1137 -- owns --> i653' "$first_render"
grep -Fq 'flowchart LR' "$first_render"
grep -Fq 'i1103 -. blocks .-> i653' "$first_render"
grep -Fq 'class i1103,s1137 incv06complete' "$first_render"
grep -Fq '<span class="inc-v06-status inc-v06-status--complete">1 complete</span>' "$first_render"
grep -Fq '<span class="inc-v06-status inc-v06-status--open">1 open</span>' "$first_render"

if grep -Fq 'Blocked-by relationships are unavailable' "$first_render"; then
  printf '%s\n' 'fixture renderer unexpectedly omitted blocker facts' >&2
  exit 1
fi
