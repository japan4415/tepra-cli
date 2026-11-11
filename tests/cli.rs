use serde_json::Value;
use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tepra"))
        .args(args)
        .output()
        .unwrap()
}
fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn json_errors_cover_parser_and_execution() {
    let parser = cli(&["print", "--json"]);
    assert_eq!(parser.status.code(), Some(2));
    assert_eq!(json(&parser)["ok"], false);
    assert!(parser.stderr.is_empty());
    let unsupported = cli(&["status", "--device", "ble:missing:99", "--json"]);
    assert_eq!(unsupported.status.code(), Some(6));
    assert_eq!(json(&unsupported)["error"]["code"], "unsupported");
}

#[test]
fn render_and_dry_run_are_offline_and_detect_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let preview = dir.path().join("preview.png");
    let job = dir.path().join("job.json");
    image::RgbImage::from_pixel(7, 3, image::Rgb([0, 0, 0]))
        .save(&input)
        .unwrap();
    let rendered = cli(&[
        "render",
        input.to_str().unwrap(),
        "--content-length-mm",
        "50",
        "--padding-right-mm",
        "1",
        "--output",
        preview.to_str().unwrap(),
        "--job",
        job.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    assert_eq!(json(&rendered)["result"]["job"]["raster"]["width"], 723);
    assert_eq!(
        json(&rendered)["result"]["job"]["options"]["padding_right_mm"],
        1.0
    );
    let args = [
        "print",
        "--job",
        job.to_str().unwrap(),
        "--device",
        "usb:does-not-exist:99",
        "--dry-run",
        "--json",
    ];
    let validated = cli(&args);
    assert!(validated.status.success());
    assert_eq!(json(&validated)["result"]["connected"], false);
    assert_eq!(json(&validated)["result"]["print_supported"], false);
    let disabled = cli(&[
        "print",
        "--job",
        job.to_str().unwrap(),
        "--device",
        "usb:does-not-exist:99",
        "--json",
    ]);
    assert_eq!(disabled.status.code(), Some(6));
    let conflicting = cli(&[
        "print",
        "--job",
        job.to_str().unwrap(),
        "--device",
        "printer",
        "--backend",
        "windows-webapi",
        "--experimental-usb",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(conflicting.status.code(), Some(2));
    std::fs::write(&preview, b"changed").unwrap();
    let tampered = cli(&args);
    assert_eq!(tampered.status.code(), Some(2));
    assert_eq!(json(&tampered)["ok"], false);
}

#[test]
fn experimental_dry_run_enforces_native_size_limit_without_connecting() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let preview = dir.path().join("preview.png");
    let job = dir.path().join("job.json");
    image::RgbImage::from_pixel(2, 2, image::Rgb([0, 0, 0]))
        .save(&input)
        .unwrap();
    assert!(
        cli(&[
            "render",
            input.to_str().unwrap(),
            "--content-length-mm",
            "100",
            "--padding-right-mm",
            "1",
            "--output",
            preview.to_str().unwrap(),
            "--job",
            job.to_str().unwrap()
        ])
        .status
        .success()
    );
    let response = cli(&[
        "print",
        "--job",
        job.to_str().unwrap(),
        "--device",
        "usb:does-not-exist:99",
        "--experimental-usb",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(response.status.code(), Some(2));
    assert_eq!(json(&response)["error"]["code"], "invalid_input");
}
