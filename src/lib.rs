use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate};
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
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
    /// Path to the Apple Health export ZIP file.
    #[arg(short, long, global = true, default_value = "./export.zip")]
    file: PathBuf,

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
    #[command(subcommand)]
    subcommand: RunningSubcommand,
}

#[derive(Args, Debug)]
struct FilterArgs {
    /// Filter by year (e.g. 2024). Cannot be used together with --from/--to.
    #[arg(long, conflicts_with_all = ["from", "to"])]
    year: Option<i32>,

    /// Filter by month number (1-12). Requires --year.
    #[arg(long, requires = "year", value_parser = clap::value_parser!(u32).range(1..=12))]
    month: Option<u32>,

    /// Start of time range (inclusive), format: YYYY-MM-DD. Requires --to.
    #[arg(long, requires = "to")]
    from: Option<NaiveDate>,

    /// End of time range (inclusive), format: YYYY-MM-DD. Requires --from.
    #[arg(long, requires = "from")]
    to: Option<NaiveDate>,
}

#[derive(Subcommand, Debug)]
enum RunningSubcommand {
    /// List all running workouts as a table
    List(FilterArgs),
    /// Show running records for longest run and fastest qualifying distances.
    Records(FilterArgs),
    /// Show details for a workout. Pass a 1-based index or "latest".
    Show {
        /// Global 1-based run ID from the list output, or "latest" for the most recent workout
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
#[derive(Clone, Default)]
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

struct IndexedWorkout<'a> {
    global_index: usize,
    workout: &'a RunningWorkout,
}

struct RecordRow {
    record_type: &'static str,
    run_id: String,
    date: String,
    total_duration: String,
    pace: String,
    total_distance: String,
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
    let Cli { file, command } = cli;
    let Commands::Running(running) = command;
    match &running.subcommand {
        RunningSubcommand::List(filters) => {
            let all_workouts = load_all_running_workouts(&file)?;
            let (from_date, to_date) = resolve_date_filter(filters)?;
            let workouts = filter_workouts(&all_workouts, from_date, to_date);
            if workouts.is_empty() {
                writeln!(
                    stdout,
                    "No running workouts found for the given time range."
                )?;
                return Ok(());
            }
            print_markdown_table(stdout, &workouts)?;
        }
        RunningSubcommand::Records(filters) => {
            let all_workouts = load_all_running_workouts(&file)?;
            let (from_date, to_date) = resolve_date_filter(filters)?;
            let filtered_workouts = filter_workouts(&all_workouts, from_date, to_date);
            let record_rows = build_record_rows(&filtered_workouts);
            print_records_table(stdout, &record_rows)?;
        }
        RunningSubcommand::Show { target } => {
            let all_workouts = load_all_running_workouts(&file)?;
            let workout = match target {
                ShowTarget::Latest => IndexedWorkout {
                    global_index: all_workouts.len(),
                    workout: all_workouts.last().context("No running workouts found.")?,
                },
                ShowTarget::Index(i) => {
                    if *i == 0 || *i > all_workouts.len() {
                        anyhow::bail!(
                            "Run ID {i} is out of range. Valid range: 1–{}",
                            all_workouts.len()
                        );
                    }
                    IndexedWorkout {
                        global_index: *i,
                        workout: &all_workouts[*i - 1],
                    }
                }
            };
            show_workout(stdout, workout.global_index, workout.workout, &file)?;
        }
    }

    Ok(())
}

fn load_all_running_workouts(file_path: &PathBuf) -> Result<Vec<RunningWorkout>> {
    load_running_workouts_in_range(file_path, None, None)
}

fn load_running_workouts_in_range(
    file_path: &PathBuf,
    from_date: Option<NaiveDate>,
    to_date: Option<NaiveDate>,
) -> Result<Vec<RunningWorkout>> {
    let file = File::open(file_path)
        .with_context(|| format!("Cannot open file: {}", file_path.display()))?;
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

fn resolve_date_filter(filters: &FilterArgs) -> Result<(Option<NaiveDate>, Option<NaiveDate>)> {
    if let Some(year) = filters.year {
        if let Some(month) = filters.month {
            let from = NaiveDate::from_ymd_opt(year, month, 1).with_context(|| {
                format!("Invalid --year/--month combination: {year}-{month:02}")
            })?;
            let (next_year, next_month) = if month == 12 {
                (
                    year.checked_add(1)
                        .with_context(|| format!("Year out of range for --month 12: {year}"))?,
                    1,
                )
            } else {
                (year, month + 1)
            };
            let to = NaiveDate::from_ymd_opt(next_year, next_month, 1)
                .with_context(|| {
                    format!(
                        "Invalid next month while resolving date range: {next_year}-{next_month:02}"
                    )
                })?
                .pred_opt()
                .context("Failed to resolve month end")?;
            Ok((Some(from), Some(to)))
        } else {
            let from = NaiveDate::from_ymd_opt(year, 1, 1)
                .with_context(|| format!("Invalid --year value: {year}"))?;
            let to = NaiveDate::from_ymd_opt(year, 12, 31)
                .with_context(|| format!("Invalid --year value: {year}"))?;
            Ok((Some(from), Some(to)))
        }
    } else {
        Ok((filters.from, filters.to))
    }
}

fn filter_workouts<'a>(
    workouts: &'a [RunningWorkout],
    from_date: Option<NaiveDate>,
    to_date: Option<NaiveDate>,
) -> Vec<IndexedWorkout<'a>> {
    workouts
        .iter()
        .enumerate()
        .filter(|(_, workout)| workout_matches_date_filter(workout, from_date, to_date))
        .map(|(index, workout)| IndexedWorkout {
            global_index: index + 1,
            workout,
        })
        .collect()
}

fn workout_matches_date_filter(
    workout: &RunningWorkout,
    from_date: Option<NaiveDate>,
    to_date: Option<NaiveDate>,
) -> bool {
    let Ok(workout_date) = NaiveDate::parse_from_str(&workout.date, "%Y-%m-%d") else {
        return false;
    };

    if let Some(from) = from_date
        && workout_date < from
    {
        return false;
    }
    if let Some(to) = to_date
        && workout_date > to
    {
        return false;
    }

    true
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

fn build_record_rows(workouts: &[IndexedWorkout<'_>]) -> Vec<RecordRow> {
    vec![
        longest_run_record(workouts),
        fastest_distance_record("Fastest 5k", workouts, 5.0),
        fastest_distance_record("Fastest 10k", workouts, 10.0),
        fastest_distance_record("Fastest Half Marathon", workouts, 21.0975),
        fastest_distance_record("Fastest Marathon", workouts, 42.195),
    ]
}

fn longest_run_record(workouts: &[IndexedWorkout<'_>]) -> RecordRow {
    let mut best: Option<&IndexedWorkout<'_>> = None;

    for workout in workouts {
        if best.is_none_or(|current| workout.workout.distance_km > current.workout.distance_km) {
            best = Some(workout);
        }
    }

    record_row_from_workout("Longest Run", best)
}

fn fastest_distance_record(
    record_type: &'static str,
    workouts: &[IndexedWorkout<'_>],
    min_distance_km: f64,
) -> RecordRow {
    let mut best: Option<(&IndexedWorkout<'_>, f64)> = None;

    for workout in workouts {
        if workout.workout.distance_km + f64::EPSILON < min_distance_km {
            continue;
        }

        let Some(avg_pace_secs) = average_pace_seconds(workout.workout) else {
            continue;
        };

        if best.is_none_or(|(_, current_pace)| avg_pace_secs < current_pace) {
            best = Some((workout, avg_pace_secs));
        }
    }

    record_row_from_workout(record_type, best.map(|(workout, _)| workout))
}

fn average_pace_seconds(workout: &RunningWorkout) -> Option<f64> {
    if workout.distance_km <= 0.0 {
        return None;
    }

    let (Ok(start), Ok(end)) = (
        parse_datetime(&workout.start_time),
        parse_datetime(&workout.end_time),
    ) else {
        return None;
    };

    let duration_secs = (end - start).num_seconds();
    if duration_secs <= 0 {
        return None;
    }

    Some(duration_secs as f64 / workout.distance_km)
}

fn record_row_from_workout(
    record_type: &'static str,
    workout: Option<&IndexedWorkout<'_>>,
) -> RecordRow {
    let Some(workout) = workout else {
        return RecordRow {
            record_type,
            run_id: "-".to_string(),
            date: "-".to_string(),
            total_duration: "-".to_string(),
            pace: "-".to_string(),
            total_distance: "-".to_string(),
        };
    };

    let pace = workout.workout.pace();

    RecordRow {
        record_type,
        run_id: workout.global_index.to_string(),
        date: workout.workout.date.clone(),
        total_duration: workout.workout.duration(),
        pace,
        total_distance: format!("{:.2}", workout.workout.distance_km),
    }
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
        writeln!(writer, "  Split (km) | Pace (min/km) | Avg HR (bpm)")?;
        writeln!(writer, "  ----------+---------------+-------------")?;
    } else {
        writeln!(writer, "  Split (km) | Pace (min/km)")?;
        writeln!(writer, "  ----------+---------------")?;
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
            writeln!(writer, "  {:<10} | {:<13} | {}", km_label, pace, hr)?;
        } else {
            writeln!(writer, "  {:<10} | {}", km_label, pace)?;
        }
    }

    Ok(())
}

/// Render the workouts as a Markdown table.
fn print_markdown_table<W: Write>(writer: &mut W, workouts: &[IndexedWorkout<'_>]) -> Result<()> {
    let headers = [
        "Run ID",
        "Date",
        "Start",
        "End",
        "Duration (min)",
        "Distance (km)",
        "Pace (min/km)",
    ];
    let rows = workouts
        .iter()
        .map(|workout| {
            vec![
                workout.global_index.to_string(),
                workout.workout.date.clone(),
                time_part(&workout.workout.start_time).to_string(),
                time_part(&workout.workout.end_time).to_string(),
                workout.workout.duration(),
                format!("{:.2}", workout.workout.distance_km),
                workout.workout.pace(),
            ]
        })
        .collect::<Vec<_>>();

    let col_widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(column_index, header)| {
            let max_data = rows
                .iter()
                .map(|row| row[column_index].len())
                .max()
                .unwrap_or(0);
            max_data.max(header.len())
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

    for row in rows {
        let data_row = row
            .iter()
            .zip(&col_widths)
            .map(|(value, width)| format!(" {value:<width$} "))
            .collect::<Vec<_>>()
            .join("|");
        writeln!(writer, "|{data_row}|")?;
    }

    Ok(())
}

fn print_records_table<W: Write>(writer: &mut W, rows: &[RecordRow]) -> Result<()> {
    let headers = [
        "Record Type",
        "Run ID",
        "Date",
        "Duration (min)",
        "Pace (min/km)",
        "Distance (km)",
    ];
    let data = rows
        .iter()
        .map(|row| {
            vec![
                row.record_type.to_string(),
                row.run_id.clone(),
                row.date.clone(),
                row.total_duration.clone(),
                row.pace.clone(),
                row.total_distance.clone(),
            ]
        })
        .collect::<Vec<_>>();

    let col_widths = headers
        .iter()
        .enumerate()
        .map(|(column_index, header)| {
            let max_data = data
                .iter()
                .map(|row| row[column_index].len())
                .max()
                .unwrap_or(0);
            max_data.max(header.len())
        })
        .collect::<Vec<_>>();

    let header_row = headers
        .iter()
        .zip(&col_widths)
        .map(|(header, width)| format!(" {header:<width$} "))
        .collect::<Vec<_>>()
        .join("|");
    writeln!(writer, "|{header_row}|")?;

    let separator_row = col_widths
        .iter()
        .map(|width| "-".repeat(width + 2))
        .collect::<Vec<_>>()
        .join("|");
    writeln!(writer, "|{separator_row}|")?;

    for row in data {
        let data_row = row
            .iter()
            .zip(&col_widths)
            .map(|(value, width)| format!(" {value:<width$} "))
            .collect::<Vec<_>>()
            .join("|");
        writeln!(writer, "|{data_row}|")?;
    }

    Ok(())
}
