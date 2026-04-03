use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate};
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
use polars::prelude::*;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;

/// Extract running workouts from an Apple Health export ZIP file.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Work with running workouts
    Running(RunningArgs),
}

#[derive(Args, Debug)]
struct RunningArgs {
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

    #[command(subcommand)]
    subcommand: RunningSubcommand,
}

#[derive(Subcommand, Debug)]
enum RunningSubcommand {
    /// List all running workouts as a table
    List,
    /// Show details for a workout. Pass a 1-based index or "latest".
    Show {
        /// 1-based index from the list output, or "latest" for the most recent workout
        target: ShowTarget,
    },
}

#[derive(Debug, Clone)]
enum ShowTarget {
    Index(usize),
    Latest,
}

impl std::str::FromStr for ShowTarget {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("latest") {
            Ok(ShowTarget::Latest)
        } else {
            s.parse::<usize>()
                .map(ShowTarget::Index)
                .map_err(|_| format!("expected a positive integer or \"latest\", got \"{s}\""))
        }
    }
}

/// A single running workout extracted from the health export.
#[derive(Default)]
struct RunningWorkout {
    date: String,
    start_time: String,
    end_time: String,
    /// Distance in kilometres
    distance_km: f64,
    energy_kj: Option<f64>,
    source_name: Option<String>,
    device: Option<String>,
    indoor: Option<bool>,
    user_entered: Option<bool>,
    /// Relative path to GPX file within the ZIP (from WorkoutRoute FileReference)
    gpx_path: Option<String>,
}

impl RunningWorkout {
    /// Duration formatted as M:SS
    fn duration(&self) -> String {
        match (
            parse_datetime(&self.start_time),
            parse_datetime(&self.end_time),
        ) {
            (Ok(s), Ok(e)) => {
                let secs = (e - s).num_seconds();
                if secs <= 0 {
                    return "-".to_string();
                }
                format!("{}:{:02}", secs / 60, secs % 60)
            }
            _ => "-".to_string(),
        }
    }

    /// Pace formatted as M:SS min/km
    fn pace(&self) -> String {
        if self.distance_km <= 0.0 {
            return "-".to_string();
        }
        match (
            parse_datetime(&self.start_time),
            parse_datetime(&self.end_time),
        ) {
            (Ok(s), Ok(e)) => {
                let secs = (e - s).num_seconds();
                if secs <= 0 {
                    return "-".to_string();
                }
                let pace = secs as f64 / self.distance_km;
                format!("{}:{:02}", (pace / 60.0) as u64, (pace % 60.0) as u64)
            }
            _ => "-".to_string(),
        }
    }
}

/// Data collected from child elements of a `<Workout>` element.
#[derive(Default)]
struct WorkoutChildData {
    distance_km: Option<f64>,
    energy_kj: Option<f64>,
    indoor: Option<bool>,
    user_entered: Option<bool>,
    gpx_path: Option<String>,
}

/// A GPX trackpoint (lat/lon + timestamp).
struct GpxPoint {
    lat: f64,
    lon: f64,
    time: DateTime<FixedOffset>,
}

/// A heart rate measurement window from the health export.
struct HrRecord {
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    bpm: f64,
}

/// A per-kilometre split.
struct KmSplit {
    km: usize,
    duration_secs: i64,
    avg_hr: Option<f64>,
    /// Set for the final partial split; holds the actual distance covered (< 1 km).
    partial_km: Option<f64>,
}

pub fn run_from_args<I, T, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    W: Write,
    E: Write,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => match err.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                write!(stdout, "{err}")?;
                return Ok(0);
            }
            _ => {
                write!(stderr, "{err}")?;
                return Ok(2);
            }
        },
    };

    match execute(cli, stdout) {
        Ok(()) => Ok(0),
        Err(err) => {
            writeln!(stderr, "Error: {err}")?;
            Ok(1)
        }
    }
}

fn execute<W: Write>(cli: Cli, stdout: &mut W) -> Result<()> {
    let Commands::Running(running) = cli.command;

    let workouts = load_running_workouts(&running)?;
    if workouts.is_empty() {
        writeln!(
            stdout,
            "No running workouts found for the given time range."
        )?;
        return Ok(());
    }

    match running.subcommand {
        RunningSubcommand::List => print_markdown_table(stdout, &workouts)?,
        RunningSubcommand::Show { target } => {
            let index = match target {
                ShowTarget::Latest => workouts.len(),
                ShowTarget::Index(i) => {
                    if i == 0 || i > workouts.len() {
                        anyhow::bail!(
                            "Index {i} is out of range. Valid range: 1–{}",
                            workouts.len()
                        );
                    }
                    i
                }
            };
            show_workout(stdout, index, &workouts[index - 1], &running.file)?;
        }
    }

    Ok(())
}

fn load_running_workouts(running: &RunningArgs) -> Result<Vec<RunningWorkout>> {
    let (from_date, to_date): (Option<NaiveDate>, Option<NaiveDate>) = if let Some(y) = running.year
    {
        let from = NaiveDate::from_ymd_opt(y, 1, 1).expect("valid date");
        let to = NaiveDate::from_ymd_opt(y, 12, 31).expect("valid date");
        (Some(from), Some(to))
    } else {
        (running.from, running.to)
    };

    let file = File::open(&running.file)
        .with_context(|| format!("Cannot open file: {}", running.file.display()))?;
    let mut archive = ::zip::ZipArchive::new(file).context("Failed to read ZIP archive")?;
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

    let mut workouts = extract_running_workouts(xml_entry, from_date, to_date)?;
    sort_workouts(&mut workouts);
    Ok(workouts)
}

fn sort_workouts(workouts: &mut [RunningWorkout]) {
    workouts.sort_by(|left, right| {
        match (
            parse_datetime(&left.start_time),
            parse_datetime(&right.start_time),
        ) {
            (Ok(left_dt), Ok(right_dt)) => left_dt.cmp(&right_dt),
            _ => left.start_time.cmp(&right.start_time),
        }
    });
}

fn parse_datetime(s: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z")
        .with_context(|| format!("Failed to parse datetime: {s}"))
}

/// Returns the YYYY-MM-DD prefix of a health export datetime string.
fn date_part(s: &str) -> &str {
    if s.len() >= 10 { &s[..10] } else { s }
}

/// Returns the HH:MM portion of a health export datetime string.
fn time_part(s: &str) -> &str {
    if s.len() >= 16 { &s[11..16] } else { s }
}

/// Haversine distance in kilometres between two lat/lon points.
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0_f64;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    r * 2.0 * a.sqrt().asin()
}

fn show_workout<W: Write>(
    stdout: &mut W,
    index: usize,
    workout: &RunningWorkout,
    zip_path: &PathBuf,
) -> Result<()> {
    let (splits, has_hr) = compute_workout_splits(workout, zip_path)?;
    print_show(stdout, index, workout, &splits, has_hr)
}

fn compute_workout_splits(
    workout: &RunningWorkout,
    zip_path: &PathBuf,
) -> Result<(Vec<KmSplit>, bool)> {
    let Some(ref gpx_path) = workout.gpx_path else {
        return Ok((Vec::new(), false));
    };

    let path_suffix = gpx_path.trim_start_matches('/');

    let points = {
        let file = File::open(zip_path)?;
        let mut archive = ::zip::ZipArchive::new(file).context("Failed to read ZIP archive")?;
        let gpx_index = (0..archive.len()).find(|&i| {
            archive
                .by_index(i)
                .map(|e: ::zip::read::ZipFile<'_>| e.name().ends_with(path_suffix))
                .unwrap_or(false)
        });
        match gpx_index {
            None => return Ok((Vec::new(), false)),
            Some(idx) => {
                let entry = archive.by_index(idx).context("Failed to open GPX file")?;
                parse_gpx(entry)?
            }
        }
    };

    if points.is_empty() {
        return Ok((Vec::new(), false));
    }

    let workout_start = parse_datetime(&workout.start_time)?;
    let workout_end = parse_datetime(&workout.end_time)?;

    let hr_records = {
        let file = File::open(zip_path)?;
        let mut archive = ::zip::ZipArchive::new(file).context("Failed to read ZIP archive")?;
        let xml_index = (0..archive.len())
            .find(|&i| {
                archive
                    .by_index(i)
                    .map(|e: ::zip::read::ZipFile<'_>| e.name().ends_with("export.xml"))
                    .unwrap_or(false)
            })
            .context("export.xml not found")?;
        let entry = archive
            .by_index(xml_index)
            .context("Failed to open export.xml")?;
        collect_heart_rate(entry, workout_start, workout_end)?
    };

    let has_hr = !hr_records.is_empty();
    let splits = compute_splits(&points, &hr_records);
    Ok((splits, has_hr))
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
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"Workout" => {
                if let Some(w) = parse_workout_attrs(e, WorkoutChildData::default(), from, to)? {
                    workouts.push(w);
                }
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"Workout" => {
                let child_data = read_workout_children(&mut xml)?;
                if let Some(w) = parse_workout_attrs(e, child_data, from, to)? {
                    workouts.push(w);
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

/// Read children of a `<Workout>` element until `</Workout>`, collecting stats and metadata.
fn read_workout_children<R: Read>(xml: &mut Reader<BufReader<R>>) -> Result<WorkoutChildData> {
    let mut data = WorkoutChildData::default();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match xml.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"WorkoutStatistics" => {
                let mut stat_type: Option<String> = None;
                let mut sum: Option<f64> = None;
                let mut unit: Option<String> = None;
                for attr in e.attributes() {
                    let attr = attr.context("Invalid XML attribute")?;
                    let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                    let value = std::str::from_utf8(attr.value.as_ref())
                        .unwrap_or("")
                        .to_owned();
                    match key {
                        "type" => stat_type = Some(value),
                        "sum" => sum = value.parse::<f64>().ok(),
                        "unit" => unit = Some(value),
                        _ => {}
                    }
                }
                match stat_type.as_deref() {
                    Some("HKQuantityTypeIdentifierDistanceWalkingRunning") => {
                        data.distance_km = match (sum, unit.as_deref()) {
                            (Some(d), Some("km")) => Some(d),
                            (Some(d), Some("mi")) => Some(d * 1.60934),
                            (Some(d), _) => Some(d),
                            (None, _) => None,
                        };
                    }
                    Some("HKQuantityTypeIdentifierActiveEnergyBurned") => {
                        data.energy_kj = match (sum, unit.as_deref()) {
                            (Some(e), Some("kJ")) => Some(e),
                            (Some(e), Some("kcal")) => Some(e * 4.184),
                            (Some(e), _) => Some(e),
                            (None, _) => None,
                        };
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"MetadataEntry" => {
                let mut key: Option<String> = None;
                let mut value: Option<String> = None;
                for attr in e.attributes() {
                    let attr = attr.context("Invalid XML attribute")?;
                    let k = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                    let v = std::str::from_utf8(attr.value.as_ref())
                        .unwrap_or("")
                        .to_owned();
                    match k {
                        "key" => key = Some(v),
                        "value" => value = Some(v),
                        _ => {}
                    }
                }
                match (key.as_deref(), value.as_deref()) {
                    (Some("HKIndoorWorkout"), Some(v)) => data.indoor = Some(v == "1"),
                    (Some("HKWasUserEntered"), Some(v)) => data.user_entered = Some(v == "1"),
                    _ => {}
                }
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"WorkoutRoute" => {
                let mut route_buf = Vec::new();
                loop {
                    route_buf.clear();
                    match xml.read_event_into(&mut route_buf) {
                        Ok(Event::Empty(ref e2)) if e2.name().as_ref() == b"FileReference" => {
                            for attr in e2.attributes() {
                                let attr = attr.context("Invalid XML attribute")?;
                                if attr.key.as_ref() == b"path" {
                                    data.gpx_path = Some(
                                        std::str::from_utf8(attr.value.as_ref())
                                            .unwrap_or("")
                                            .to_owned(),
                                    );
                                }
                            }
                        }
                        Ok(Event::End(ref e2)) if e2.name().as_ref() == b"WorkoutRoute" => break,
                        Ok(Event::Eof) => break,
                        Err(e) => {
                            return Err(anyhow::anyhow!(
                                "XML parse error at position {}: {e}",
                                xml.error_position()
                            ));
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"Workout" => break,
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "XML parse error at position {}: {e}",
                    xml.error_position()
                ));
            }
            _ => {}
        }
    }
    Ok(data)
}

fn parse_workout_attrs(
    e: &quick_xml::events::BytesStart<'_>,
    child_data: WorkoutChildData,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<Option<RunningWorkout>> {
    let mut activity_type: Option<String> = None;
    let mut start_date: Option<String> = None;
    let mut end_date: Option<String> = None;
    let mut total_distance: Option<f64> = None;
    let mut distance_unit: Option<String> = None;
    let mut source_name: Option<String> = None;
    let mut device: Option<String> = None;

    for attr in e.attributes() {
        let attr = attr.context("Invalid XML attribute")?;
        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
        let value = attr
            .unescape_value()
            .with_context(|| format!("Invalid XML attribute value for `{key}` in Workout element"))?
            .into_owned();

        match key {
            "workoutActivityType" => activity_type = Some(value),
            "startDate" => start_date = Some(value),
            "endDate" => end_date = Some(value),
            "totalDistance" => total_distance = value.parse::<f64>().ok(),
            "totalDistanceUnit" => distance_unit = Some(value),
            "sourceName" => source_name = Some(value),
            "device" => device = Some(value),
            _ => {}
        }
    }

    if activity_type.as_deref() != Some("HKWorkoutActivityTypeRunning") {
        return Ok(None);
    }

    let start = match start_date {
        Some(s) => s,
        None => return Ok(None),
    };
    let end = match end_date {
        Some(e) => e,
        None => return Ok(None),
    };

    let workout_date = match NaiveDate::parse_from_str(date_part(&start), "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };
    if let Some(f) = from
        && workout_date < f
    {
        return Ok(None);
    }
    if let Some(t) = to
        && workout_date > t
    {
        return Ok(None);
    }

    let distance_km = if let Some(d) = child_data.distance_km {
        d
    } else {
        match (total_distance, distance_unit.as_deref()) {
            (Some(d), Some("km")) => d,
            (Some(d), Some("mi")) => d * 1.60934,
            (Some(d), _) => d,
            (None, _) => 0.0,
        }
    };

    Ok(Some(RunningWorkout {
        date: date_part(&start).to_string(),
        start_time: start,
        end_time: end,
        distance_km,
        energy_kj: child_data.energy_kj,
        source_name,
        device,
        indoor: child_data.indoor,
        user_entered: child_data.user_entered,
        gpx_path: child_data.gpx_path,
    }))
}

/// Parse a GPX file into a list of trackpoints.
fn parse_gpx<R: Read>(reader: R) -> Result<Vec<GpxPoint>> {
    let buf_reader = BufReader::new(reader);
    let mut xml = Reader::from_reader(buf_reader);
    xml.config_mut().trim_text(true);

    let mut points = Vec::new();
    let mut buf = Vec::new();
    let mut current_lat: Option<f64> = None;
    let mut current_lon: Option<f64> = None;
    let mut current_time: Option<DateTime<FixedOffset>> = None;
    let mut in_trkpt = false;
    let mut reading_time = false;

    loop {
        buf.clear();
        match xml.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"trkpt" => {
                    in_trkpt = true;
                    reading_time = false;
                    current_lat = None;
                    current_lon = None;
                    current_time = None;
                    for attr in e.attributes() {
                        let attr = attr.context("Invalid GPX attribute")?;
                        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                        let value = std::str::from_utf8(attr.value.as_ref()).unwrap_or("");
                        match key {
                            "lat" => current_lat = value.parse().ok(),
                            "lon" => current_lon = value.parse().ok(),
                            _ => {}
                        }
                    }
                }
                b"time" if in_trkpt => reading_time = true,
                _ => {}
            },
            Ok(Event::Text(ref e)) if reading_time => {
                if let Ok(text) = e.unescape() {
                    current_time = DateTime::parse_from_rfc3339(text.trim()).ok();
                }
                reading_time = false;
            }
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"trkpt" => {
                    if let (Some(lat), Some(lon), Some(time)) =
                        (current_lat, current_lon, current_time)
                    {
                        points.push(GpxPoint { lat, lon, time });
                    }
                    in_trkpt = false;
                    reading_time = false;
                }
                b"time" => reading_time = false,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("GPX parse error: {e}")),
            _ => {}
        }
    }

    Ok(points)
}

/// Scan export.xml and return HR records that overlap the workout time window.
fn collect_heart_rate<R: Read>(
    reader: R,
    workout_start: DateTime<FixedOffset>,
    workout_end: DateTime<FixedOffset>,
) -> Result<Vec<HrRecord>> {
    let buf_reader = BufReader::new(reader);
    let mut xml = Reader::from_reader(buf_reader);
    xml.config_mut().trim_text(true);

    let mut records = Vec::new();
    let mut buf = Vec::new();

    loop {
        buf.clear();
        match xml.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"Record" => {
                let mut is_hr = false;
                let mut bpm: Option<f64> = None;
                let mut start_str: Option<String> = None;
                let mut end_str: Option<String> = None;

                for attr in e.attributes() {
                    let attr = attr.context("Invalid XML attribute")?;
                    let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                    let value = std::str::from_utf8(attr.value.as_ref())
                        .unwrap_or("")
                        .to_owned();
                    match key {
                        "type" => {
                            is_hr = value == "HKQuantityTypeIdentifierHeartRate";
                        }
                        "value" => bpm = value.parse().ok(),
                        "startDate" => start_str = Some(value),
                        "endDate" => end_str = Some(value),
                        _ => {}
                    }
                }

                if is_hr
                    && let (Some(bpm), Some(start_s), Some(end_s)) = (bpm, start_str, end_str)
                    && let (Ok(start), Ok(end)) = (parse_datetime(&start_s), parse_datetime(&end_s))
                    && start < workout_end
                    && end > workout_start
                {
                    records.push(HrRecord { start, end, bpm });
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("XML parse error: {e}")),
            _ => {}
        }
    }

    Ok(records)
}

fn avg_hr_for_window(
    hr_records: &[HrRecord],
    window_start: DateTime<FixedOffset>,
    window_end: DateTime<FixedOffset>,
) -> Option<f64> {
    if hr_records.is_empty() {
        return None;
    }

    let (sum, count) = hr_records
        .iter()
        .filter(|hr| hr.start < window_end && hr.end > window_start)
        .fold((0.0_f64, 0usize), |(sum, count), hr| {
            (sum + hr.bpm, count + 1)
        });

    if count == 0 {
        None
    } else {
        Some(sum / count as f64)
    }
}

/// Compute per-kilometre splits from GPS trackpoints, optionally enriched with HR data.
fn compute_splits(points: &[GpxPoint], hr_records: &[HrRecord]) -> Vec<KmSplit> {
    if points.len() < 2 {
        return Vec::new();
    }

    let mut splits = Vec::new();
    let mut accumulated_km = 0.0_f64;
    let mut split_start_time = points[0].time;
    let mut km_number = 1_usize;

    for i in 1..points.len() {
        let previous_accumulated_km = accumulated_km;
        let segment_km = haversine_km(
            points[i - 1].lat,
            points[i - 1].lon,
            points[i].lat,
            points[i].lon,
        );
        accumulated_km += segment_km;
        let segment_ms = (points[i].time - points[i - 1].time).num_milliseconds();

        while accumulated_km >= km_number as f64 {
            let split_end_time = if segment_km > 0.0 {
                let fraction = (km_number as f64 - previous_accumulated_km) / segment_km;
                points[i - 1].time
                    + chrono::Duration::milliseconds(
                        (segment_ms as f64 * fraction.clamp(0.0, 1.0)) as i64,
                    )
            } else {
                points[i].time
            };
            let duration_secs = (split_end_time - split_start_time).num_seconds().max(0);
            let avg_hr = avg_hr_for_window(hr_records, split_start_time, split_end_time);

            splits.push(KmSplit {
                km: km_number,
                duration_secs,
                avg_hr,
                partial_km: None,
            });
            split_start_time = split_end_time;
            km_number += 1;
        }
    }

    let remaining_km = accumulated_km - (km_number - 1) as f64;
    if remaining_km > 0.01 {
        let last = points.last().expect("points length checked above");
        let duration_secs = (last.time - split_start_time).num_seconds().max(0);
        let avg_hr = avg_hr_for_window(hr_records, split_start_time, last.time);
        splits.push(KmSplit {
            km: km_number,
            duration_secs,
            avg_hr,
            partial_km: Some(remaining_km),
        });
    }

    splits
}

fn print_show<W: Write>(
    writer: &mut W,
    index: usize,
    workout: &RunningWorkout,
    splits: &[KmSplit],
    has_hr: bool,
) -> Result<()> {
    writeln!(writer, "Workout #{index}")?;
    writeln!(writer, "  Date:        {}", workout.date)?;
    writeln!(writer, "  Start:       {}", time_part(&workout.start_time))?;
    writeln!(writer, "  End:         {}", time_part(&workout.end_time))?;
    writeln!(writer, "  Duration:    {}", workout.duration())?;
    writeln!(writer, "  Distance:    {:.2} km", workout.distance_km)?;
    writeln!(writer, "  Pace:        {} (M:SS/km)", workout.pace())?;
    if let Some(kj) = workout.energy_kj {
        writeln!(writer, "  Energy:      {kj:.0} kJ")?;
    }
    if let Some(ref s) = workout.source_name {
        writeln!(writer, "  Source:      {s}")?;
    }
    if let Some(ref d) = workout.device {
        writeln!(writer, "  Device:      {d}")?;
    }
    match workout.indoor {
        Some(true) => writeln!(writer, "  Environment: Indoor")?,
        Some(false) => writeln!(writer, "  Environment: Outdoor")?,
        None => {}
    }
    if let Some(true) = workout.user_entered {
        writeln!(writer, "  Note:        Manually entered")?;
    }

    if splits.is_empty() {
        writeln!(writer, "\nNo GPS data available for splits.")?;
        return Ok(());
    }

    writeln!(writer)?;
    if has_hr {
        writeln!(writer, "  km | pace    | avg hr")?;
        writeln!(writer, "  ---+---------+-------")?;
    } else {
        writeln!(writer, "  km | pace")?;
        writeln!(writer, "  ---+--------")?;
    }
    for split in splits {
        let pace_secs = match split.partial_km {
            None => split.duration_secs,
            Some(km) if km > 0.0 => (split.duration_secs as f64 / km) as i64,
            _ => split.duration_secs,
        };
        let pace = format!("{}:{:02}", pace_secs / 60, pace_secs % 60);
        let km_label = match split.partial_km {
            None => format!("{:2}", split.km),
            Some(_) => format!("{:2}~", split.km),
        };
        if has_hr {
            let hr = split
                .avg_hr
                .map(|h| format!("{h:.0}"))
                .unwrap_or_else(|| "-".to_string());
            writeln!(writer, "  {} | {:<7}  | {}", km_label, pace, hr)?;
        } else {
            writeln!(writer, "  {} | {}", km_label, pace)?;
        }
    }

    Ok(())
}

/// Render the workouts as a Markdown table.
fn print_markdown_table<W: Write>(writer: &mut W, workouts: &[RunningWorkout]) -> Result<()> {
    let indices: Vec<String> = (1..=workouts.len()).map(|i| i.to_string()).collect();
    let dates: Vec<String> = workouts.iter().map(|w| w.date.clone()).collect();
    let starts: Vec<String> = workouts
        .iter()
        .map(|w| time_part(&w.start_time).to_string())
        .collect();
    let ends: Vec<String> = workouts
        .iter()
        .map(|w| time_part(&w.end_time).to_string())
        .collect();
    let durations: Vec<String> = workouts.iter().map(|w| w.duration()).collect();
    let distances: Vec<f64> = workouts.iter().map(|w| w.distance_km).collect();
    let paces: Vec<String> = workouts.iter().map(|w| w.pace()).collect();

    let df = DataFrame::new(vec![
        Column::new("#".into(), indices),
        Column::new("Date".into(), dates),
        Column::new("Start".into(), starts),
        Column::new("End".into(), ends),
        Column::new("Duration (M:SS)".into(), durations),
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

    let header_row = headers
        .iter()
        .zip(&col_widths)
        .map(|(h, w)| format!(" {h:<w$} "))
        .collect::<Vec<_>>()
        .join("|");
    writeln!(writer, "|{header_row}|")?;

    let sep_row = col_widths
        .iter()
        .map(|w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("|");
    writeln!(writer, "|{sep_row}|")?;

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
        writeln!(writer, "|{data_row}|")?;
    }

    Ok(())
}

fn cell_str(df: &DataFrame, row: usize, col: usize) -> String {
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
