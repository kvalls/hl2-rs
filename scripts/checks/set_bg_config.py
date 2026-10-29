"""Sets the saved menu background (game, map) and FOV in bin/hl2-config.json from the
BG_GAME, BG_MAP and BG_FOV environment variables (used by runtime_bg_compare.sh)."""
import json, os
from pathlib import Path
path = Path(__file__).resolve().parents[2] / "bin/hl2-config.json"
config = json.loads(path.read_text()) if path.exists() else {}
config.setdefault("gameplay", {})["background_map"] = os.environ["BG_MAP"]
config["gameplay"]["background_game"] = os.environ["BG_GAME"]
config.setdefault("video", {})["fov"] = float(os.environ["BG_FOV"])
path.write_text(json.dumps(config, indent=2))
