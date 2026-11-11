"""Release metadata, portable archives, and complete-set checksums (stdlib only)."""

import argparse
import gzip
import hashlib
import io
import os
from pathlib import Path
import re
import stat
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile


TARGETS = {
    "aarch64-apple-darwin": ("darwin", "arm64"),
    "x86_64-apple-darwin": ("darwin", "amd64"),
    "aarch64-unknown-linux-gnu": ("linux", "arm64"),
    "x86_64-unknown-linux-gnu": ("linux", "amd64"),
    "x86_64-pc-windows-msvc": ("windows", "amd64"),
}
ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(
    r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)


def validate_tag(tag: str, version: str) -> None:
    match = SEMVER.fullmatch(tag)
    if not match or tag != f"v{version}":
        raise ValueError(f"Tag {tag!r} must be v<Cargo.toml version>: v{version}")
    if match[4] and any(
        part.isdigit() and len(part) > 1 and part.startswith("0")
        for part in match[4].split(".")
    ):
        raise ValueError("Numeric prerelease identifiers cannot have leading zeroes")


def version() -> str:
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    value = manifest["package"]["version"]
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    packages = [p for p in lock["package"] if p["name"] == "tepra-cli"]
    if len(packages) != 1 or packages[0]["version"] != value:
        raise ValueError("Cargo.toml and Cargo.lock versions differ")
    validate_tag(f"v{value}", value)
    return value


def asset_name(target: str, release_version: str) -> str:
    system, arch = TARGETS[target]
    extension = "zip" if system == "windows" else "tar.gz"
    return f"tepra_{release_version}_{system}_{arch}.{extension}"


def write_archive(destination: Path, files: dict[str, bytes]) -> None:
    """Stable metadata; binaries remain executable after extraction."""
    if destination.suffix == ".zip":
        with zipfile.ZipFile(destination, "x", zipfile.ZIP_DEFLATED) as archive:
            for name, data in sorted(files.items()):
                info = zipfile.ZipInfo(name, date_time=(2000, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = (stat.S_IFREG | (0o755 if name.startswith("tepra") else 0o644)) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, data)
    else:
        with destination.open("xb") as raw:
            with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w") as archive:
                    for name, data in sorted(files.items()):
                        info = tarfile.TarInfo(name)
                        info.size = len(data)
                        info.mode = 0o755 if name == "tepra" else 0o644
                        archive.addfile(info, io.BytesIO(data))


def smoke(archive_path: Path, release_version: str) -> None:
    with tempfile.TemporaryDirectory(prefix="tepra-release-") as directory:
        root = Path(directory)
        if archive_path.suffix == ".zip":
            with zipfile.ZipFile(archive_path) as archive:
                archive.extractall(root)
            executable = root / "tepra.exe"
        else:
            with tarfile.open(archive_path) as archive:
                archive.extractall(root, filter="data")
            executable = root / "tepra"
        result = subprocess.check_output([str(executable), "--version"], text=True).strip()
        if result != f"tepra {release_version}":
            raise ValueError(f"Unexpected executable version: {result!r}")
        subprocess.run([str(executable), "--help"], check=True, stdout=subprocess.DEVNULL)


def checksums(directory: Path, release_version: str) -> Path:
    expected = {asset_name(target, release_version) for target in TARGETS}
    actual = {p.name for p in directory.iterdir()}
    if actual != expected:
        raise ValueError(f"Incomplete/unexpected assets: missing={expected - actual}, extra={actual - expected}")
    lines = []
    for name in sorted(expected):
        path = directory / name
        if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
            raise ValueError(f"Invalid asset: {name}")
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        lines.append(f"{digest}  {name}\n")
    destination = directory / "SHA256SUMS"
    destination.write_text("".join(lines), encoding="utf-8", newline="\n")
    return destination


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["metadata", "package", "checksums"])
    parser.add_argument("--tag", default="")
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--directory", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    release_version = version()
    if args.tag:
        validate_tag(args.tag, release_version)
    if args.command == "metadata":
        output = f"version={release_version}\nprerelease={str('-' in release_version).lower()}\n"
        print(output, end="")
        if path := os.environ.get("GITHUB_OUTPUT"):
            with open(path, "a", encoding="utf-8") as stream:
                stream.write(output)
    elif args.command == "package":
        if not args.target:
            parser.error("package requires --target")
        binary_name = "tepra.exe" if TARGETS[args.target][0] == "windows" else "tepra"
        binary = ROOT / "target" / args.target / "release" / binary_name
        files = {binary_name: binary.read_bytes(), "README.md": (ROOT / "README.md").read_bytes()}
        files.update({p.relative_to(ROOT).as_posix(): p.read_bytes() for p in sorted((ROOT / "docs").glob("*.md"))})
        args.directory.mkdir(parents=True, exist_ok=True)
        archive = args.directory / asset_name(args.target, release_version)
        write_archive(archive, files)
        smoke(archive, release_version)
        print(archive)
    else:
        print(checksums(args.directory, release_version))


if __name__ == "__main__":
    main()
