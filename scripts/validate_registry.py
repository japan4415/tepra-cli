"""Run with uv --with PyYAML --with jsonschema; schema comes from pinned aqua."""

import json
from pathlib import Path
import sys

import jsonschema
import yaml

from release import ROOT, TARGETS, asset_name, version


registry = yaml.safe_load((ROOT / "aqua-registry.yaml").read_text())
jsonschema.validate(registry, json.loads(Path(sys.argv[1]).read_text()))
package = registry["packages"][0]
assert set(package["supported_envs"]) == {"/".join(env) for env in TARGETS.values()}
for target, (system, arch) in TARGETS.items():
    template = package["asset"]
    if system == "windows":
        template = package["overrides"][0]["asset"]
    name = template.replace("{{trimV .Version}}", version()).replace("{{.OS}}", system).replace("{{.Arch}}", arch)
    assert name == asset_name(target, version()), name
print("aqua registry schema and five release asset mappings are valid")
