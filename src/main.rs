use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};
use tepra_cli::{
    discovery,
    error::{AppError, Result},
    job,
    render::{self, Dither, Fit, RenderOptions},
    session::{PrintSession, state_directory},
    webapi::WebApi,
};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    Direct,
    WindowsWebapi,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Transport {
    Usb,
    Ble,
}
impl Transport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Usb => "usb",
            Self::Ble => "ble",
        }
    }
}

#[derive(Parser)]
#[command(
    name = "tepra",
    version,
    about = "Prepare images and diagnose TEPRA SR-MK2 connections"
)]
struct Cli {
    /// Emit one JSON response on stdout (progress/errors never interleave).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// List USB/BLE devices, or printers registered with the Windows WebAPI.
    Devices {
        #[arg(long, value_enum, default_value = "direct")]
        backend: Backend,
        #[arg(long, value_enum, default_value = "usb")]
        transport: Transport,
        #[arg(long, default_value_t=3, value_parser=clap::value_parser!(u64).range(1..=30))]
        scan_seconds: u64,
    },
    /// Read device descriptors/GATT structure without sending printer commands.
    Doctor {
        #[arg(long)]
        device: String,
        #[arg(long, default_value_t=3, value_parser=clap::value_parser!(u64).range(1..=30))]
        scan_seconds: u64,
    },
    /// Convert PNG to a 360 dpi monochrome preview and a validated job.
    Render {
        input: PathBuf,
        #[arg(long, default_value = "sr-mk2")]
        model: String,
        #[arg(long, default_value_t = 24)]
        tape_width: u32,
        #[arg(long)]
        content_length_mm: f64,
        /// Append white space on the right after rendering, without resizing the artwork.
        #[arg(long, default_value_t = 0.0)]
        padding_right_mm: f64,
        #[arg(long, value_enum, default_value = "contain")]
        fit: Fit,
        #[arg(long, value_enum, default_value = "threshold")]
        dither: Dither,
        #[arg(long, default_value_t = 128)]
        threshold: u8,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        job: PathBuf,
    },
    /// Get printer status through a verified backend.
    Status {
        #[arg(long)]
        device: String,
        #[arg(long, value_enum, default_value = "direct")]
        backend: Backend,
    },
    /// Print a saved job using the official WebAPI or experimental USB backend.
    Print {
        #[arg(long)]
        job: PathBuf,
        #[arg(long)]
        device: String,
        #[arg(long, value_enum, default_value = "direct")]
        backend: Backend,
        #[arg(long, default_value_t = 1)]
        copies: u32,
        /// Validate the job offline; never opens a connection or creates an attempt.
        #[arg(long)]
        dry_run: bool,
        /// Use the SR-MK2 USB prototype (observed 24mm cartridge, one copy, <=100mm).
        #[arg(long)]
        experimental_usb: bool,
        /// USB: entire operation. WebAPI: submission/completion (preflight: 5s/request).
        #[arg(long, default_value_t=120, value_parser=clap::value_parser!(u64).range(1..=3600))]
        timeout_seconds: u64,
    },
}

#[tokio::main]
async fn main() {
    let json_requested = std::env::args_os().any(|a| a == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e)
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let _ = e.print();
            return;
        }
        Err(e) => {
            if json_requested {
                output_error(&AppError::invalid(e.to_string()), true);
            } else {
                let _ = e.print();
            }
            std::process::exit(2);
        }
    };
    let result = run(cli.command).await;
    match result {
        Ok(value) => {
            let response = json!({"schema_version":1,"ok":true,"result":value});
            if let Err(e) = output_value(&response, cli.json) {
                eprintln!("output_failed: {e}");
                std::process::exit(4);
            }
        }
        Err(e) => {
            output_error(&e, cli.json);
            std::process::exit(e.exit_code);
        }
    }
}

async fn run(command: Commands) -> Result<Value> {
    match command {
        Commands::Devices {
            backend,
            transport,
            scan_seconds,
        } => match backend {
            Backend::Direct => discovery::devices(transport.as_str(), scan_seconds).await,
            Backend::WindowsWebapi => WebApi::new()?.devices().await,
        },
        Commands::Doctor {
            device,
            scan_seconds,
        } => discovery::doctor(&device, scan_seconds).await,
        Commands::Render {
            input,
            model,
            tape_width,
            content_length_mm,
            padding_right_mm,
            fit,
            dither,
            threshold,
            output,
            job,
        } => {
            if model != "sr-mk2" || tape_width != 24 {
                return Err(AppError::unsupported(
                    "Only sr-mk2 with 24 mm tape geometry is currently verified",
                ));
            }
            let rendered = render::render(
                &input,
                &RenderOptions {
                    content_length_mm,
                    padding_right_mm,
                    fit,
                    dither,
                    threshold,
                },
            )?;
            let saved = job::write_job(&rendered, &output, &job)?;
            Ok(
                json!({"state":"rendered", "job_path":job, "preview_path":output, "job":saved, "total_label_length_mm":null}),
            )
        }
        Commands::Status { device, backend } => match backend {
            Backend::Direct => tepra_cli::usb_status::status(&device).await,
            Backend::WindowsWebapi => WebApi::new()?.status(&device).await,
        },
        Commands::Print {
            job: path,
            device,
            backend,
            copies,
            dry_run,
            experimental_usb,
            timeout_seconds,
        } => {
            if copies != 1 {
                return Err(AppError::unsupported(
                    "Only one copy per print attempt is supported",
                ));
            }
            if device.trim().is_empty() {
                return Err(AppError::invalid("Device must not be empty"));
            }
            let validated = job::load_job(&path)?;
            let canonical =
                serde_json::to_vec(&validated.job).map_err(|e| AppError::invalid(e.to_string()))?;
            let job_id = render::sha256(&canonical);
            if experimental_usb
                && (!matches!(backend, Backend::Direct) || !device.starts_with("usb:"))
            {
                return Err(AppError::invalid(
                    "--experimental-usb requires --backend direct and a usb: device ID",
                ));
            }
            if experimental_usb {
                tepra_cli::native_usb::validate(&validated.png_bytes)?;
            }
            if dry_run {
                return Ok(
                    json!({"state":"validated", "dry_run":true,"job_id":job_id,"device_id":device,
                    "job":validated.job,"connected":false,"print_supported":matches!(backend,Backend::WindowsWebapi) || (experimental_usb && cfg!(any(target_os="macos", target_os="linux"))),
                    "note":"Offline validation only; printer presence, installed tape and physical output are not verified."}),
                );
            }
            if matches!(backend, Backend::Direct) && !experimental_usb {
                return Err(direct_unsupported());
            }
            let mut session = PrintSession::begin(&state_directory()?, &job_id, &device)?;
            let usb_progress = tepra_cli::native_usb::PrintProgress::default();
            let print = async {
                match backend {
                    Backend::Direct => {
                        tepra_cli::native_usb::print_with_progress(
                            &device,
                            &validated.png_bytes,
                            &job_id,
                            Duration::from_secs(timeout_seconds),
                            usb_progress.clone(),
                        )
                        .await
                    }
                    Backend::WindowsWebapi => {
                        WebApi::new()?
                            .print(
                                &device,
                                &validated.png_bytes,
                                &job_id,
                                Duration::from_secs(timeout_seconds),
                            )
                            .await
                    }
                }
            };
            let result = await_print(print, tokio::signal::ctrl_c(), backend, &usb_progress).await;
            finish_attempt(&mut session, &job_id, &device, result)
        }
    }
}

async fn await_print(
    print: impl std::future::Future<Output = Result<Value>>,
    cancel: impl std::future::Future<Output = io::Result<()>>,
    backend: Backend,
    progress: &tepra_cli::native_usb::PrintProgress,
) -> Result<Value> {
    // Check an already-delivered interrupt before polling a future that can
    // submit a USB transfer. If completion races cancellation, be conservative.
    tokio::select! {
        biased;
        signal = cancel => {
            let mut error = if matches!(backend, Backend::Direct) {
                progress.interrupted()
            } else {
                AppError::unknown("Interrupted; printer may still be printing. Do not automatically retry.")
            };
            if let Err(signal_error) = signal {
                error.message = format!("Interrupt handler failed: {signal_error}. {}", error.message);
            }
            Err(error)
        },
        result = print => result,
    }
}

/// Persist exactly the outcome returned to the caller, including correlation IDs.
fn finish_attempt(
    session: &mut PrintSession,
    job_id: &str,
    device_id: &str,
    result: Result<Value>,
) -> Result<Value> {
    let result = match result {
        Ok(v) => Ok(
            json!({"state":"completed","job_id":job_id,"device_id":device_id,
            "attempt_id":session.attempt_id,"record_path":session.record_path,"backend_result":v}),
        ),
        Err(mut e) => {
            let state = error_state(&e).to_owned();
            e.context = Some(json!({"state":state,"job_id":job_id,"device_id":device_id,
                "attempt_id":session.attempt_id,"record_path":session.record_path,"backend_context":e.context}));
            Err(e)
        }
    };
    let record = match &result {
        Ok(v) => json!({"event":"result","ok":true,"state":"completed","result":v}),
        Err(e) => json!({"event":"result","ok":false,"state":error_state(e),"error":e}),
    };
    if let Err(e) = session.append(&record) {
        return Err(AppError::unknown(format!("Print attempted but final record could not be persisted: {e}"))
            .with_context(json!({"job_id":job_id,"device_id":device_id,"attempt_id":session.attempt_id,"record_path":session.record_path,"outcome":record})));
    }
    result
}

fn error_state(error: &AppError) -> &'static str {
    if error.exit_code == 7 {
        "unknown"
    } else if error
        .context
        .as_ref()
        .is_some_and(|c| c["state"] == "not_submitted" || c["transmission_started"] == false)
    {
        "not_submitted"
    } else {
        "failed"
    }
}

fn direct_unsupported() -> AppError {
    AppError::unsupported(
        "Direct printing requires --experimental-usb and the observed 24 mm USB profile. BLE printing is not implemented. No print commands were sent.",
    )
}
fn output_error(error: &AppError, json_output: bool) {
    if json_output {
        let value = json!({"schema_version":1,"ok":false,"state":error_state(error),"error":error});
        if let Err(e) = output_value(&value, true) {
            eprintln!("output_failed: {e}");
        }
    } else {
        eprintln!("{error}");
    }
}
fn output_value(value: &Value, compact: bool) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if compact {
        serde_json::to_writer(&mut out, value)?;
    } else {
        serde_json::to_writer_pretty(&mut out, value)?;
    }
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn command_graph_is_valid() {
        Cli::command().debug_assert();
    }

    #[tokio::test]
    async fn ready_interrupt_prevents_polling_print_submission() {
        let submitted = std::cell::Cell::new(false);
        let print = async {
            submitted.set(true);
            Ok(json!({"state":"completed"}))
        };
        let progress = tepra_cli::native_usb::PrintProgress::default();
        let error = await_print(
            print,
            std::future::ready(Ok(())),
            Backend::Direct,
            &progress,
        )
        .await
        .unwrap_err();
        assert!(!submitted.get());
        assert_eq!(error.exit_code, 4);
        assert_eq!(error_state(&error), "not_submitted");
    }

    #[tokio::test]
    async fn interrupt_drops_pending_print_future() {
        use std::cell::Cell;
        use std::task::Poll;
        struct Guard<'a>(&'a Cell<bool>);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let started = Cell::new(false);
        let dropped = Cell::new(false);
        let print = async {
            let _guard = Guard(&dropped);
            started.set(true);
            std::future::pending::<Result<Value>>().await
        };
        let cancel = std::future::poll_fn(|cx| {
            if started.get() {
                Poll::Ready(Ok(()))
            } else {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        });
        let progress = tepra_cli::native_usb::PrintProgress::default();
        let error = await_print(print, cancel, Backend::Direct, &progress)
            .await
            .unwrap_err();
        assert!(started.get());
        assert!(dropped.get());
        assert_eq!(error.exit_code, 4);
    }

    #[test]
    fn final_record_preserves_returned_failure_and_correlation_ids() {
        for (exit_code, transmitted, expected_state) in [
            (4, false, "not_submitted"),
            (7, true, "unknown"),
            (5, true, "failed"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut session = PrintSession::begin(dir.path(), "job", "usb:sr-mk2").unwrap();
            let error = AppError::new("test_failure", "synthetic test error", exit_code)
                .with_context(json!({"phase":"raster","transmission_started":transmitted,"raw_status":[22,0,1]}));
            let error = finish_attempt(&mut session, "job", "usb:sr-mk2", Err(error)).unwrap_err();
            assert_eq!(error_state(&error), expected_state);
            let records = std::fs::read_to_string(&session.record_path).unwrap();
            let last: Value = serde_json::from_str(records.lines().last().unwrap()).unwrap();
            assert_eq!(last["error"], serde_json::to_value(&error).unwrap());
            assert_eq!(last["state"], expected_state);
            assert_eq!(last["error"]["context"]["job_id"], "job");
            assert_eq!(last["error"]["context"]["attempt_id"], session.attempt_id);
            assert_eq!(
                last["error"]["context"]["backend_context"]["phase"],
                "raster"
            );
        }
    }
}
