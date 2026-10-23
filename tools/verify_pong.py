#!/usr/bin/env python3
"""Exercise CLI recording, replay, pixels, and malformed-input handling headlessly."""
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    command = ['cargo', 'run', '--locked', '--quiet', '-p', 'pong', '--no-default-features', '--']
    with tempfile.TemporaryDirectory(prefix='quasar-pong-') as temporary:
        work = Path(temporary)
        def run(*args, success=True):
            result = subprocess.run(command + list(args), cwd=ROOT, text=True, capture_output=True)
            if (result.returncode == 0) != success:
                raise RuntimeError(result.stdout + result.stderr)
            return result
        for mode in [[], ['--two-player']]:
            run('--headless', '--autoplay', '--ticks', '12000', *mode,
                '--record', str(work / 'replay.json'), '--state', str(work / 'original.json'), '--screenshot', str(work / 'original.png'))
            run('--headless', '--replay', str(work / 'replay.json'),
                '--state', str(work / 'played.json'), '--screenshot', str(work / 'played.png'))
            assert (work / 'original.json').read_bytes() == (work / 'played.json').read_bytes()
            assert (work / 'original.png').read_bytes() == (work / 'played.png').read_bytes()
            state = json.loads((work / 'played.json').read_text())
            assert state['ticks'] == 12000 and state['longest_rally'] > 0
        controls_path = work / 'controls.json'
        run('--write-default-bindings', str(controls_path))
        controls = json.loads(controls_path.read_text())
        controls['bindings']['actions']['serve'] = [{'type': 'button', 'button': {'Key': 'Enter'}}]
        controls_path.write_text(json.dumps(controls))
        run('--check-bindings', str(controls_path))
        controls['bindings']['actions']['serve'][0]['button']['Key'] = 'NotAKey'
        controls_path.write_text(json.dumps(controls))
        run('--check-bindings', str(controls_path), success=False)
        run('--headless', '--bindings', str(controls_path), success=False)
        run('--headless', '--ticks', '0', success=False)
        (work / 'invalid.json').write_text('{"version":99,"mode":"Solo","tick_hz":120,"inputs":[]}')
        run('--headless', '--replay', str(work / 'invalid.json'), success=False)
        print('Solo/local: 12,000-tick replay states and PNGs match; controls export/validation and invalid CLI/replays checked.')


if __name__ == '__main__':
    main()
