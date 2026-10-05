use tracing_subscriber::filter::LevelFilter;

pub fn init() {
    let level = match std::env::var("LOG_LEVEL").as_deref() {
        Ok("off") => LevelFilter::OFF,
        Ok("error") => LevelFilter::ERROR,
        Ok("warn") => LevelFilter::WARN,
        Ok("debug") => LevelFilter::DEBUG,
        Ok("trace") => LevelFilter::TRACE,
        _ => LevelFilter::INFO,
    };
    tracing_subscriber::fmt().with_max_level(level).init();
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    const MARKERS: [&str; 5] = [
        "log-probe-error",
        "log-probe-warn",
        "log-probe-info",
        "log-probe-debug",
        "log-probe-trace",
    ];

    #[test]
    #[ignore = "executed in a child process to isolate the global subscriber and environment"]
    fn emit_logs() {
        super::init();
        tracing::error!("log-probe-error");
        tracing::warn!("log-probe-warn");
        tracing::info!("log-probe-info");
        tracing::debug!("log-probe-debug");
        tracing::trace!("log-probe-trace");
    }

    #[test]
    fn initialized_logger_obeys_log_level() {
        let cases = [
            (None, 3),
            (Some("off"), 0),
            (Some("error"), 1),
            (Some("warn"), 2),
            (Some("info"), 3),
            (Some("debug"), 4),
            (Some("trace"), 5),
            (Some(""), 3),
            (Some("invalid"), 3),
            (Some("DEBUG"), 3),
            (Some("Trace"), 3),
            (Some(" debug"), 3),
            (Some("debug "), 3),
            (Some("trace\n"), 3),
            (Some("0"), 3),
            (Some("debug,tower_http=trace"), 3),
            (Some("tower_http=debug"), 3),
        ];
        for (level, emitted) in cases {
            for legacy in ["off", "trace"] {
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "logging::tests::emit_logs",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env_remove("LOG_LEVEL")
                    .env("RUST_LOG", legacy)
                    .env("AGENT_TALK_LOG_LEVEL", legacy);
                if let Some(level) = level {
                    command.env("LOG_LEVEL", level);
                }
                let output = command.output().unwrap();
                assert!(output.status.success(), "{output:?}");
                let logs = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
                for (index, marker) in MARKERS.iter().enumerate() {
                    assert_eq!(
                        logs.contains(marker),
                        index < emitted,
                        "LOG_LEVEL={level:?}, legacy={legacy}, marker={marker}: {logs}",
                    );
                }
            }
        }
    }
}
