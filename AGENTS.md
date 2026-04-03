# Agent Instructions

## Project overview

Rust CLI (`health-data-parser`) that parses Apple Health export ZIP files and extracts running workout data. All logic lives in `src/main.rs`.

## CLI structure

```
health-data-parser running --file <PATH> [--year Y | --from D --to D] <SUBCOMMAND>
  list            Print a markdown table of all running workouts (with 1-based # index)
  show <N|latest> Show detail view for workout at 1-based index N, or the most recent workout with `latest`
```

`--file`, `--year`, `--from`, `--to` belong to the `running` subcommand, not the root.

## Key data sources

- `export.xml` inside the ZIP — all workout and heart rate data
- `workout-routes/route_*.gpx` files inside the ZIP — GPS trackpoints (lat/lon/time, ~1 Hz)
- Heart rate: `<Record type="HKQuantityTypeIdentifierHeartRate" .../>` elements in `export.xml`; time-windowed, matched against workout/split time ranges by overlap

Workouts have **no unique ID** in the export format. Identification is by 1-based position in the sorted list.

## Code structure (`src/main.rs`)

| Item | Purpose |
|---|---|
| `Cli / Commands / RunningArgs / RunningSubcommand` | clap 4 derive hierarchy |
| `RunningWorkout` | Parsed workout; `#[derive(Default)]` — use `..Default::default()` in test literals |
| `WorkoutChildData` | Collects child-element data (distance, energy, metadata, GPX path) during XML parsing |
| `GpxPoint` | lat/lon/time from a GPX trackpoint |
| `HrRecord` | Heart rate window (start/end/bpm) |
| `KmSplit` | Per-km split (km number, duration_secs, avg_hr) |
| `extract_running_workouts` | Streaming XML parse; entry point for `list` |
| `read_workout_children` | Reads `<Workout>` child elements; handles `WorkoutStatistics`, `MetadataEntry`, `WorkoutRoute`/`FileReference` |
| `parse_workout_attrs` | Builds `RunningWorkout` from element attributes + child data; uses `attr.unescape_value()` for string fields |
| `parse_gpx` | Streaming GPX parse; extracts `GpxPoint` list |
| `collect_heart_rate` | Second pass through `export.xml`; returns HR records overlapping a time window |
| `compute_splits` | Haversine accumulation over GPX points → `Vec<KmSplit>` |
| `haversine_km` | Inline haversine formula, no extra dependency |
| `print_markdown_table` | Polars DataFrame → markdown table; first column is `#` index |
| `print_show` | Key-value detail + km splits table; omits HR column when no HR data |
| `show_workout` / `compute_workout_splits` | Orchestrate GPX + HR loading for `show`/`latest`; open ZIP in separate scoped blocks |

## Important implementation details

- **XML parsing uses `quick-xml` in non-namespace mode** — element names are plain (`trkpt`, not `{ns}trkpt`).
- **`zip` name conflicts with polars glob import** — always use `::zip::ZipArchive` and `::zip::read::ZipFile<'_>` (crate-root prefix).
- **ZIP opened multiple times for `show`/`latest`**: once for workouts, once for the GPX file, once for HR. Each open is scoped to its own block so borrows don't overlap.
- **`attr.unescape_value()`** must be used (not raw `attr.value`) for string fields that may contain XML entities (e.g. the `device` attribute contains `<` / `>`).
- **GPX times** are RFC 3339 (`2022-10-06T06:01:12Z`); use `DateTime::parse_from_rfc3339`.
- **Health export times** are `"YYYY-MM-DD HH:MM:SS +ZZZZ"`; use `parse_datetime()`.
- **`chrono::DateTime<FixedOffset>` comparisons** are timezone-aware — safe to compare GPX (UTC) against health export (local offset).
- `WorkoutRoute` is a `Start` event (has children), not `Empty`. Use a separate `route_buf` inside the inner loop to avoid borrow conflicts with the outer `buf`.

## Testing

```bash
cargo test                                               # 11 unit tests, must all pass
cargo run -- running --file example/export.zip list
cargo run -- running --file example/export.zip show latest
cargo run -- running --file example/export.zip show 1
cargo run -- running --file example/export.zip show 999   # should error
cargo run -- running --file example/export.zip --year 2024 list
```

The example ZIP is at `example/export.zip` (157 MB). Example GPX files are under `example/apple_health_export/workout-routes/`.

## After every change

Run these three commands in order and fix any issues before considering work done:

```bash
cargo fmt                  # auto-format; never leave unformatted code
cargo clippy               # must produce zero warnings
cargo test                 # all tests must pass
```

`cargo fmt` rewrites the files in place. `cargo clippy` without extra flags uses the default lint set — treat every warning as a required fix, not a suggestion.

Keep `docs/requirements.md` and `docs/test-cases.md` up to date:

- **New or changed behaviour** → add or update requirements in `docs/requirements.md` using the existing ID scheme (`CLI-*`, `PARSE-*`, `COMP-*`, `OUT-*`). Never reuse a retired ID.
- **New, changed, or removed tests** → reflect the change in `docs/test-cases.md`. Mark automated tests with ✅ and the test function name; mark gaps with ❌. Update the `Automated` column whenever a previously manual case gets a test, or vice-versa.

## Coding conventions

- No new crate dependencies without a strong reason — all current needs are met by `clap`, `zip`, `quick-xml`, `polars`, `chrono`, `anyhow`.
- `RunningWorkout` uses `#[derive(Default)]`; test literals should use `..Default::default()` for fields not under test.
- Streaming XML throughout — never load the entire file into memory.
