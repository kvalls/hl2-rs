"""Sets the saved menu background to episodic/ep1_background02 and Options > Audio closed
captions to CAPTIONS (default closed_captions) in bin/hl2-config.json
(used by runtime_captions.sh)."""
import json, os
from pathlib import Path
path = Path(__file__).resolve().parents[2] / "bin/hl2-config.json"
config = json.loads(path.read_text()) if path.exists() else {}
config.setdefault("gameplay", {})["background_map"] = "ep1_background02"
config["gameplay"]["background_game"] = "episodic"
config.setdefault("audio", {})["captions"] = os.environ.get("CAPTIONS", "closed_captions")
path.write_text(json.dumps(config, indent=2))
