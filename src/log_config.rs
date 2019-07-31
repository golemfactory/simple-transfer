use flexi_logger::Duplicate;
use log::Level;
use std::path::{Path};

fn log_string_for_level(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Info => "info,sentry=trace",
        Level::Debug => "hyperg=debug,info",
        Level::Trace => "hyperg=trace,info",
        Level::Warn => "hyperg=info,warn",
    }
}

fn is_dir_path(p: &Path) -> bool {
    p.to_str()
        .and_then(|s| s.chars().rev().next())
        .map(|ch| ch == std::path::MAIN_SEPARATOR)
        .unwrap_or(false)
}

fn detailed_format(
    w: &mut dyn std::io::Write,
    now: &mut flexi_logger::DeferredNow,
    record: &flexi_logger::Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "{} {} {} {}",
        now.now().format("%Y-%m-%d %H:%M:%S"),
        record.level(),
        record.module_path().unwrap_or("<unnamed>"),
        &record.args()
    )
}

pub fn init(log_level: Level, log_path: Option<&Path>) {
    let log_builder = flexi_logger::Logger::with_env_or_str(log_string_for_level(log_level));

    if let Some(logfile) = log_path {
        let log_builder = if is_dir_path(logfile) {
            log_builder.directory(logfile)
        } else {
            match (logfile.file_name(), logfile.parent()) {
                (Some(_file_name), Some(dir_name)) if dir_name.is_dir() => {
                    log_builder.directory(dir_name).create_symlink(logfile)
                }
                _ => log_builder.create_symlink(logfile),
            }
        };
        log_builder
            .log_to_file()
            .duplicate_to_stderr(Duplicate::Info)
            .format_for_files(detailed_format)
            .start()
            .unwrap_or_else(|e| {
                eprintln!("Error {}", e);
                // fallback to stderr only logger.
                flexi_logger::Logger::with_env_or_str(log_string_for_level(log_level))
                    .start()
                    .unwrap()
            });
    } else {
        log_builder.start().unwrap();
    }
}
