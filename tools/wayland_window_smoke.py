#!/usr/bin/env python3
"""Verify Wayland application identity through KWin's scripting API.

A Wayland surface is only visible to the compositor after a graphics API
attaches a buffer, so this probe runs the framebuffer example rather than a
window-only process. `DISPLAY` is removed from the child environment to force
winit's Wayland backend; KWin's `resourceClass` must then equal the
`WindowConfig.application_id`.

    cargo build -p framebuffer
    python3 tools/wayland_window_smoke.py --timeout 20
"""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from x11_window_smoke import Failure, inspect_kwin  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/framebuffer")
    parser.add_argument("--app-id", default="quasar-framebuffer")
    parser.add_argument("--frames", type=int, default=1000000)
    parser.add_argument("--timeout", type=float, default=20.0)
    args = parser.parse_args()

    if not os.environ.get("WAYLAND_DISPLAY"):
        raise Failure("WAYLAND_DISPLAY is not set")
    if not os.path.exists(args.binary):
        raise Failure(f"{args.binary} is missing; build it with cargo build -p framebuffer")
    for tool in ("qdbus6", "journalctl"):
        if shutil.which(tool) is None:
            raise Failure(f"Wayland KWin validation requires {tool}")

    environment = dict(os.environ)
    environment.pop("DISPLAY", None)
    process = subprocess.Popen(
        [args.binary, "--frames", str(args.frames)],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    try:
        kwin = inspect_kwin(args.app_id, None, True, args.timeout)
        if process.poll() is not None and process.returncode != 0:
            raise Failure(f"probe exited with {process.returncode} before validation")
        print(
            f"PASS wayland_app_id={args.app_id} display=Wayland kwin={kwin}"
        )
    finally:
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


if __name__ == "__main__":
    try:
        main()
    except Failure as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
