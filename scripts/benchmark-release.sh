#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary_path="$repo_root/target/release/health-export-cli"
export_zip="$repo_root/example/export.zip"
start_year=2016
end_year=2025

usage() {
  cat <<EOF
Usage: $(basename "$0") [--binary PATH] [--export PATH] [--from-year YEAR] [--to-year YEAR]

Benchmarks the release binary against an Apple Health export ZIP.

Defaults:
  --binary $binary_path
  --export $export_zip
  --from-year $start_year
  --to-year $end_year
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      if (($# < 2)); then
        printf 'Missing value for %s\n\n' "$1" >&2
        usage >&2
        exit 2
      fi
      binary_path="$2"
      shift 2
      ;;
    --export)
      if (($# < 2)); then
        printf 'Missing value for %s\n\n' "$1" >&2
        usage >&2
        exit 2
      fi
      export_zip="$2"
      shift 2
      ;;
    --from-year)
      if (($# < 2)); then
        printf 'Missing value for %s\n\n' "$1" >&2
        usage >&2
        exit 2
      fi
      start_year="$2"
      shift 2
      ;;
    --to-year)
      if (($# < 2)); then
        printf 'Missing value for %s\n\n' "$1" >&2
        usage >&2
        exit 2
      fi
      end_year="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown argument: %s\n\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if (( start_year > end_year )); then
  printf 'Invalid year range: %s..%s\n' "$start_year" "$end_year" >&2
  exit 2
fi

if [[ ! -f "$export_zip" ]]; then
  printf 'Export ZIP not found: %s\n' "$export_zip" >&2
  exit 1
fi

if [[ ! -x "$binary_path" ]]; then
  printf 'Release binary not found at %s; building it now.\n' "$binary_path"
  (
    cd "$repo_root"
    cargo build --release
  )
fi

ns_now() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import time; print(time.time_ns())'
    return
  fi

  if command -v gdate >/dev/null 2>&1; then
    gdate +%s%N
    return
  fi

  local now
  now="$(date +%s%N 2>/dev/null || true)"
  if [[ "$now" =~ ^[0-9]+$ ]]; then
    printf '%s\n' "$now"
    return
  fi

  printf 'Unable to determine nanosecond timestamp: install python3 or GNU date (gdate), or run on a system with GNU date support.\n' >&2
  exit 1
}

format_ns() {
  awk -v ns="$1" 'BEGIN { printf "%.3f ms", ns / 1000000 }'
}

command_string() {
  printf '%q ' "$@"
}

extract_first_run_id() {
  awk -F'|' '
    /^[|][[:space:]]*[0-9]/ {
      id = $2
      gsub(/[[:space:]]/, "", id)
      print id
      exit
    }
  '
}

declare -a years=()
declare -a selected_ids=()
declare -a result_labels=()
declare -a result_commands=()
declare -a result_durations_ns=()
total_duration_ns=0

# Global registry of temp files; cleaned up by trap on EXIT/INT/TERM.
declare -a _cleanup_files=()
_do_cleanup() {
  if (( ${#_cleanup_files[@]} > 0 )); then
    rm -f "${_cleanup_files[@]}"
  fi
}
trap _do_cleanup EXIT INT TERM

# Stdout captured by the most recent run_benchmark call.
BENCHMARK_STDOUT=""

for (( year = start_year; year <= end_year; year++ )); do
  years+=("$year")
done

run_benchmark() {
  local label="$1"
  shift

  local stdout_file
  local stderr_file
  local start_ns
  local end_ns
  local duration_ns
  local exit_code
  local command_repr

  stdout_file="$(mktemp)"
  stderr_file="$(mktemp)"
  _cleanup_files+=("$stdout_file" "$stderr_file")
  command_repr="$(command_string "$@")"

  printf 'Running %-28s %s\n' "$label" "$command_repr"

  start_ns="$(ns_now)"
  if "$@" >"$stdout_file" 2>"$stderr_file"; then
    exit_code=0
  else
    exit_code=$?
  fi
  end_ns="$(ns_now)"

  duration_ns=$((end_ns - start_ns))
  total_duration_ns=$((total_duration_ns + duration_ns))
  result_labels+=("$label")
  result_commands+=("$command_repr")
  result_durations_ns+=("$duration_ns")

  if (( exit_code != 0 )); then
    printf '\nCommand failed: %s\n' "$label" >&2
    printf 'Command: %s\n' "$command_repr" >&2
    printf 'Exit code: %s\n' "$exit_code" >&2
    printf '\nstdout:\n' >&2
    cat "$stdout_file" >&2
    printf '\nstderr:\n' >&2
    cat "$stderr_file" >&2
    exit "$exit_code"
  fi

  BENCHMARK_STDOUT="$(cat "$stdout_file")"
  rm -f "$stdout_file" "$stderr_file"
}

printf 'Benchmark target: %s\n' "$binary_path"
printf 'Export ZIP: %s\n' "$export_zip"
printf 'Year range: %s..%s\n' "$start_year" "$end_year"
printf '\n'

run_benchmark "list all" "$binary_path" --file "$export_zip" running list

for year in "${years[@]}"; do
  run_benchmark "list $year" "$binary_path" --file "$export_zip" running list --year "$year"

  selected_id="$(printf '%s\n' "$BENCHMARK_STDOUT" | extract_first_run_id)"
  if [[ -z "$selected_id" ]]; then
    printf 'No running workouts found for %s; cannot select a workout for running show.\n' "$year" >&2
    exit 1
  fi
  selected_ids+=("$selected_id")
done

printf '\nSelected workouts for running show (first run in each year):\n'

for index in "${!years[@]}"; do
  printf '  %s -> run #%s\n' "${years[$index]}" "${selected_ids[$index]}"
done

printf '\n'

for index in "${!selected_ids[@]}"; do
  run_benchmark \
    "show ${years[$index]}" \
    "$binary_path" \
    --file "$export_zip" \
    running show "${selected_ids[$index]}"
done

run_benchmark "records all" "$binary_path" --file "$export_zip" running records

for year in "${years[@]}"; do
  run_benchmark "records $year" "$binary_path" --file "$export_zip" running records --year "$year"
done

printf '\nResults:\n'
printf '%-28s %-14s %s\n' 'Benchmark' 'Runtime' 'Command'
printf '%-28s %-14s %s\n' '----------------------------' '--------------' '-------'

for index in "${!result_labels[@]}"; do
  printf \
    '%-28s %-14s %s\n' \
    "${result_labels[$index]}" \
    "$(format_ns "${result_durations_ns[$index]}")" \
    "${result_commands[$index]}"
done

printf '%-28s %-14s %s\n' '----------------------------' '--------------' '-------'
printf \
  '%-28s %-14s %s\n' \
  "suite total (${#result_labels[@]} cmds)" \
  "$(format_ns "$total_duration_ns")" \
  'all benchmarks'