"""Prints the closed caption section of artifacts/bevy-report.json (dictionaries,
processed count, queued and recent caption streams, missing tokens) and the loaded map."""
import json
from pathlib import Path
report = json.loads((Path(__file__).resolve().parents[2] / "artifacts/bevy-report.json").read_text())
def find(node, key):
    if isinstance(node, dict):
        if key in node:
            return node[key]
        for v in node.values():
            r = find(v, key)
            if r is not None:
                return r
    return None
print("map:", find(report, "loaded_game"), find(report, "loaded_map"))
captions = find(report, "captions")
if captions is None:
    print("no captions section")
else:
    print("dictionaries:", captions.get("dictionaries"), "errors:", captions.get("errors"))
    print("processed:", captions.get("processed"))
    for s in captions.get("queued", []):
        print("  queued:", s[:120])
    for s in captions.get("recent", []):
        print("  recent:", s[:120])
    print("missing (first 20):", captions.get("missing", [])[:20])
print("attenuated scene captions:", find(report, "attenuated_captions"))
