"""Use the published version in the aqua example for release installation tests."""

import re
import sys

from release import ROOT, validate_tag


tag = sys.argv[1]
validate_tag(tag, tag.removeprefix("v"))
path = ROOT / "examples/aqua/aqua.yaml"
config = path.read_text()
config = re.sub(r"(?m)^    ref: .+$", f"    ref: {tag}", config)
config = re.sub(r"japan4415/tepra-cli@[^\s]+", f"japan4415/tepra-cli@{tag}", config)
path.write_text(config)
