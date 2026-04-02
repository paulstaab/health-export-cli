use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate};
use clap::Parser;
use polars::prelude::*;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;

/// Extract running workouts from an Apple Health export ZIP file.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to the Apple Health export ZIP file
    #[arg(short, long)]
    file: PathBuf,

    /// Filter by year (e.g. 2024). Cannot be used together with --from/--to.
    #[arg(long, conflicts_with_all = ["from", "to"])]
    year: Option<i32>,

    /// Start of time range (inclusive), format: YYYY-MM-DD. Requires --to.
    #[arg(long, requires = "to")]
    from: Option<NaiveDate>,

    /// End of time range (inclusive), format: YYYY-MM-DD. Requires --from.
    #[arg(long, requires = "from")]
    to: Option<NaiveDate>,
}

/// A single running workout extracted from the health export.
struct RunningWorkout {
    date: String,
    start_time: String,
    end_time: String,
    /// Distance in kilometres
    distance_km: f64,
}

impl RunningWorkout {
    /// Pace formatted as M:SS min/km
    fn pace(&self) -> String {
        if self.distance_km <= 0.0 {
            return "-".to_string();
        }
        let start = parse_datetime(&self.start_time);
        let end = parse_datetime(&self.end_time);
        match (start, end) {
            (Ok(s), Ok(e)) => {
                let duration_secs = (e - s).num_seconds();
                if duration_secs <= 0 {
                    return "-".to_string();
                }
                let pace_secs_per_km = duration_secs as f64 / self.distance_km;
                let mins = (pace_secs_per_km / 60.0) as u64;
                let secs = (pace_secs_per_km % 60.0) as u64;
                format!("{mins}:{secs:02}")
            }
            _ => "-".to_string(),
        }
    }
}

fn parse_datetime(s: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z")
        .with_context(|| format!("Failed to parse datetime: {s}"))
}

/// Parse the date portion (YYYY-MM-DD) from a health export datetime string.
fn date_part(s: &str) -> &str {
    if s.len() >= 10 { &s[..10] } else { s }
}

/// Parse the time portion (HH:MM) from a health export datetime string.
fn time_part(s: &str) -> &str {
    // format: "2024-01-15 08:30:00 +0100"
    if s.len() >= 16 { &s[11..16] } else { s }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine the date range filter.
    let (from_date, to_date): (Option<NaiveDate>, Option<NaiveDate>) = if let Some(y) = cli.year {
        let from = NaiveDate::from_ymd_opt(y, 1, 1).expect("valid date");
        let to = NaiveDate::from_ymd_opt(y, 12, 31).expect("valid date");
        (Some(from), Some(to))
    } else {
        (cli.from, cli.to)
    };

    // Open the ZIP archive without extracting it.
    let file = File::open(&cli.file)
        .with_context(|| format!("Cannot open file: {}", cli.file.display()))?;
    let mut archive = ::zip::ZipArchive::new(file).context("Failed to read ZIP archive")?;

    // Find the export.xml entry (it may be nested inside a folder).
    let xml_index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .map(|e: ::zip::read::ZipFile<'_>| e.name().ends_with("export.xml"))
                .unwrap_or(false)
        })
        .context("export.xml not found inside the ZIP archive")?;

    let xml_entry = archive
        .by_index(xml_index)
        .context("Failed to open export.xml inside ZIP")?;

    let workouts = extract_running_workouts(xml_entry, from_date, to_date)?;

    if workouts.is_empty() {
        println!("No running workouts found for the given time range.");
        return Ok(());
    }

    print_markdown_table(&workouts)?;
    Ok(())
}

/// Stream-parse the XML and collect running workouts within the optional date range.
fn extract_running_workouts<R: Read>(
    reader: R,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<Vec<RunningWorkout>> {
    let buf_reader = BufReader::new(reader);
    let mut xml = Reader::from_reader(buf_reader);
    xml.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut workouts: Vec<RunningWorkout> = Vec::new();

    loop {
        match xml.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"Workout" {
                    process_workout_element(e, &mut workouts, from, to)?;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "XML parse error at position {}: {e}",
                    xml.error_position()
                ));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(workouts)
}

fn process_workout_element(
    e: &quick_xml::events::BytesStart<'_>,
    workouts: &mut Vec<RunningWorkout>,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<()> {
    let mut activity_type: Option<String> = None;
    let mut start_date: Option<String> = None;
    let mut end_date: Option<String> = None;
    let mut total_distance: Option<f64> = None;
    let mut distance_unit: Option<String> = None;

    for attr in e.attributes() {
        let attr = attr.context("Invalid XML attribute")?;
        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
        let value = std::str::from_utf8(attr.value.as_ref())
            .unwrap_or("")
            .to_owned();

        match key {
            "workoutActivityType" => activity_type = Some(value),
            "startDate" => start_date = Some(value),
            "endDate" => end_date = Some(value),
            "totalDistance" => total_distance = value.parse::<f64>().ok(),
            "totalDistanceUnit" => distance_unit = Some(value),
            _ => {}
        }
    }

    let is_running = activity_type
        .as_deref()
        .map(|t| t == "HKWorkoutActivityTypeRunning")
        .unwrap_or(false);

    if !is_running {
        return Ok(());
    }

    let start = match start_date {
        Some(s) => s,
        None => return Ok(()),
    };
    let end = match end_date {
        Some(e) => e,
        None => return Ok(()),
    };

    // Filter by date range
    let workout_date = match NaiveDate::parse_from_str(date_part(&start), "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return Ok(()), // skip workouts with unparseable dates
    };
    if let Some(f) = from
        && workout_date < f
    {
        return Ok(());
    }
    if let Some(t) = to
        && workout_date > t
    {
        return Ok(());
    }

    // Convert distance to km
    let distance_km = match (total_distance, distance_unit.as_deref()) {
        (Some(d), Some("km")) => d,
        (Some(d), Some("mi")) => d * 1.60934,
        (Some(d), _) => d, // assume km
        (None, _) => 0.0,
    };

    workouts.push(RunningWorkout {
        date: date_part(&start).to_string(),
        start_time: start,
        end_time: end,
        distance_km,
    });

    Ok(())
}

/// Render the workouts as a Markdown table.
fn print_markdown_table(workouts: &[RunningWorkout]) -> Result<()> {
    let dates: Vec<String> = workouts.iter().map(|w| w.date.clone()).collect();
    let starts: Vec<String> = workouts
        .iter()
        .map(|w| time_part(&w.start_time).to_string())
        .collect();
    let ends: Vec<String> = workouts
        .iter()
        .map(|w| time_part(&w.end_time).to_string())
        .collect();
    let distances: Vec<f64> = workouts.iter().map(|w| w.distance_km).collect();
    let paces: Vec<String> = workouts.iter().map(|w| w.pace()).collect();

    let df = DataFrame::new(vec![
        Column::new("Date".into(), dates),
        Column::new("Start".into(), starts),
        Column::new("End".into(), ends),
        Column::new("Distance (km)".into(), distances),
        Column::new("Pace (min/km)".into(), paces),
    ])?;

    let headers: Vec<&str> = df.get_column_names().iter().map(|s| s.as_str()).collect();
    let col_widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let max_data = (0..df.height())
                .map(|r| cell_str(&df, r, i).len())
                .max()
                .unwrap_or(0);
            max_data.max(h.len())
        })
        .collect();

    // Header row
    let header_row = headers
        .iter()
        .zip(&col_widths)
        .map(|(h, w)| format!(" {h:<w$} "))
        .collect::<Vec<_>>()
        .join("|");
    println!("|{header_row}|");

    // Separator row
    let sep_row = col_widths
        .iter()
        .map(|w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("|");
    println!("|{sep_row}|");

    // Data rows
    for row in 0..df.height() {
        let data_row = headers
            .iter()
            .enumerate()
            .zip(&col_widths)
            .map(|((i, _), w)| {
                let val = cell_str(&df, row, i);
                format!(" {val:<w$} ")
            })
            .collect::<Vec<_>>()
            .join("|");
        println!("|{data_row}|");
    }

    Ok(())
}

fn cell_str(df: &DataFrame, row: usize, col: usize) -> String {
    // col is always a valid index since we iterate over df.get_column_names().enumerate()
    let Some(column) = df.get_columns().get(col) else {
        return "-".to_string();
    };
    let Some(series) = column.as_series() else {
        return "-".to_string();
    };
    match series.dtype() {
        DataType::Float64 => {
            if let Ok(ca) = series.f64() {
                ca.get(row)
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "-".to_string())
            } else {
                "-".to_string()
            }
        }
        _ => {
            if let Ok(ca) = series.str() {
                ca.get(row)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string())
            } else {
                series
                    .get(row)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| "-".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<HealthData>
  <Workout workoutActivityType="HKWorkoutActivityTypeRunning"
           duration="60" durationUnit="min"
           totalDistance="10.5" totalDistanceUnit="km"
           startDate="2024-03-15 08:00:00 +0100"
           endDate="2024-03-15 09:00:00 +0100"/>
  <Workout workoutActivityType="HKWorkoutActivityTypeCycling"
           duration="45" durationUnit="min"
           totalDistance="25" totalDistanceUnit="km"
           startDate="2024-03-16 07:00:00 +0100"
           endDate="2024-03-16 07:45:00 +0100"/>
  <Workout workoutActivityType="HKWorkoutActivityTypeRunning"
           duration="30" durationUnit="min"
           totalDistance="5.0" totalDistanceUnit="km"
           startDate="2023-12-20 18:00:00 +0100"
           endDate="2023-12-20 18:30:00 +0100"/>
</HealthData>"#;

    #[test]
    fn test_extract_all_running_workouts() {
        let workouts = extract_running_workouts(SAMPLE_XML.as_bytes(), None, None).unwrap();
        assert_eq!(workouts.len(), 2, "Should find exactly 2 running workouts");
        assert_eq!(workouts[0].date, "2024-03-15");
        assert!((workouts[0].distance_km - 10.5).abs() < 0.001);
    }

    #[test]
    fn test_filter_by_year() {
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let workouts =
            extract_running_workouts(SAMPLE_XML.as_bytes(), Some(from), Some(to)).unwrap();
        assert_eq!(workouts.len(), 1);
        assert_eq!(workouts[0].date, "2024-03-15");
    }

    #[test]
    fn test_filter_by_date_range() {
        let from = NaiveDate::from_ymd_opt(2023, 12, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2023, 12, 31).unwrap();
        let workouts =
            extract_running_workouts(SAMPLE_XML.as_bytes(), Some(from), Some(to)).unwrap();
        assert_eq!(workouts.len(), 1);
        assert_eq!(workouts[0].date, "2023-12-20");
    }

    #[test]
    fn test_pace_calculation() {
        let workout = RunningWorkout {
            date: "2024-03-15".to_string(),
            start_time: "2024-03-15 08:00:00 +0000".to_string(),
            end_time: "2024-03-15 09:00:00 +0000".to_string(),
            distance_km: 10.0,
        };
        // 60 min / 10 km = 6:00 min/km
        assert_eq!(workout.pace(), "6:00");
    }

    #[test]
    fn test_pace_with_seconds() {
        let workout = RunningWorkout {
            date: "2024-03-15".to_string(),
            start_time: "2024-03-15 08:00:00 +0000".to_string(),
            end_time: "2024-03-15 08:57:30 +0000".to_string(),
            distance_km: 10.0,
        };
        // 3450 sec / 10 km = 345 sec/km = 5:45 min/km
        assert_eq!(workout.pace(), "5:45");
    }

    #[test]
    fn test_mile_conversion() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<HealthData>
  <Workout workoutActivityType="HKWorkoutActivityTypeRunning"
           totalDistance="6.21" totalDistanceUnit="mi"
           startDate="2024-01-01 08:00:00 +0000"
           endDate="2024-01-01 09:00:00 +0000"/>
</HealthData>"#;
        let workouts = extract_running_workouts(xml.as_bytes(), None, None).unwrap();
        assert_eq!(workouts.len(), 1);
        assert!((workouts[0].distance_km - 6.21 * 1.60934).abs() < 0.01);
    }

    #[test]
    fn test_no_workouts() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<HealthData>
  <Workout workoutActivityType="HKWorkoutActivityTypeCycling"
           startDate="2024-01-01 08:00:00 +0000"
           endDate="2024-01-01 09:00:00 +0000"/>
</HealthData>"#;
        let workouts = extract_running_workouts(xml.as_bytes(), None, None).unwrap();
        assert!(workouts.is_empty());
    }
}
