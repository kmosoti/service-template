#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use engine_service::{render_compatibility_json, run_stdio_batch_files, self_check};

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         admission-engine --print-compatibility\n  \
         admission-engine --self-check\n  \
         admission-engine --run-stdio-batch [--config <path>] [--state-in <path>] [--state-out <path>]"
    );
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let Some(mode) = args.next() else {
        print_usage();
        return Err("missing required mode flag".to_owned());
    };

    match mode.as_str() {
        "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        "--print-compatibility" => {
            if args.next().is_some() {
                return Err("--print-compatibility accepts no extra arguments".to_owned());
            }
            let json = render_compatibility_json().map_err(|e| e.to_string())?;
            print!("{json}");
            Ok(())
        }
        "--self-check" => {
            if args.next().is_some() {
                return Err("--self-check accepts no extra arguments".to_owned());
            }
            self_check().map_err(|e| e.to_string())?;
            println!("ok: admission-engine self-check passed");
            Ok(())
        }
        "--run-stdio-batch" => {
            let mut config_path: Option<PathBuf> = None;
            let mut state_in: Option<PathBuf> = None;
            let mut state_out: Option<PathBuf> = None;

            while let Some(flag) = args.next() {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing path value for {flag}"))?;
                match flag.as_str() {
                    "--config" => config_path = Some(PathBuf::from(value)),
                    "--state-in" => state_in = Some(PathBuf::from(value)),
                    "--state-out" => state_out = Some(PathBuf::from(value)),
                    other => return Err(format!("unknown option for --run-stdio-batch: {other}")),
                }
            }

            run_stdio_batch_files(
                config_path.as_deref(),
                state_in.as_deref(),
                state_out.as_deref(),
            )
            .map_err(|e| e.to_string())
        }
        other => {
            print_usage();
            Err(format!("unknown flag: {other}"))
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
