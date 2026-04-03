use health_export_cli::run_from_args;
use std::fs;
use std::path::PathBuf;

fn fixture_zip() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/export.zip")
}

fn run_app(args: &[&str]) -> (i32, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let mut all_args = vec!["health-export-cli"];
    all_args.extend_from_slice(args);

    let exit_code = run_from_args(all_args, &mut stdout, &mut stderr).unwrap();
    (
        exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

struct FileCleanup {
    path: PathBuf,
    backup_path: Option<PathBuf>,
}

impl Drop for FileCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(backup_path) = &self.backup_path {
            let _ = fs::rename(backup_path, &self.path);
        }
    }
}

fn install_default_fixture_zip() -> FileCleanup {
    let destination = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("export.zip");
    let backup_path = if destination.exists() {
        let backup = destination.with_extension("zip.test-backup");
        if backup.exists() {
            fs::remove_file(&backup).unwrap();
        }
        fs::rename(&destination, &backup).unwrap();
        Some(backup)
    } else {
        None
    };

    if let Err(err) = fs::copy(fixture_zip(), &destination) {
        if let Some(backup_path) = &backup_path {
            let _ = fs::rename(backup_path, &destination);
        }
        panic!("failed to install fixture export.zip: {err}");
    }

    FileCleanup {
        path: destination,
        backup_path,
    }
}

#[test]
fn tc_cli_001_list_orders_workouts_chronologically() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "list"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| Run ID | Date       | Start | End   | Duration (min) | Distance (km) | Pace (min/km) |"
    ));
    assert!(stdout.contains(
        "| 1      | 2023-12-28 | 06:30 | 06:42 | 12:30          | 2.50          | 5:00          |"
    ));
    assert!(stdout.contains(
        "| 2      | 2024-01-05 | 07:00 | 07:30 | 30:00          | 5.00          | 6:00          |"
    ));
    assert!(stdout.contains(
        "| 5      | 2025-03-01 | 09:00 | 09:55 | 55:00          | 10.00         | 5:30          |"
    ));
    assert!(stdout.contains(
        "| 7      | 2025-05-04 | 06:15 | 10:15 | 240:00         | 42.20         | 5:41          |"
    ));
    assert!(!stdout.contains("2024-04-01"));
}

#[test]
fn tc_cli_002_filters_by_year() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "list",
        "--year",
        "2024",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| 2      | 2024-01-05 | 07:00 | 07:30 | 30:00          | 5.00          | 6:00          |"
    ));
    assert!(stdout.contains(
        "| 3      | 2024-06-10 | 18:15 | 18:45 | 30:00          | 6.00          | 5:00          |"
    ));
    assert!(!stdout.contains("2023-12-28"));
    assert!(!stdout.contains("2025-02-01"));
}

#[test]
fn tc_cli_003_filters_by_date_range() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "list",
        "--from",
        "2025-02-01",
        "--to",
        "2025-02-28",
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
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "list",
        "--year",
        "2022",
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
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "show",
        "999",
    ]);

    assert_eq!(exit_code, 1);
    assert_eq!(stdout, "");
    assert!(stderr.contains("Run ID 999 is out of range. Valid range: 1–7"));
}

#[test]
fn tc_cli_006_show_latest_selects_most_recent_workout() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "show",
        "latest",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Workout #7"));
    assert!(stdout.contains("Date:        2025-05-04"));
    assert!(stdout.contains("Pace:        5:41 (M:SS/km)"));
}

#[test]
fn tc_cli_006_show_uses_global_run_ids_without_filters() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "show", "3"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Workout #3"));
    assert!(stdout.contains("Date:        2024-06-10"));
}

#[test]
fn tc_cli_011_show_rejects_filter_flags() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "show",
        "--year",
        "2024",
        "4",
    ]);

    assert_eq!(exit_code, 2);
    assert_eq!(stdout, "");
    assert!(stderr.contains("unexpected argument '--year' found"));
}

#[test]
fn tc_cli_007_records_renders_expected_rows() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "records"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| Record Type           | Run ID | Date       | Duration (min) | Pace (min/km) | Distance (km) |"
    ));
    assert!(stdout.contains(
        "| Longest Run           | 7      | 2025-05-04 | 240:00         | 5:41          | 42.20         |"
    ));
    assert!(stdout.contains(
        "| Fastest 5k            | 3      | 2024-06-10 | 30:00          | 5:00          | 6.00          |"
    ));
    assert!(stdout.contains(
        "| Fastest 10k           | 5      | 2025-03-01 | 55:00          | 5:30          | 10.00         |"
    ));
    assert!(stdout.contains(
        "| Fastest Half Marathon | 6      | 2025-04-06 | 118:00         | 5:35          | 21.10         |"
    ));
    assert!(stdout.contains(
        "| Fastest Marathon      | 7      | 2025-05-04 | 240:00         | 5:41          | 42.20         |"
    ));
}

#[test]
fn tc_cli_008_records_keep_global_run_ids_under_filters() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "records",
        "--year",
        "2024",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| Longest Run           | 3      | 2024-06-10 | 30:00          | 5:00          | 6.00          |"
    ));
    assert!(stdout.contains(
        "| Fastest 5k            | 3      | 2024-06-10 | 30:00          | 5:00          | 6.00          |"
    ));
    assert!(stdout.contains(
        "| Fastest 10k           | -      | -          | -              | -             | -             |"
    ));
}

#[test]
fn tc_cli_009_records_render_all_placeholder_rows_when_filter_is_empty() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "records",
        "--year",
        "2022",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| Longest Run           | -      | -    | -              | -             | -             |"
    ));
    assert!(stdout.contains(
        "| Fastest Marathon      | -      | -    | -              | -             | -             |"
    ));
}

#[test]
fn tc_cli_012_defaults_file_to_export_zip_in_current_directory() {
    let _cleanup = install_default_fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&["running", "records"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| Longest Run           | 7      | 2025-05-04 | 240:00         | 5:41          | 42.20         |"
    ));
}

#[test]
fn tc_cli_010_records_select_long_distance_winners() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "records",
        "--year",
        "2025",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| Fastest 10k           | 5      | 2025-03-01 | 55:00          | 5:30          | 10.00         |"
    ));
    assert!(stdout.contains(
        "| Fastest Half Marathon | 6      | 2025-04-06 | 118:00         | 5:35          | 21.10         |"
    ));
    assert!(stdout.contains(
        "| Fastest Marathon      | 7      | 2025-05-04 | 240:00         | 5:41          | 42.20         |"
    ));
}

#[test]
fn tc_parse_001_show_uses_child_stats_metadata_and_unescaped_attributes() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "show", "1"]);

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
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "show", "2"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Distance:    5.00 km"));
    assert!(stdout.contains("No GPS data available for splits."));
}

#[test]
fn tc_parse_003_show_converts_kcal_and_reports_manual_indoor_workout() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "show", "4"]);

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
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "show", "1"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Split (km) | Pace (min/km) | Avg HR (bpm)"));
    assert!(stdout.contains(" 1         | 4:59          | 151"));
    assert!(stdout.contains(" 2         | 4:59          | 158"));
    assert!(stdout.contains(" 3~        | 4:58          | 166"));
}

#[test]
fn tc_comp_002_show_omits_hr_column_when_no_hr_exists() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "show", "3"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("Split (km) | Pace (min/km)"));
    assert!(!stdout.contains("avg hr"));
    assert!(stdout.contains(" 6         | 4:59"));
}

#[test]
fn tc_comp_003_records_break_pace_ties_by_earliest_run() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) =
        run_app(&["--file", zip_path.to_str().unwrap(), "running", "records"]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains(
        "| Fastest 5k            | 3      | 2024-06-10 | 30:00          | 5:00          | 6.00          |"
    ));
    assert!(!stdout.contains(
        "| Fastest 5k            | 4      | 2025-02-01 | 25:00          | 5:00          | 5.00          |"
    ));
}

#[test]
fn tc_cli_013_list_filters_by_month_within_year() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "list",
        "--year",
        "2025",
        "--month",
        "2",
    ]);

    assert_eq!(exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("2025-02-01"));
    assert!(!stdout.contains("2025-03-01"));
    assert!(!stdout.contains("2025-04-06"));
}

#[test]
fn tc_cli_014_records_require_year_when_month_is_present() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "records",
        "--month",
        "2",
    ]);

    assert_eq!(exit_code, 2);
    assert_eq!(stdout, "");
    assert!(stderr.contains("the following required arguments were not provided"));
    assert!(stderr.contains("--year <YEAR>"));
}

#[test]
fn tc_cli_015_list_rejects_month_out_of_range() {
    let zip_path = fixture_zip();
    let (exit_code, stdout, stderr) = run_app(&[
        "--file",
        zip_path.to_str().unwrap(),
        "running",
        "list",
        "--year",
        "2025",
        "--month",
        "13",
    ]);

    assert_eq!(exit_code, 2);
    assert_eq!(stdout, "");
    assert!(stderr.contains("invalid value '13' for '--month <MONTH>'"));
}
