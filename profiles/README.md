# Settings profiles

Each file is a partial `hl2-config.json`: sections and fields it leaves out take their defaults.
A run uses a profile through a copy, so the profile itself never changes:

```bash
mkdir -p artifacts/run && cp profiles/shadows-low.json artifacts/run/hl2-config.json
HL2RS_SETTINGS=artifacts/run/hl2-config.json bin/hl2-bevy --map d1_trainstation_02
```

- `test.json`: the baseline every packaged check starts from (all defaults).
- `shadows-low.json`, `shadows-medium.json`, `shadows-high.json`: Shadow detail for captures and
  benchmarks.

Without `HL2RS_SETTINGS` the game uses `hl2-config.json` beside the executable.
