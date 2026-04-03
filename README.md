# health-export-cli

A Rust CLI tool that extracts running workouts from an Apple Health export ZIP file and renders lists, details, and running records.

## Features

- Reads directly from the Apple Health export ZIP — no manual unzipping required
- Streams the XML data to avoid loading the entire export into memory
- Filters workouts by year or a custom date range
- Outputs Markdown-compatible tables for workout lists and running records
- Shows detailed workout views with pace, distance, metadata, and per-km splits when GPS data is available

## Usage

```
health-export-cli [--file <export.zip>] running <list|records|show <RUN_ID|latest>>
```

If `--file` is omitted, the tool defaults to `./export.zip` in the current working directory.

`running list` and `running records` accept filters: `[--year <YEAR> [--month <1-12>] | --from <YYYY-MM-DD> --to <YYYY-MM-DD>]`.

`--month` requires `--year` and is not accepted by `running show`.

### Examples

List all running workouts from 2024:

```sh
health-export-cli --file export.zip running list --year 2024
```

Show running records across the full export:

```sh
health-export-cli running records
```

Show running records for a filtered date range:

```sh
health-export-cli --file export.zip running records --from 2024-01-01 --to 2024-12-31
```

Show monthly records for February 2025:

```sh
health-export-cli --file export.zip running records --year 2025 --month 2
```

Show the most recent workout in detail:

```sh
health-export-cli running show latest
```

### Sample list output

```
| Run ID | Date       | Start | End   | Duration (min) | Distance (km) | Pace (min/km) |
|--------|------------|-------|-------|----------------|---------------|---------------|
| 11     | 2024-03-15 | 07:30 | 08:17 | 47:00          | 8.50          | 5:31          |
| 12     | 2024-03-20 | 06:00 | 07:02 | 62:00          | 10.00         | 6:12          |
```

### Sample records output

```
| Record Type           | Run ID | Date       | Duration (min) | Pace (min/km) | Distance (km) |
|-----------------------|--------|------------|----------------|---------------|---------------|
| Longest Run           | 25     | 2024-11-10 | 224:30         | 5:19          | 42.20         |
| Fastest 5k            | 7      | 2024-04-18 | 23:45          | 4:45          | 5.00          |
| Fastest 10k           | 12     | 2024-07-21 | 47:10          | 4:43          | 10.00         |
| Fastest Half Marathon | 18     | 2024-10-13 | 98:30          | 4:40          | 21.10         |
| Fastest Marathon      | 25     | 2024-11-10 | 224:30         | 5:19          | 42.20         |
```

## Building

```sh
cargo build --release
```

The binary will be at `target/release/health-export-cli`.

## Testing

```sh
cargo test
```
