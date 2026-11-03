"""Owned GPU check: retail camera blur reaches the final frame while the camera turns.

Scripted turns usually capture just after the camera stops, when blur is zero by design, so these
runs capture at a frame count with the camera still sweeping. Uses an isolated muted package. The
negative control replaces the blur with a plain read of the frame in the private copy.
"""
import json
import statistics
import replays as replay
from images import png_rgb

CAPTURE_FRAME = "500"
# Blur removes at least this share of the horizontal edge energy of a sharp turn.
MIN_ENERGY_DROP = 0.2
RUNS = 3
SWEEP_TICKS = 16
SWEEP_DEGREES_PER_TICK = 1.5
TURN = [{"tick": 1, "fly": True, "eye": [-3104, -2018, 128], "yaw": -48,
         "actions": [{"action": "loadout"}, {"action": "slot", "slot": 0}]},
        {"tick": 2, "actions": [{"action": "confirm"}]}]
# Sweep back and forth so every capture sees the same scene, whatever frame it lands on.
TURN += [{"tick": tick, "yaw": -48.+abs((tick % SWEEP_TICKS)-SWEEP_TICKS/2)*SWEEP_DEGREES_PER_TICK-SWEEP_TICKS/4*SWEEP_DEGREES_PER_TICK}
         for tick in range(100, 1100)]
TURN.append({"tick": 1100, "label": "end"})
PLAIN_READ = "    return textureSampleLevel(source, linear_sampler, in.uv, 0.0);\n"
BLUR_ENTRY = "fn motion_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {\n"


def edge_energy(path):
    width, height, rgb = png_rgb(path)
    return sum(abs(rgb[(y*width+x)*3+1]-rgb[(y*width+x+1)*3+1])
               for y in range(0, height*2//3, 2) for x in range(width-1))


def turn(mode, aa):
    energies = []
    for run in range(RUNS):
        if replay.CONFIG.exists():
            replay.CONFIG.unlink()
        replay.settings(video={"width": 1024, "height": 768, "hdr": "none", "hdr_migrated": True, "advanced": {
            "motion_blur": mode != "off", "motion_blur_viewmodel": mode == "advanced", "motion_blur_world": False,
            "antialiasing": aa, "vsync": False}})
        fixture = replay.OUT / "turn.input.json"
        fixture.write_text(json.dumps(TURN), encoding="utf-8")
        name = f"retail_{mode}_{aa}_{run}"
        capture = replay.OUT / f"{name}.png"
        try:
            replay.game(name, "--map", "d1_trainstation_02", "--width", "1024", "--height", "768",
                        "--movement-script", str(fixture), "--frames", CAPTURE_FRAME)
        except replay.Failure:
            # The game reports the deliberately unfinished script after writing its capture.
            if not capture.exists():
                raise
        energies.append(edge_energy(capture))
    return statistics.median(energies)


def blurred(mode, aa, sharp):
    return turn(mode, aa) < sharp*(1-MIN_ENERGY_DROP)


def main():
    replay.OUT = replay.REPO / "artifacts/retail-blur"
    replay.GAME = replay.OUT / "game"
    replay.EXE = replay.GAME / replay.EXE_NAME
    replay.CONFIG = replay.GAME / "hl2-config.json"
    replay.OUT.mkdir(parents=True, exist_ok=True)
    replay.build()
    replay.stage()
    shader = replay.GAME / "bevy-assets/shaders/motion_blur.wgsl"
    original = shader.read_text(encoding="utf-8")
    replay.holds("blur entry point", BLUR_ENTRY in original)
    for aa in ("none", "x4"):
        sharp = turn("off", aa)
        try:
            shader.write_text(original.replace(BLUR_ENTRY, BLUR_ENTRY+PLAIN_READ), encoding="utf-8")
            replay.holds(f"negative control detects missing blur ({aa})", not blurred("enabled", aa, sharp))
        finally:
            shader.write_text(original, encoding="utf-8")
        for mode in ("enabled", "advanced"):
            replay.holds(f"{mode} blurs a turn ({aa})", blurred(mode, aa, sharp))
    print("retail blur GPU checks passed")


if __name__ == "__main__":
    main()
