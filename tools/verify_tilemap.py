#!/usr/bin/env python3
"""Verify tilemap CLI interoperability, deterministic game replay, and relocation."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    subprocess.run(['cargo', 'build', '--locked', '-p', 'quasar-tilemap', '-p', 'tilequest', '--no-default-features', '--features', 'quasar-tilemap/render-2d'], cwd=ROOT, check=True)
    suffix = '.exe' if os.name == 'nt' else ''
    cli = ROOT / 'target/debug' / ('quasar-tilemap' + suffix)
    game = ROOT / 'target/debug' / ('tilequest' + suffix)
    source = ROOT / 'examples/tilequest/assets/island.tmj'
    with tempfile.TemporaryDirectory(prefix='quasar-tilemap-') as temporary:
        work = Path(temporary)

        def run(binary, *args, code=0, cwd=work):
            result = subprocess.run([str(binary), *map(str, args)], cwd=cwd, capture_output=True, text=True)
            if result.returncode != code:
                raise RuntimeError(f'{args}: {result.stdout}\n{result.stderr}')
            return result

        run(cli, '--help')
        run(cli, 'validate', source, '--assets')
        normalized = work / 'bundle/map.json'
        run(cli, 'build', source, '--output', normalized)
        exported = work / 'tiled/map.tmj'
        run(cli, 'convert', normalized, '--tiled', '--output', exported)
        restored = work / 'restored/map.json'
        run(cli, 'build', exported, '--output', restored)
        assert normalized.read_bytes() == restored.read_bytes(), 'Tiled roundtrip changed normalized data'
        for name in ['a', 'b']:
            run(cli, 'preview', normalized, '--camera', 100, 100, '--time', 800, '--output', work / f'{name}.png')
        assert (work / 'a.png').read_bytes() == (work / 'b.png').read_bytes()
        run(cli, 'preview', normalized, '--camera', 700, 500, '--output', work / 'other.png')
        assert (work / 'a.png').read_bytes() != (work / 'other.png').read_bytes()
        run(cli, 'set', normalized, '--layer', 1, '--cell', 10, 10, '--gid', 2, '--output', work / 'edit/map.json')
        run(cli, 'validate', work / 'edit/map.json', '--assets')
        bad = json.loads(normalized.read_text())
        bad['quasar_version'] = 99
        (work / 'bad.json').write_text(json.dumps(bad))
        error = run(cli, 'validate', work / 'bad.json', code=1)
        assert json.loads(error.stdout)['error']['code'] == 'version'
        run(cli, 'preview', normalized, '--unknown', code=2)
        run(cli, 'preview', normalized, '--size', 4294967295, 4294967295, '--output', work / 'huge.png', code=1)
        run(cli, 'benchmark', normalized, '--time', 18446744073709551615, '--iterations', 2)
        atlas = normalized.parent / 'atlas.png'
        original = atlas.read_bytes()
        atlas.write_bytes(original[:30])
        run(cli, 'validate', normalized, '--assets', code=1)
        atlas.write_bytes(original)
        atlas.unlink()
        run(cli, 'validate', normalized, '--assets', code=1)
        atlas.write_bytes(original)
        run(game, '--headless', '--autoplay', '--ticks', 6000, '--record', work / 'replay.json', '--state', work / 'state.json', '--screenshot', work / 'game.png')
        run(game, '--headless', '--replay', work / 'replay.json', '--state', work / 'played.json', '--screenshot', work / 'played.png')
        assert (work / 'state.json').read_bytes() == (work / 'played.json').read_bytes()
        assert (work / 'game.png').read_bytes() == (work / 'played.png').read_bytes()
        state = json.loads((work / 'state.json').read_text())
        assert state['phase'] == 'Won' and len(state['collected']) == 3
        run(game, '--write-default-bindings', work / 'controls.json')
        run(game, '--headless', '--ticks', 1, '--bindings', work / 'controls.json')
        controls = json.loads((work / 'controls.json').read_text())
        controls['version'] = 99
        (work / 'controls.json').write_text(json.dumps(controls))
        run(game, '--headless', '--bindings', work / 'controls.json', code=1)
        replay = json.loads((work / 'replay.json').read_text())
        replay['map_hash'] ^= 1
        (work / 'bad-replay.json').write_text(json.dumps(replay))
        run(game, '--headless', '--replay', work / 'bad-replay.json', code=1)
        run(game, '--headless', '--ticks', 0, code=1)
        # Executable-relative Basalt packaging, launched from unrelated CWD.
        packaged = work / 'Tilequest/app/bin' / game.name
        packaged.parent.mkdir(parents=True)
        shutil.copy2(game, packaged)
        shutil.copytree(ROOT / 'examples/tilequest/assets', work / 'Tilequest/app/assets')
        run(packaged, '--headless', '--autoplay', '--ticks', 6000, '--state', work / 'relocated.json', '--screenshot', work / 'relocated.png')
        assert (work / 'state.json').read_bytes() == (work / 'relocated.json').read_bytes()
        assert (work / 'game.png').read_bytes() == (work / 'relocated.png').read_bytes()
        assert not (work / 'Tilequest/data').exists(), 'unsolicited data writes'
        generated = work / 'regenerated'
        generated.mkdir()
        shutil.copy2(ROOT / 'examples/tilequest/assets/generate.py', generated / 'generate.py')
        subprocess.run([__import__('sys').executable, str(generated / 'generate.py')], check=True, capture_output=True)
        for asset in ['atlas.png', 'island.tmj', 'tilesets/island.tsj']:
            # Text newline conventions differ across platforms; PNG bytes are exact.
            actual = generated / asset
            expected = ROOT / 'examples/tilequest/assets' / asset
            assert actual.read_bytes() == expected.read_bytes() if asset.endswith('.png') else actual.read_text() == expected.read_text()

    print('Tilemap assets/roundtrip/edit/previews/errors, 6,000-tick winning replay, bindings, relocated Basalt package, and reproducible assets verified.')


if __name__ == '__main__':
    main()
