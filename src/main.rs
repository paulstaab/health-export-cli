use anyhow::Result;

fn main() -> Result<()> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let exit_code =
        health_export_cli::run_from_args(std::env::args_os(), &mut stdout, &mut stderr)?;
    std::process::exit(exit_code);
}
