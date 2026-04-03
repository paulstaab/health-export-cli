#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixtures_dir="$repo_root/tests/fixtures"
zip_path="$fixtures_dir/export.zip"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/workout-routes"
cp "$fixtures_dir/export.xml" "$tmp_dir/export.xml"
cp "$fixtures_dir"/workout-routes/*.gpx "$tmp_dir/workout-routes/"
find "$tmp_dir" -exec touch -t 202401010000 {} +

rm -f "$zip_path"
(
  cd "$tmp_dir"
  find export.xml workout-routes -type f | LC_ALL=C sort | zip -q -X "$zip_path" -@
)

echo "Wrote $zip_path"