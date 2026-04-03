# Requirements

## Purpose

This document defines the product requirements independent of implementation details.

## Requirement IDs

- IDs are stable and unique.
- Prefixes indicate domain:
  - `CLI-*`: command-line interface and argument handling
  - `PARSE-*`: data extraction and XML/GPX parsing
  - `COMP-*`: computed metrics (duration, pace, splits)
  - `OUT-*`: output formatting

## Requirements

### CLI

- `CLI-001`: The tool shall accept a required `--file` path pointing to an Apple Health export ZIP file, specified on the `running` subcommand.
- `CLI-002`: The tool shall provide a `running list` subcommand that prints all running workouts as a table.
- `CLI-003`: The tool shall provide a `running show <TARGET>` subcommand that prints details for a single workout. `<TARGET>` is either a 1-based index or the literal `latest` (case-insensitive) to select the most recent workout.
- `CLI-005`: The `running` subcommand shall accept an optional `--year` flag to filter workouts to a single calendar year.
- `CLI-006`: The `running` subcommand shall accept optional `--from` and `--to` flags (both required together) to filter workouts to an inclusive date range.
- `CLI-007`: `--year` and `--from`/`--to` shall be mutually exclusive.
- `CLI-008`: When no workouts match the filter, the tool shall print a clear message and exit successfully.
- `CLI-009`: `running show <INDEX>` shall exit with an error when the index is 0 or exceeds the number of matching workouts; `running show latest` shall never produce this error.

### Parsing

- `PARSE-001`: The tool shall read `export.xml` from inside the ZIP without extracting the archive to disk.
- `PARSE-002`: The tool shall extract only workouts with `workoutActivityType="HKWorkoutActivityTypeRunning"` and ignore all other activity types.
- `PARSE-003`: The tool shall support self-closing `<Workout ... />` elements (older export format, no child elements).
- `PARSE-004`: The tool shall support `<Workout>` elements with child elements (newer Apple Watch export format).
- `PARSE-005`: When a `<WorkoutStatistics type="HKQuantityTypeIdentifierDistanceWalkingRunning">` child is present, its `sum` value shall be preferred over the `totalDistance` attribute for distance.
- `PARSE-006`: Distance values in miles shall be converted to kilometres (`× 1.60934`).
- `PARSE-007`: Energy burned shall be extracted from a `<WorkoutStatistics type="HKQuantityTypeIdentifierActiveEnergyBurned">` child element; `kJ` values are stored as-is, `kcal` values are converted to `kJ` (`× 4.184`).
- `PARSE-008`: The recording app (`sourceName`) and device (`device`) shall be extracted from `<Workout>` attributes; XML entities in these values shall be unescaped.
- `PARSE-009`: Indoor/outdoor environment and manual-entry flag shall be extracted from `<MetadataEntry>` children with keys `HKIndoorWorkout` and `HKWasUserEntered`.
- `PARSE-010`: The GPX file path shall be extracted from the `<FileReference>` child of `<WorkoutRoute>`.
- `PARSE-011`: GPX trackpoints shall be parsed from the referenced GPX file inside the ZIP, extracting latitude, longitude, and UTC timestamp from each `<trkpt>`.
- `PARSE-012`: Heart rate records shall be collected from `export.xml` by scanning for `<Record type="HKQuantityTypeIdentifierHeartRate">` elements whose time window overlaps the workout's start–end interval.

### Computed Metrics

- `COMP-001`: Duration shall be computed as the difference between `endDate` and `startDate`, formatted as `M:SS`.
- `COMP-002`: Pace shall be computed as duration divided by distance in km, formatted as `M:SS min/km`.
- `COMP-003`: Per-kilometre splits shall be computed by accumulating haversine distances between consecutive GPX trackpoints and recording elapsed time at each 1 km boundary.
- `COMP-004`: Average heart rate per split shall be computed by averaging all HR records whose time window overlaps the split's time window.

### Output

- `OUT-001`: `running list` shall render a Markdown table with columns: `#`, `Date`, `Start`, `End`, `Duration (M:SS)`, `Distance (km)`, `Pace (min/km)`.
- `OUT-002`: `running show` shall render a key-value detail block including date, start, end, duration, distance, and pace; and optionally energy, source, device, environment, and manual-entry note when those fields are present in the data.
- `OUT-003`: When GPS data is available, `running show` shall render a per-km splits table below the detail block. Complete km splits are labelled by their km number; the final partial km is labelled with a `~` suffix and its pace is extrapolated to per-km so it is directly comparable to complete splits.
- `OUT-004`: The splits table shall include an average heart rate column only when at least one HR record overlapping the workout is found.
- `OUT-005`: When no GPS data is available for a workout, `running show` shall display "No GPS data available for splits."
