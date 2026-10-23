#!/usr/bin/env python3
"""Build an app-only Basalt v1 archive; never publish or alter user data."""
import argparse
import json
from pathlib import Path
import platform
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def package_version():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1', '--locked'],
        cwd=ROOT, text=True))
    return next(package['version'] for package in metadata['packages']
                if package['name'] == 'pong')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='Package an existing native desktop build instead of building release')
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    args = parser.parse_args()
    system = {'Linux': 'linux', 'Windows': 'windows', 'Darwin': 'macos'}.get(platform.system())
    machine = {'x86_64': 'x64', 'AMD64': 'x64', 'aarch64': 'arm64', 'arm64': 'arm64', 'riscv64': 'riscv64'}.get(platform.machine())
    if system is None or machine is None:
        parser.error('unrecognized host platform; cross packaging is not implemented')
    suffix = '.exe' if system == 'windows' else ''
    if args.binary is None:
        subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'pong'], cwd=ROOT, check=True)
        binary = ROOT / 'target' / 'release' / ('pong' + suffix)
    else:
        binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f'executable not found: {binary}')
    args.output.mkdir(parents=True, exist_ok=True)
    version = package_version()
    archive = args.output / f'Pong-{version}-{system}-{machine}.zip'
    runtime_notes = (ROOT / 'packaging/PONG-RUNTIME.txt').read_text().splitlines(keepends=True)
    if not runtime_notes or not runtime_notes[0].startswith('Quasar Pong '):
        raise SystemExit('packaging/PONG-RUNTIME.txt must start with "Quasar Pong <version>"')
    runtime_notes[0] = f'Quasar Pong {version}\n'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as output:
        executable = zipfile.ZipInfo('app/bin/pong' + suffix)
        executable.create_system = 3
        executable.external_attr = (0o100755 << 16)
        executable.compress_type = zipfile.ZIP_DEFLATED
        output.writestr(executable, binary.read_bytes())
        output.write(ROOT / 'examples/pong/assets/cover.png', 'app/basalt/cover.png')
        output.writestr('app/PONG-RUNTIME.txt', ''.join(runtime_notes))
    with zipfile.ZipFile(archive) as check:
        names = check.namelist()
        assert all(name.startswith('app/') and '..' not in Path(name).parts for name in names)
        assert check.getinfo('app/bin/pong' + suffix).external_attr >> 16 & 0o111
        assert check.testzip() is None
    print(archive)


if __name__ == '__main__':
    main()
