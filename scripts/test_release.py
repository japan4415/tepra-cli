import hashlib
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

import release


class ReleaseTests(unittest.TestCase):
    def test_tag_must_match_manifest_and_semver(self):
        for version in ["0.1.0", "1.2.3-rc.1"]:
            release.validate_tag("v" + version, version)
        for tag, version in [("v0.2.0", "0.1.0"), ("0.1.0", "0.1.0"), ("v01.2.3", "01.2.3"),
                             ("v1.2.3-01", "1.2.3-01"), ("v1.2.3;echo", "1.2.3;echo")]:
            with self.assertRaises(ValueError):
                release.validate_tag(tag, version)

    def test_archive_contents_permissions_and_reproducibility(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for ext, executable in [("tar.gz", "tepra"), ("zip", "tepra.exe")]:
                files = {executable: b"binary", "README.md": b"documentation", "docs/release.md": b"release"}
                a, b = root / ("a." + ext), root / ("b." + ext)
                release.write_archive(a, files)
                release.write_archive(b, files)
                self.assertEqual(a.read_bytes(), b.read_bytes())
                if ext == "tar.gz":
                    with tarfile.open(a) as archive:
                        self.assertEqual(set(archive.getnames()), set(files))
                        self.assertEqual(archive.getmember(executable).mode, 0o755)
                        self.assertEqual(archive.extractfile(executable).read(), b"binary")
                else:
                    with zipfile.ZipFile(a) as archive:
                        self.assertEqual(set(archive.namelist()), set(files))
                        self.assertEqual(archive.read(executable), b"binary")
                with self.assertRaises(FileExistsError):
                    release.write_archive(a, files)

    def test_checksums_require_complete_exact_asset_set(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(ValueError):
                release.checksums(root, "0.1.0")
            for target in release.TARGETS:
                (root / release.asset_name(target, "0.1.0")).write_bytes(target.encode())
            extra = root / "unexpected.tar.gz"
            extra.write_bytes(b"extra")
            with self.assertRaises(ValueError):
                release.checksums(root, "0.1.0")
            extra.unlink()
            manifest = release.checksums(root, "0.1.0")
            lines = manifest.read_text().splitlines()
            self.assertEqual(len(lines), 5)
            for line in lines:
                digest, name = line.split("  ")
                self.assertEqual(digest, hashlib.sha256((root / name).read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
