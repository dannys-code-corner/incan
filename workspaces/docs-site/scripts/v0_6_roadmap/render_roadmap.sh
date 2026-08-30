#!/bin/sh

# Render a checked-in roadmap snapshot from the public JSON fetched by
# refresh_github.sh. The renderer intentionally has no network behavior.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
output_path="$script_dir/../../docs/_snippets/project/v0_6_roadmap.md"
snapshot_date_path="$script_dir/github_snapshot_date.txt"

if [ ! -f "$snapshot_date_path" ]; then
  printf '%s\n' "Missing $snapshot_date_path; run refresh_github.sh first." >&2
  exit 1
fi

jq --null-input \
  --raw-output \
  --arg snapshot_date "$(cat "$snapshot_date_path")" \
  --slurpfile issues "$script_dir/github_issues.json" \
  --slurpfile blocked_by "$script_dir/github_blocked_by.json" \
  --from-file "$script_dir/render_roadmap.jq" > "$output_path"
