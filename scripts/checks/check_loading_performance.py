"""Alternating isolated map-load pairs; LOADING_BASELINE names a retained package folder.

Writes raw timing, frame and environment reports under ignored artifacts/loading-performance.
LOADING_RUNS defaults to three; LOADING_OUTPUT selects a separate artifact folder. Run after validation/builds finish; each launch is muted.
"""
import hashlib
import json
import os
from pathlib import Path
import statistics
import time
import replays as r
from bench_environment import Monitor, known_interference

SCENES = [('hl2', 'background01', True), ('ep2', 'ep2_background01', True),
          ('gameplay', 'd1_trainstation_02', False)]


def package(folder):
    r.GAME = folder
    r.EXE = folder / r.EXE_NAME
    r.CONFIG = folder / 'hl2-config.json'


def sample(kind, folder, scene, repeat):
    label, map_name, background = scene
    package(folder)
    if r.CONFIG.exists():
        r.CONFIG.unlink()
    r.settings(video={'width': 1024, 'height': 768, 'hdr': 'none', 'hdr_migrated': True,
                      'advanced': {'antialiasing': 'none', 'vsync': False}})
    name = f'{kind}-{label}-{repeat}'
    args = ['--map', map_name, '--width', '1024', '--height', '768', '--frames', '240', '--profile']
    if background:
        args += ['--main-menu']
    with Monitor(r.EXE, 0.5) as monitor:
        blocked = known_interference(monitor.baseline)
        r.holds('no known competing workload', not blocked, blocked)
        started = time.monotonic()
        run = r.game(name, *args, capture=False)
        wall_ms = (time.monotonic() - started) * 1000
    frame = run.report['render']['presentation']['hud']['developer_overlay']['benchmark']
    r.holds('measured past frame warm-up', frame['measured_frames'] >= 100)
    r.holds('map load completed', not run.report['render']['presentation']['campaign']['failures'])
    audio = run.report['render']['presentation']['audio']
    r.holds('audio fully prepared', audio.get('ready', True))
    r.holds('world running after readiness', run.report['render']['simulation']['world_running'])
    environment = monitor.finish(r.OUT / (name + '.environment.json'), frame, (1024, 768))
    result = {'package': kind, 'scene': label, 'repeat': repeat, 'wall_ms': wall_ms,
              'loading_ms': run.report['render']['map_metadata']['loading_ms'],
              'audio_preparation_ms': audio.get('preparation_ms'),
              'audio_waves': audio['preloaded_waves'], 'frame': frame,
              'environment': environment,
              'sha256': hashlib.sha256(r.EXE.read_bytes()).hexdigest()}
    print(name, 'installed_ms', result['loading_ms']['request_to_installed_ms'],
          'wall_ms', wall_ms, flush=True)
    return result


def summarize(raw):
    out = {}
    for scene, _, _ in SCENES:
        groups = {}
        for kind in ['baseline', 'current']:
            rows = [v for v in raw if v['scene'] == scene and v['package'] == kind]
            installed = [v['loading_ms']['request_to_installed_ms'] for v in rows]
            groups[kind] = {'installed_ms': installed, 'median_ms': statistics.median(installed),
                            'range_ms': [min(installed), max(installed)],
                            'wall_ms': [v['wall_ms'] for v in rows],
                            'frame_ms': [v['frame']['mean_frame_ms'] for v in rows]}
        groups['paired_current_over_baseline'] = [b / a for a, b in
            zip(groups['baseline']['installed_ms'], groups['current']['installed_ms'])]
        out[scene] = groups
    return out


def main():
    baseline = Path(os.environ['LOADING_BASELINE']).resolve()
    if not baseline.is_relative_to((r.REPO / 'artifacts').resolve()):
        raise ValueError('LOADING_BASELINE must be a retained package under artifacts/')
    repeats = int(os.environ.get('LOADING_RUNS', '3'))
    if not 1 <= repeats <= 9:
        raise ValueError('LOADING_RUNS must be between 1 and 9')
    r.OUT = (r.REPO / os.environ.get('LOADING_OUTPUT', 'artifacts/loading-performance')).resolve()
    if not r.OUT.is_relative_to((r.REPO / 'artifacts').resolve()):
        raise ValueError('LOADING_OUTPUT must be under artifacts/')
    r.OUT.mkdir(parents=True, exist_ok=True)
    current = r.OUT / 'current-game'
    if baseline == current.resolve():
        raise ValueError('baseline and current packages must differ')
    package(current)
    r.stage()
    r.holds('different retained executables',
            hashlib.sha256((baseline / r.EXE_NAME).read_bytes()).digest() != hashlib.sha256(r.EXE.read_bytes()).digest())
    raw = []
    for repeat in range(repeats):
        order = [('baseline', baseline), ('current', current)]
        if repeat % 2:
            order.reverse()
        for scene in SCENES:
            for kind, folder in order:
                raw.append(sample(kind, folder, scene, repeat))
                (r.OUT / 'paired-raw.json').write_text(json.dumps(raw, indent=2))
    result = summarize(raw)
    (r.OUT / 'paired-summary.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
