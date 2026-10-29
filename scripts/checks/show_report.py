"""Prints the loaded maps (console "Loaded <game>/<map>" lines) and final campaign state from
artifacts/bevy-report.json, written by a bounded hl2-bevy run."""
import json
from pathlib import Path
report = json.loads((Path(__file__).resolve().parents[2] / "artifacts/bevy-report.json").read_text())
presentation = report.get("render", {}).get("presentation", {})
for line in presentation.get("console", {}).get("output", []):
    if line.startswith("Loaded ") or "background" in line.lower():
        print("console:", line)
campaign = presentation.get("campaign", {})
for key in ("loaded_game", "loaded_map", "last_event_game", "last_event", "failures"):
    print(f"campaign.{key}: {campaign.get(key)}")
