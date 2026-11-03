"""Sets only the saved menu background (game, map) in bin/hl2-config.json from the BG_GAME
and BG_MAP environment variables (used by runtime_chapter_quit.sh)."""
import json, os
from pathlib import Path
path = Path(__file__).resolve().parents[2] / "bin/hl2-config.json"
config = json.loads(path.read_text()) if path.exists() else {}
config.setdefault("gameplay", {})["background_map"] = os.environ["BG_MAP"]
config["gameplay"]["background_game"] = os.environ["BG_GAME"]
path.write_text(json.dumps(config, indent=2))
