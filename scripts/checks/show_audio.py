"""Prints the world/audio pause state, sinks and soundscape from artifacts/bevy-report.json."""
import json
from pathlib import Path
report = json.loads((Path(__file__).resolve().parents[2] / "artifacts/bevy-report.json").read_text())
sim = report.get("render", {}).get("simulation", {})
presentation = report.get("render", {}).get("presentation", {})
audio = presentation.get("audio", {})
print("map:", report.get("map"), "| mode:", presentation.get("console", {}).get("mode"),
      "| background_level:", sim.get("background_level"), "| paused:", sim.get("paused"),
      "| world_running:", sim.get("world_running"))
for key in ("paused", "pause_changes", "resume_changes", "sinks", "paused_sinks", "soundscape"):
    print(f"audio.{key}: {audio.get(key)}")
playing = [s for s in audio.get("players", []) if isinstance(s, dict)]
print("audio players:", len(playing))
for s in playing[:12]:
    print("  ", {k: s.get(k) for k in ("path", "paused", "volume", "position")})
