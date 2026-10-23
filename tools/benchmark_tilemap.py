#!/usr/bin/env python3
"""Compare indexed draw-list construction at fixed visible area and growing maps."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--iterations', type=int, default=1000)
    args = parser.parse_args()
    command = ['cargo', 'run', '--release', '--locked', '--quiet', '-p', 'quasar-tilemap', '--']
    reports = []
    with tempfile.TemporaryDirectory(prefix='quasar-map-bench-') as temporary:
        work = Path(temporary)
        shutil.copy2(ROOT / 'examples/tilequest/assets/atlas.png', work / 'atlas.png')
        tileset = json.loads((ROOT / 'examples/tilequest/assets/tilesets/island.tsj').read_text())
        tileset['firstgid'] = 1
        tileset['image'] = 'atlas.png'
        for size in [100, 1000]:
            source = work / f'{size}.tmj'
            source.write_text(json.dumps({'type': 'map', 'orientation': 'orthogonal', 'renderorder': 'right-down', 'width': size, 'height': size, 'tilewidth': 16, 'tileheight': 16, 'tilesets': [tileset], 'layers': [{'id': 1, 'name': 'ground', 'type': 'tilelayer', 'width': size, 'height': size, 'data': [1] * (size * size)}]}))
            result = subprocess.run(command + ['benchmark', str(source), '--size', '480', '320', '--iterations', str(args.iterations)], cwd=ROOT, capture_output=True, text=True, check=True)
            report = json.loads(result.stdout)
            assert report['stored_cells'] == size * size
            assert report['max_visited_cells'] <= 620
            reports.append(report)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({'profile': 'release', 'scope': 'one fully occupied tile layer; fixed 480x320 view; CPU only', 'cases': reports}, indent=2) + '\n')
    print(args.output.read_text())


if __name__ == '__main__':
    main()
