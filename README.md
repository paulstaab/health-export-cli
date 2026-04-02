# health-data-parser

A Rust CLI tool that extracts running workouts from an Apple Health export ZIP file and renders them as a Markdown table.

## Features

- Reads directly from the Apple Health export ZIP — no manual unzipping required
- Streams the XML data to avoid loading the entire export into memory
- Filters workouts by year or a custom date range
- Outputs a Markdown table with date, start/end time, distance (km), and pace (min/km)

## Usage

```
health-data-parser --file <export.zip> [--year <YEAR> | --from <YYYY-MM-DD> --to <YYYY-MM-DD>]
```

### Examples

Extract all running workouts from 2024:

```sh
health-data-parser --file export.zip --year 2024
```

Extract running workouts between two dates:

```sh
health-data-parser --file export.zip --from 2024-01-01 --to 2024-06-30
```

Extract all running workouts (no date filter):

```sh
health-data-parser --file export.zip
```

### Sample output

```
| Date       | Start | End   | Distance (km) | Pace (min/km) |
|------------|-------|-------|---------------|---------------|
| 2024-03-15 | 07:30 | 08:17 | 8.50          | 5:33          |
| 2024-03-20 | 06:00 | 07:02 | 10.00         | 6:15          |
```

## Building

```sh
cargo build --release
```

The binary will be at `target/release/health-data-parser`.

## Testing

```sh
cargo test
```
