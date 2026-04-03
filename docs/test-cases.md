# Test Cases

## Purpose

This document catalogs the tests that should exist for health-data-parser.
Each test case has a stable ID, a short description, and an expected result so it can be understood without opening source files.

## Test Conventions

- Automated tests shall be written as Rust integration tests under `tests/`.
- Automated tests shall run against the static fixture archive at `tests/fixtures/export.zip` rather than ad hoc inline XML strings or the large example export.
- The plain-text source files used to build the static fixture shall remain in `tests/fixtures/` alongside the generated ZIP so fixture changes are reviewable.
- When the fixture contents change, regenerate `tests/fixtures/export.zip` with `./scripts/regenerate-test-export.sh` instead of editing the archive directly.
- Automated tests should exercise the application through the high-level Rust entry point `health_data_parser::run_from_args(...)` with injected stdout/stderr writers, rather than calling lower-level parsing helpers directly.
- Test names for automated cases shall include the corresponding stable test-case ID, for example `tc_cli_001_*`.
- New automated coverage should prefer user-visible command behavior over internal helper behavior unless a lower-level test is strictly necessary.

## Enumerated Test Cases

### CLI

| ID         | Description                                  | Precondition                                                                 | Actions                                                                                  | Expected Result                                                                                         | Automated |
| ---------- | -------------------------------------------- | ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------- |
| TC-CLI-001 | List output is chronologically ordered       | Static test fixture ZIP with five running workouts stored out of order in XML. | Call `running --file tests/fixtures/export.zip list`.                                    | The Markdown table lists only running workouts, ordered by `startDate`, with stable 1-based indices.   | ✅ `tc_cli_001_list_orders_workouts_chronologically` |
| TC-CLI-002 | Filter workouts by year                      | Static test fixture ZIP with workouts in 2023, 2024, and 2025.               | Call `running --file tests/fixtures/export.zip --year 2024 list`.                       | Output includes only the 2024 workouts.                                                                 | ✅ `tc_cli_002_filters_by_year` |
| TC-CLI-003 | Filter workouts by date range                | Static test fixture ZIP with a workout on `2025-02-01` and other dates outside that month. | Call `running --file tests/fixtures/export.zip --from 2025-02-01 --to 2025-02-28 list`. | Output includes the February 2025 workout and excludes workouts outside the inclusive range.            | ✅ `tc_cli_003_filters_by_date_range` |
| TC-CLI-004 | No matches prints a clear success message    | Static test fixture ZIP with no workouts in 2022.                            | Call `running --file tests/fixtures/export.zip --year 2022 list`.                       | The tool prints `No running workouts found for the given time range.` and exits successfully.           | ✅ `tc_cli_004_reports_no_matches` |
| TC-CLI-005 | Out-of-range show index errors               | Static test fixture ZIP with five matching workouts.                         | Call `running --file tests/fixtures/export.zip show 999`.                               | The tool exits with an error naming the valid range `1–5`.                                              | ✅ `tc_cli_005_show_errors_for_out_of_range_index` |
| TC-CLI-006 | `latest` selects the most recent workout     | Static test fixture ZIP with a most recent workout on `2025-03-01`.          | Call `running --file tests/fixtures/export.zip show latest`.                            | The detail view is rendered for the chronologically last workout in the filtered list.                  | ✅ `tc_cli_006_show_latest_selects_most_recent_workout` |

### Parsing And Data Extraction

| ID          | Description                                         | Precondition                                                                                          | Actions                                                            | Expected Result                                                                                                               | Automated |
| ----------- | --------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- | --------- |
| TC-PARSE-001 | Child stats, metadata, and XML entities are honored | Static test fixture ZIP whose first workout has conflicting `totalDistance`, child statistics, metadata, and escaped attribute values. | Call `running --file tests/fixtures/export.zip show 1`.            | The detail view uses the child distance, shows energy, reports outdoor environment, and prints unescaped source and device. | ✅ `tc_parse_001_show_uses_child_stats_metadata_and_unescaped_attributes` |
| TC-PARSE-002 | Self-closing running workout without GPX is supported | Static test fixture ZIP whose second workout is a self-closing running workout using miles and no route file. | Call `running --file tests/fixtures/export.zip show 2`.            | The detail view shows the converted kilometre distance and prints `No GPS data available for splits.`                       | ✅ `tc_parse_002_show_reports_no_gps_for_self_closing_workout` |
| TC-PARSE-003 | Energy conversion and manual/indoor metadata are shown | Static test fixture ZIP whose fourth workout stores energy in `kcal` and includes `HKIndoorWorkout` and `HKWasUserEntered`. | Call `running --file tests/fixtures/export.zip show 4`.            | The detail view converts energy to kilojoules and shows both the indoor label and manual-entry note.                        | ✅ `tc_parse_003_show_converts_kcal_and_reports_manual_indoor_workout` |

### Computed Metrics And Output

| ID          | Description                                        | Precondition                                                                 | Actions                                                 | Expected Result                                                                                                             | Automated |
| ----------- | -------------------------------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | --------- |
| TC-COMP-001 | Splits include average HR and a partial final km   | Static test fixture ZIP whose first workout has GPX points for 2.5 km and overlapping HR records. | Call `running --file tests/fixtures/export.zip show 1`. | The splits table includes the `avg hr` column, renders two full kilometres plus a `3~` partial split, and shows an extrapolated pace for the partial kilometre. | ✅ `tc_comp_001_show_renders_hr_splits_and_partial_final_km` |
| TC-COMP-002 | Splits omit HR column when no HR data is available | Static test fixture ZIP whose third workout has GPX data but no overlapping HR records.             | Call `running --file tests/fixtures/export.zip show 3`. | The splits table is rendered without the `avg hr` column.                                                                  | ✅ `tc_comp_002_show_omits_hr_column_when_no_hr_exists` |
