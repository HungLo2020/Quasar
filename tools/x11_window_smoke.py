#!/usr/bin/env python3
"""Verify Quasar's native X11 window identity and opaque-region metadata.

The script runs the `desktop_identity` example, then polls xprop/xwininfo until
the requested properties and a resize are visible. It uses xdotool when
available and falls back to a small libX11 ctypes resize, so it also works in a
minimal Xvfb image.

Typical isolated run:

    cargo build -p quasar-window --example desktop_identity
    xvfb-run -a python3 tools/x11_window_smoke.py

KDE/KWin check (XWayland session):

    python3 tools/x11_window_smoke.py --kwin \
        --binary target/debug/examples/desktop_identity
"""

import argparse
import ctypes
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time


class Failure(Exception):
    pass


def output(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT)


def xprop(window, name):
    try:
        return output("xprop", "-id", window, name)
    except subprocess.CalledProcessError as error:
        return error.output or ""


def prop_values(window, name):
    text = xprop(window, name)
    if "not found" in text or "no such atom" in text:
        return None
    _, _, values = text.partition("=")
    return [part.strip() for part in values.split(",")] if values else []


def prop_ints(window, name):
    values = prop_values(window, name)
    return None if values is None else [int(value, 0) for value in values]


def window_size(window):
    text = output("xwininfo", "-id", window)
    width = int(re.search(r"Width:\s+(\d+)", text).group(1))
    height = int(re.search(r"Height:\s+(\d+)", text).group(1))
    return width, height


def poll(description, check, timeout):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            result = check()
        except (Failure, ValueError, AttributeError, subprocess.CalledProcessError) as error:
            last = str(error)
        else:
            if result:
                return result
            last = "condition is false"
        time.sleep(0.02)
    raise Failure(f"timed out waiting for {description}: {last}")


def resize_with_xdotool(window, width, height):
    subprocess.run(
        ["xdotool", "windowsize", "--sync", window, str(width), str(height)],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def open_xlib():
    xlib = ctypes.cdll.LoadLibrary("libX11.so.6")
    xlib.XOpenDisplay.restype = ctypes.c_void_p
    xlib.XOpenDisplay.argtypes = [ctypes.c_char_p]
    xlib.XResizeWindow.argtypes = [
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.c_uint,
        ctypes.c_uint,
    ]
    xlib.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    xlib.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = xlib.XOpenDisplay(None)
    if not display:
        raise Failure(f"cannot open DISPLAY={os.environ.get('DISPLAY')}")
    return xlib, display


def resize_with_xlib(window, width, height):
    xlib, display = open_xlib()
    try:
        xlib.XResizeWindow(display, int(window, 0), width, height)
        xlib.XSync(display, 0)
    finally:
        xlib.XCloseDisplay(display)


def inspect_kwin(class_name, instance, opaque, timeout):
    """Read KWin's own view of the window through its scripting API.

    `kwindowprop` only reports the active window and can block or show another
    application. A short KWin script lists the matching class directly and prints
    the compositor-side opacity, normal-window flag, class and instance to the
    user journal.
    """
    for tool in ("qdbus6", "journalctl"):
        if shutil.which(tool) is None:
            raise Failure(f"--kwin requires {tool}")

    def kwin_result():
        # Reload the script on every poll: a Wayland surface is not in KWin's
        # window list until its first buffer is attached.
        nonce = f"{os.getpid()}-{time.monotonic_ns()}"
        script = f"""
const target = workspace.windowList().find(w => w.resourceClass === {json.dumps(class_name)});
if (target) {{
    print("QUASAR_PROBE {nonce} class=" + target.resourceClass
        + " name=" + target.resourceName
        + " opacity=" + target.opacity
        + " normal=" + target.normalWindow);
}} else {{
    print("QUASAR_PROBE {nonce} missing");
}}
"""
        with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as file:
            file.write(script)
            path = file.name
        since = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        try:
            subprocess.run(
                ["qdbus6", "org.kde.KWin", "/Scripting",
                 "org.kde.kwin.Scripting.loadScript", path],
                check=True,
                capture_output=True,
                text=True,
                timeout=5,
            )
            subprocess.run(
                ["qdbus6", "org.kde.KWin", "/Scripting",
                 "org.kde.kwin.Scripting.start"],
                check=True,
                capture_output=True,
                text=True,
                timeout=5,
            )
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            raise Failure(f"load KWin inspection script: {error}") from error
        finally:
            os.unlink(path)

        try:
            journal = subprocess.run(
                ["journalctl", "--user", "--since", since, "--no-pager"],
                check=True,
                capture_output=True,
                text=True,
                timeout=5,
            ).stdout
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            raise Failure(f"read KWin journal: {error}") from error
        matches = re.findall(rf"QUASAR_PROBE {re.escape(nonce)} (.*)", journal)
        if not matches:
            raise Failure("KWin did not report the probe window yet")
        fields = dict(
            part.split("=", 1) for part in matches[-1].split() if "=" in part
        )
        if fields.get("class") != class_name:
            raise Failure(f"KWin class is {fields.get('class')}, expected {class_name}")
        if instance is not None and fields.get("name") != instance:
            raise Failure(f"KWin instance is {fields.get('name')}, expected {instance}")
        if fields.get("normal") != "true":
            raise Failure(f"KWin normalWindow is {fields.get('normal')}, expected true")
        if opaque:
            try:
                opacity = float(fields["opacity"])
            except (KeyError, ValueError) as error:
                raise Failure(f"KWin did not report opacity: {fields}") from error
            if opacity < 0.999:
                raise Failure(f"KWin opacity is {opacity}, expected full opacity")
        return fields

    return poll("KWin window identity and opacity", kwin_result, timeout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        default="target/debug/examples/desktop_identity",
        help="desktop_identity example binary",
    )
    parser.add_argument("--app-id", default="quasar-smoke")
    parser.add_argument("--instance", default="quasar-smoke")
    parser.add_argument("--class", dest="class_name", default="QuasarSmoke")
    parser.add_argument("--transparent", action="store_true")
    parser.add_argument(
        "--kwin",
        action="store_true",
        help="also inspect KWin class/instance/opacity through its scripting journal",
    )
    parser.add_argument("--timeout", type=float, default=10.0)
    args = parser.parse_args()

    if not os.environ.get("DISPLAY"):
        raise Failure("DISPLAY is not set")
    for tool in ("xprop", "xwininfo"):
        if shutil.which(tool) is None:
            raise Failure(f"{tool} is required")
    if not os.path.exists(args.binary):
        raise Failure(f"{args.binary} is missing; build it with cargo build --example desktop_identity")

    command = [
        args.binary,
        "--app-id",
        args.app_id,
        "--instance",
        args.instance,
        "--class",
        args.class_name,
        "--force-x11",
        "--hold-ms",
        str(int((args.timeout + 20) * 1000)),
    ]
    if args.transparent:
        command.append("--transparent")
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        window = None
        ready = None
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            line = process.stdout.readline()
            if not line:
                if process.poll() is not None:
                    raise Failure("desktop_identity exited before reporting READY")
                continue
            if "x11_properties=false" in line:
                raise Failure(
                    "desktop_identity was built without the x11 feature; "
                    "run cargo build -p quasar-window --example desktop_identity"
                )
            match = re.search(r"READY id=(0x[0-9a-fA-F]+)", line)
            if match:
                window = match.group(1)
                ready = line.strip()
                break
        if window is None:
            raise Failure("timed out waiting for READY from desktop_identity")

        def class_matches():
            values = prop_values(window, "WM_CLASS")
            expected = [f'"{args.instance}"', f'"{args.class_name}"']
            if values != expected:
                raise Failure(f"WM_CLASS is {values}, expected {expected}")
            return True

        def pid_matches():
            values = prop_ints(window, "_NET_WM_PID")
            if values != [process.pid]:
                raise Failure(f"_NET_WM_PID is {values}, expected {process.pid}")
            return True

        def normal_type():
            values = prop_values(window, "_NET_WM_WINDOW_TYPE")
            if values is None or "_NET_WM_WINDOW_TYPE_NORMAL" not in values:
                raise Failure(f"_NET_WM_WINDOW_TYPE is {values}")
            return True

        def opacity_matches():
            values = prop_ints(window, "_NET_WM_WINDOW_OPACITY")
            if args.transparent:
                if values is not None:
                    raise Failure(f"transparent window has opacity {values}")
            elif values != [0xFFFFFFFF]:
                raise Failure(f"_NET_WM_WINDOW_OPACITY is {values}, expected full opacity")
            return True

        def region_matches(size):
            values = prop_ints(window, "_NET_WM_OPAQUE_REGION")
            if args.transparent:
                if values is not None:
                    raise Failure(f"transparent window has opaque region {values}")
                return True
            expected = [0, 0, size[0], size[1]]
            if values != expected:
                raise Failure(f"_NET_WM_OPAQUE_REGION is {values}, expected {expected}")
            return True

        poll("WM_CLASS", class_matches, args.timeout)
        poll("_NET_WM_PID", pid_matches, args.timeout)
        poll("_NET_WM_WINDOW_TYPE", normal_type, args.timeout)
        poll("_NET_WM_WINDOW_OPACITY", opacity_matches, args.timeout)
        def initial_region():
            size = window_size(window)
            region_matches(size)
            return size

        initial = poll("initial geometry and opaque region", initial_region, args.timeout)

        if shutil.which("xdotool"):
            resize = lambda width, height: resize_with_xdotool(window, width, height)
            resized_with = "xdotool"
        else:
            resize = lambda width, height: resize_with_xlib(window, width, height)
            resized_with = "libX11"
        resized = []
        for width, height in ((800, 600), (1100, 650), (440, 700)):
            resize(width, height)
            poll(
                f"resized {width}x{height} geometry and opaque region",
                lambda: window_size(window) == (width, height)
                and region_matches((width, height)),
                args.timeout,
            )
            resized.append((width, height))

        kwin = None
        if args.kwin:
            kwin = inspect_kwin(
                args.class_name, args.instance, not args.transparent, args.timeout
            )

        print(
            f"PASS {ready} class={args.class_name} instance={args.instance} "
            f"initial={initial[0]}x{initial[1]} resized={resized} via={resized_with} "
            f"transparent={args.transparent}"
            + (f" kwin={kwin}" if kwin else "")
        )
    finally:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


if __name__ == "__main__":
    try:
        main()
    except (Failure, subprocess.CalledProcessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
