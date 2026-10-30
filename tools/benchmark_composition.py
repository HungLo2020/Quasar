#!/usr/bin/env python3
"""Alternate identical native cube/HUD workloads with wait vs poll completion.

Run correctness tests separately with validation on. This driver uses release,
upstream default validation, GPU timestamps on, 64 warmup frames, and no requested vsync.
Desktop/compositor activity can still affect timings. Completion latency is a
host-observed GPU completion proxy, not physical input-to-photon latency.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--frames', type=int, default=600, help='measured frames per run')
    parser.add_argument('--pairs', type=int, default=3)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.frames < 1 or args.pairs < 1:
        parser.error('frames and pairs must be positive')
    args.output.mkdir(parents=True, exist_ok=True)
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'quasar-composition'], cwd=ROOT, check=True)
    env = os.environ.copy()
    binary = ROOT / 'target' / 'release' / ('quasar-composition.exe' if os.name == 'nt' else 'quasar-composition')
    runs = []
    for pair in range(args.pairs):
        order = [True, False] if pair % 2 == 0 else [False, True]
        for sync in order:
            policy = 'sync' if sync else 'poll'
            path = args.output.resolve() / f'{pair + 1}-{policy}.json'
            cmd = [str(binary), '--frames', str(args.frames + 64), '--benchmark', str(path)]
            if sync:
                cmd.append('--sync')
            result = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True, timeout=90)
            (args.output / f'{pair + 1}-{policy}.log').write_text(result.stdout + result.stderr)
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
            run = json.loads(path.read_text())
            assert run['frames'] == args.frames
            assert run['device_creations'] == 1
            runs.append(run)
            print(f'{pair + 1}-{policy}: CPU p50 {run["cpu_frame_ms"]["p50"]:.3f} ms; GPU {run["gpu_frame_ms"]}; waits {run["blocking_waits"]}', flush=True)
    summary = {'platform': platform.platform(), 'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(), 'frames_per_run': args.frames, 'pairs': args.pairs, 'validation': 'upstream defaults', 'gpu_timestamps': True, 'baseline_scope': 'same native scene and renderers; Wait versus Poll; excludes display latency', 'results': {}}
    for name, policy in [('sync', 'synchronous'), ('poll', 'completion-poll')]:
        subset = [r for r in runs if r['policy'] == policy]
        summary['results'][name] = {key: statistics.median([r[key] for r in subset]) for key in ['blocking_waits', 'rust_allocations', 'rust_allocated_bytes']}
        for key in ['cpu_frame_ms', 'gpu_frame_ms', 'cpu_submit_ms', 'acquire_ms', 'present_ms']:
            available = [r[key] for r in subset if r[key] is not None]
            summary['results'][name][key] = {quantile: statistics.median([r[quantile] for r in available]) for quantile in ['mean', 'p50', 'p95', 'p99']} if available else None
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
