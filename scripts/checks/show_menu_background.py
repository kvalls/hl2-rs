"""Prints the saved menu background of bin/hl2-config.json as <game>/<map>."""
import json
from pathlib import Path
config = Path(__file__).resolve().parents[2] / "bin/hl2-config.json"
gameplay = json.loads(config.read_text()).get("gameplay", {}) if config.exists() else {}
print(f"{gameplay.get('background_game')}/{gameplay.get('background_map')}")
