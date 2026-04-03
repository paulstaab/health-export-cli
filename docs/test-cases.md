# Test Cases

## Purpose

This document catalogs the tests that should exist for health-data-parser.
Each test case has a stable ID, a short description, and an expected result so it can be understood without opening source files.

## Enumerated Test Cases

### Parsing

| ID          | Description                                      | Precondition                                                        | Actions                                                                                                          | Expected Result                                                                             | Automated |
| ----------- | ------------------------------------------------ | ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | --------- |
| TC-PARSE-001 | Extract running workouts, skip other types      | XML with two running workouts and one cycling workout.              | Call `extract_running_workouts` with no date filter.                                                             | Returns exactly 2 workouts; first has date `2024-03-15` and distance `10.5 km`. | ✅ `test_extract_all_running_workouts` |
| TC-PARSE-002 | Miles-to-km conversion                          | XML with a running workout whose distance is in miles.              | Call `extract_running_workouts` with `totalDistanceUnit="mi"` and `totalDistance="6.21"`.                        | Stored `distance_km` equals `6.21 × 1.60934` within `0.01` tolerance.           | ✅ `test_mile_conversion` |
| TC-PARSE-003 | Distance from WorkoutStatistics child element   | XML with a running workout that has no `totalDistance` attribute but has a `WorkoutStatistics` distance child. | Call `extract_running_workouts`.                                                             | `distance_km` is taken from the child element's `sum` value (`5.54719 km`).     | ✅ `test_distance_from_workout_statistics_child` |
| TC-PARSE-004 | Non-running workout is excluded                 | XML with only a cycling workout.                                    | Call `extract_running_workouts` with no date filter.                                                             | Returns an empty list.                                                                      | ✅ `test_no_workouts` |
| TC-PARSE-005 | Energy extracted from WorkoutStatistics child   | XML with a running workout whose energy is in a `WorkoutStatistics` child with `unit="kJ"`. | Call `extract_running_workouts`.                                                        | `energy_kj` on the returned workout equals the `sum` value.                      | ✅ `test_energy_kj_from_workout_statistics_child` |
| TC-PARSE-006 | GPX path extracted from WorkoutRoute            | XML with a running workout containing a `WorkoutRoute`/`FileReference` child. | Call `extract_running_workouts`.                                                        | `gpx_path` on the returned workout equals the `path` attribute value.            | ✅ `test_gpx_path_from_workout_route` |
| TC-PARSE-007 | Indoor flag extracted from MetadataEntry        | XML with a running workout containing `<MetadataEntry key="HKIndoorWorkout" value="1"/>`. | Call `extract_running_workouts`.                                                   | `indoor` on the returned workout is `Some(true)`.                                | ✅ `test_metadata_entry_indoor_and_user_entered` |
| TC-PARSE-008 | Source name and device extracted from attributes | XML with a running workout that has `sourceName` and `device` attributes with XML entities. | Call `extract_running_workouts`.                                                | `source_name` and `device` are correctly unescaped strings.                      | ❌ |

### Filtering

| ID         | Description                    | Precondition                                         | Actions                                                                                 | Expected Result                                                    | Automated |
| ---------- | ------------------------------ | ---------------------------------------------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------------------ | --------- |
| TC-CLI-001 | Filter workouts by year        | XML with workouts in 2024 and 2023.                  | Call `extract_running_workouts` with `from=2024-01-01`, `to=2024-12-31`.                | Returns only the 2024 workout.                                     | ✅ `test_filter_by_year` |
| TC-CLI-002 | Filter workouts by date range  | XML with workouts in March 2024 and December 2023.   | Call `extract_running_workouts` with `from=2023-12-01`, `to=2023-12-31`.                | Returns only the December 2023 workout.                            | ✅ `test_filter_by_date_range` |
| TC-CLI-003 | Out-of-range show index errors | Any non-empty list of workouts.                      | Call the `show` dispatch path with `index=0` or `index > workouts.len()`.              | Returns an error naming the valid range.                           | ❌ |

### Computed Metrics

| ID         | Description                          | Precondition                                            | Actions                                           | Expected Result                                                  | Automated |
| ---------- | ------------------------------------ | ------------------------------------------------------- | ------------------------------------------------- | ---------------------------------------------------------------- | --------- |
| TC-COMP-001 | Pace calculation — whole minutes    | `RunningWorkout` with a 60-minute duration over 10 km.  | Call `.pace()`.                                   | Returns `"6:00"`.                                                | ✅ `test_pace_calculation` |
| TC-COMP-002 | Pace calculation — with seconds     | `RunningWorkout` with a 57m30s duration over 10 km.     | Call `.pace()`.                                   | Returns `"5:45"`.                                                | ✅ `test_pace_with_seconds` |
| TC-COMP-003 | Duration calculation                | `RunningWorkout` with known start and end timestamps.   | Call `.duration()`.                               | Returns elapsed time formatted as `M:SS`.                        | ❌ |
| TC-COMP-004 | Haversine distance between two points | Two lat/lon coordinates with a known ground-truth distance. | Call `haversine_km`.                          | Returns the expected distance within a small tolerance.          | ❌ |
| TC-COMP-005 | Per-km split timing                 | A sequence of `GpxPoint` values covering more than 1 km with known timestamps. | Call `compute_splits` with no HR records. | Returns one `KmSplit` per complete km with correct `duration_secs`. | ❌ |
| TC-COMP-006 | Average HR per split                | A sequence of `GpxPoint` values and overlapping `HrRecord` values. | Call `compute_splits`.                       | Each split's `avg_hr` is the mean of overlapping HR records.     | ❌ |
| TC-COMP-007 | Partial final-km split emitted      | A sequence of `GpxPoint` values whose total haversine distance is not a whole number of km. | Call `compute_splits` with no HR records. | The last `KmSplit` has `partial_km = Some(...)` and its `duration_secs` covers only the remaining fraction. | ❌ |
| TC-COMP-008 | Partial split pace is extrapolated  | A partial final split with known `duration_secs` and `partial_km`. | Render via `print_show`.                    | The displayed pace equals `duration_secs / partial_km`, formatted as `M:SS`; the km label ends with `~`. | ❌ |
