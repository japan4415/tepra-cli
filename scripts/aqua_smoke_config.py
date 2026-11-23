"""Read or update the release tag in examples/aqua/aqua.yaml."""

import os
import re
import sys

from release import ROOT, validate_tag


def read_example_tag() -> str:
    config = (ROOT / "examples/aqua/aqua.yaml").read_text()
    ref_match = re.search(r"(?m)^    ref: (\S+)$", config)
    pkg_match = re.search(r"japan4415/tepra-cli@(\S+)", config)
    if not ref_match or not pkg_match or ref_match[1] != pkg_match[1]:
        raise ValueError("examples/aqua/aqua.yaml ref and package tag must match")
    tag = ref_match[1]
    validate_tag(tag, tag.removeprefix("v"))
    return tag


def main() -> None:
    if len(sys.argv) == 2 and sys.argv[1] == "--read":
        tag = read_example_tag()
        output = f"tag={tag}\n"
        print(output, end="")
        if path := os.environ.get("GITHUB_OUTPUT"):
            with open(path, "a", encoding="utf-8") as stream:
                stream.write(output)
        return
    tag = sys.argv[1]
    validate_tag(tag, tag.removeprefix("v"))
    path = ROOT / "examples/aqua/aqua.yaml"
    config = path.read_text()
    config = re.sub(r"(?m)^    ref: .+$", f"    ref: {tag}", config)
    config = re.sub(r"japan4415/tepra-cli@[^\s]+", f"japan4415/tepra-cli@{tag}", config)
    path.write_text(config)


if __name__ == "__main__":
    main()
