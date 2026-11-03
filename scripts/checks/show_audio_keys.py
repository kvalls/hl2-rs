"""Prints the keys and scalar values of the audio report in artifacts/bevy-report.json."""
import json
from pathlib import Path
report = json.loads((Path(__file__).resolve().parents[2] / "artifacts/bevy-report.json").read_text())
audio = report["render"]["presentation"]["audio"]
for k, v in audio.items():
    text = json.dumps(v)
    print(k, ":", text[:300])
