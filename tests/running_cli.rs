use health_data_parser::run_from_args;
use std::path::PathBuf;

fn fixture_zip() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/export.zip")
}

fn run_app(args: &[&str]) -> (i32, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let mut all_args = vec!["health-data-parser"];
    all_args.extend_from_slice(args);

    let exit_code = run_from_args(all_args, &mut stdout, &mut stderr).unwrap();
    (
        exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn tc_cli_001_list_orders_workouts_chronologically() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["running", "--file", zip_path.to_str().unwrap(), "list"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| # | Date       | Start | End   | Duration (M:SS) | Distance (km) | Pace (min/km) |"
    ));
    assert!(stdout.contains(
        "| 1 | 2023-12-28 | 06:30 | 06:42 | 12:30           | 2.50          | 5:00          |"
    ));
    assert!(stdout.contains(
        "| 2 | 2024-01-05 | 07:00 | 07:30 | 30:00           | 5.00          | 6:00          |"
    ));
    assert!(stdout.contains(
        "| 5 | 2025-03-01 | 09:00 | 09:55 | 55:00           | 10.00         | 5:30          |"
    ));
    assert!(!stdout.contains("2024-04-01"));
}

#[test]
fn tc_cli_002_filters_by_year() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "running",
        "--file",
        zip_path.to_str().unwrap(),
        "--year",
        "2024",
        "list",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("2024-01-05"));
    assert!(stdout.contains("2024-06-10"));
    assert!(!stdout.contains("2023-12-28"));
    assert!(!stdout.contains("2025-02-01"));
}

#[test]
fn tc_cli_003_filters_by_date_range() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "running",
        "--file",
        zip_path.to_str().unwrap(),
        "--from",
        "2025-02-01",
        "--to",
        "2025-02-28",
        "list",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("2025-02-01"));
    assert!(!stdout.contains("2025-03-01"));
    assert!(!stdout.contains("2024-06-10"));
}

#[test]
fn tc_cli_004_reports_no_matches() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "running",
        "--file",
        zip_path.to_str().unwrap(),
        "--year",
        "2022",
        "list",
    ]);

    assert_eq!(exit_code, 0);
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "No running workouts found for the given time range.\n"
    );
}

#[test]
fn tc_cli_005_show_errors_for_out_of_range_index() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "running",
        "--file",
        zip_path.to_str().unwrap(),
        "show",
        "999",
    ]);

    assert_eq!(exit_code, 1);
    assert_eq!(stdout, "");
    assert!(stderr.contains("Index 999 is out of range. Valid range: 1–5"));
}

#[test]
fn tc_cli_006_show_latest_selects_most_recent_workout() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "running",
        "--file",
        zip_path.to_str().unwrap(),
        "show",
        "latest",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Workout #5"));
    assert!(stdout.contains("Date:        2025-03-01"));
    assert!(stdout.contains("Pace:        5:30 (M:SS/km)"));
}

#[test]
fn tc_parse_001_show_uses_child_stats_metadata_and_unescaped_attributes() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["running", "--file", zip_path.to_str().unwrap(), "show", "1"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Distance:    2.50 km"));
    assert!(stdout.contains("Energy:      900 kJ"));
    assert!(stdout.contains("Source:      Synthetic & Run Club"));
    assert!(stdout.contains("Device:      <Watch6,4>"));
    assert!(stdout.contains("Environment: Outdoor"));
}

#[test]
fn tc_parse_002_show_reports_no_gps_for_self_closing_workout() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["running", "--file", zip_path.to_str().unwrap(), "show", "2"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Distance:    5.00 km"));
    assert!(stdout.contains("No GPS data available for splits."));
}

#[test]
fn tc_parse_003_show_converts_kcal_and_reports_manual_indoor_workout() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["running", "--file", zip_path.to_str().unwrap(), "show", "4"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Energy:      418 kJ"));
    assert!(stdout.contains("Environment: Indoor"));
    assert!(stdout.contains("Note:        Manually entered"));
}

#[test]
fn tc_comp_001_show_renders_hr_splits_and_partial_final_km() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["running", "--file", zip_path.to_str().unwrap(), "show", "1"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("km | pace    | avg hr"));
    assert!(stdout.contains("  1 | 4:59     | 151"));
    assert!(stdout.contains("  2 | 4:59     | 158"));
    assert!(stdout.contains("  3~ | 4:58     | 166"));
}

#[test]
fn tc_comp_002_show_omits_hr_column_when_no_hr_exists() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["running", "--file", zip_path.to_str().unwrap(), "show", "3"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("km | pace"));
    assert!(!stdout.contains("avg hr"));
    assert!(stdout.contains("  6 | 4:59"));
}
