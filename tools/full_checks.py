#!/usr/bin/env python3
"""Quasar's full check matrix: the feature-permutation checks, optional
feature tests, native X11, GPU, demo and documentation checks that CI ran
before it was cut down to a last-ditch catch. Run it locally before a release
(Linux; the GPU steps need a Vulkan device and OpenGL).

    python3 tools/full_checks.py              # everything
    python3 tools/full_checks.py --list       # sections and steps
    python3 tools/full_checks.py -s gpu-composition --keep-going

Steps that ran under `xvfb-run` use it when installed, run on the current
display otherwise (except the forced-scale X11 check, which needs Xvfb), and
are skipped when there is neither. Run heavy steps under
a memory limit on a shared desktop, for example
`systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 python3 tools/full_checks.py`.
"""
from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# (section, [(label, bash script)]), in the order the old CI ran them.
SECTIONS: list[list] = [
    [
        "headless",
        [
            [
                "cargo check --locked -p quasar-window --all-targets",
                "cargo check --locked -p quasar-window --all-targets"
            ],
            [
                "cargo check --locked -p quasar-app --all-targets",
                "cargo check --locked -p quasar-app --all-targets"
            ],
            [
                "cargo check --locked -p quasar-app --no-default-features --all-targets",
                "cargo check --locked -p quasar-app --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-jobs --all-targets",
                "cargo check --locked -p quasar-jobs --all-targets"
            ],
            [
                "cargo check --locked -p quasar-jobs --no-default-features --all-targets",
                "cargo check --locked -p quasar-jobs --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-sim --all-targets",
                "cargo check --locked -p quasar-sim --all-targets"
            ],
            [
                "cargo check --locked -p quasar-sim --no-default-features --all-targets",
                "cargo check --locked -p quasar-sim --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-sim --all-targets --features serde,jobs",
                "cargo check --locked -p quasar-sim --all-targets --features serde,jobs"
            ],
            [
                "cargo check --locked -p quasar-sim --all-targets --features serde",
                "cargo check --locked -p quasar-sim --all-targets --features serde"
            ],
            [
                "cargo check --locked -p quasar-sim --all-targets --features jobs",
                "cargo check --locked -p quasar-sim --all-targets --features jobs"
            ],
            [
                "cargo check --locked -p quasar-config --all-targets",
                "cargo check --locked -p quasar-config --all-targets"
            ],
            [
                "cargo check --locked -p quasar-config --no-default-features --all-targets",
                "cargo check --locked -p quasar-config --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-config --no-default-features --all-targets --features toml",
                "cargo check --locked -p quasar-config --no-default-features --all-targets --features toml"
            ],
            [
                "cargo check --locked -p quasar-config --no-default-features --all-targets --features json",
                "cargo check --locked -p quasar-config --no-default-features --all-targets --features json"
            ],
            [
                "cargo check --locked -p quasar-config --all-targets --features storage",
                "cargo check --locked -p quasar-config --all-targets --features storage"
            ],
            [
                "cargo check --locked -p quasar-profile --all-targets",
                "cargo check --locked -p quasar-profile --all-targets"
            ],
            [
                "cargo check --locked -p quasar-profile --no-default-features --all-targets",
                "cargo check --locked -p quasar-profile --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-spatial --all-targets",
                "cargo check --locked -p quasar-spatial --all-targets"
            ],
            [
                "cargo check --locked -p quasar-physics --all-targets",
                "cargo check --locked -p quasar-physics --all-targets"
            ],
            [
                "cargo check --locked -p quasar-physics --no-default-features --all-targets",
                "cargo check --locked -p quasar-physics --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-physics --no-default-features --features dim2 --all-targets",
                "cargo check --locked -p quasar-physics --no-default-features --features dim2 --all-targets"
            ],
            [
                "cargo check --locked -p quasar-physics --no-default-features --features dim3 --all-targets",
                "cargo check --locked -p quasar-physics --no-default-features --features dim3 --all-targets"
            ],
            [
                "cargo check --locked -p quasar-animation --all-targets",
                "cargo check --locked -p quasar-animation --all-targets"
            ],
            [
                "cargo check --locked -p quasar-animation --no-default-features --all-targets",
                "cargo check --locked -p quasar-animation --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-animation --features serde,sprite --all-targets",
                "cargo check --locked -p quasar-animation --features serde,sprite --all-targets"
            ],
            [
                "cargo check --locked -p quasar-nav --all-targets",
                "cargo check --locked -p quasar-nav --all-targets"
            ],
            [
                "cargo check --locked -p quasar-nav --no-default-features --all-targets",
                "cargo check --locked -p quasar-nav --no-default-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-nav --all-features --all-targets",
                "cargo check --locked -p quasar-nav --all-features --all-targets"
            ],
            [
                "cargo check --locked -p quasar-ui --lib",
                "cargo check --locked -p quasar-ui --lib"
            ],
            [
                "cargo check --locked -p quasar-ui --lib --no-default-features",
                "cargo check --locked -p quasar-ui --lib --no-default-features"
            ],
            [
                "cargo check --locked -p quasar-ui --lib --features input,config,serde",
                "cargo check --locked -p quasar-ui --lib --features input,config,serde"
            ],
            [
                "cargo check --locked -p quasar-ui --lib --features text",
                "cargo check --locked -p quasar-ui --lib --features text"
            ],
            [
                "cargo check --locked -p quasar-text --lib",
                "cargo check --locked -p quasar-text --lib"
            ],
            [
                "cargo check --locked -p quasar-text --lib --no-default-features",
                "cargo check --locked -p quasar-text --lib --no-default-features"
            ],
            [
                "cargo check --locked -p quasar-text --lib --features serde",
                "cargo check --locked -p quasar-text --lib --features serde"
            ],
            [
                "cargo check --locked -p quasar-input --features gilrs",
                "cargo check --locked -p quasar-input --features gilrs"
            ],
            [
                "cargo check --locked -p quasar-assets --features serde,storage",
                "cargo check --locked -p quasar-assets --features serde,storage"
            ],
            [
                "cargo check --locked -p quasar-assets --features async,hot-reload",
                "cargo check --locked -p quasar-assets --features async,hot-reload"
            ],
            [
                "cargo check --locked -p quasar-diagnostics --features timing,serde",
                "cargo check --locked -p quasar-diagnostics --features timing,serde"
            ],
            [
                "cargo test --locked -p quasar-input -p pong -p quasar-tilemap -p tilequest -p quasar-audio --no-default-features",
                "cargo test --locked -p quasar-input -p pong -p quasar-tilemap -p tilequest -p quasar-audio --no-default-features"
            ],
            [
                "cargo test --locked -p quasar-audio --no-default-features --features settings --test settings",
                "cargo test --locked -p quasar-audio --no-default-features --features settings --test settings"
            ],
            [
                "cargo test --locked -p quasar-workspace-checks",
                "cargo test --locked -p quasar-workspace-checks"
            ],
            [
                "python tools/verify_pong.py",
                "python tools/verify_pong.py"
            ],
            [
                "python tools/verify_tilemap.py",
                "python tools/verify_tilemap.py"
            ]
        ]
    ],
    [
        "audio-native",
        [
            [
                "cargo check --locked -p quasar-audio --all-features",
                "cargo check --locked -p quasar-audio --all-features"
            ]
        ]
    ],
    [
        "desktop",
        [
            [
                "cargo fmt --all -- --check",
                "cargo fmt --all -- --check"
            ],
            [
                "cargo test --workspace --locked",
                "cargo test --workspace --locked"
            ],
            [
                "Test optional feature layers",
                "cargo test --locked -p quasar-input --features gilrs,diagnostics\ncargo test --locked -p quasar-assets --features serde,storage,diagnostics\ncargo test --locked -p quasar-assets --features async,hot-reload,serde,storage,diagnostics\ncargo test --locked -p quasar-diagnostics --features timing,serde\ncargo test --locked -p quasar-app\ncargo test --locked -p quasar-app --no-default-features\ncargo test --locked -p quasar-jobs\ncargo test --locked -p quasar-jobs --features diagnostics\ncargo test --locked -p quasar-sim\ncargo test --locked -p quasar-sim --features serde,jobs\ncargo test --locked -p quasar-config\ncargo test --locked -p quasar-config --no-default-features\ncargo test --locked -p quasar-config --features storage\ncargo test --locked -p quasar-config --no-default-features --features toml\ncargo test --locked -p quasar-config --no-default-features --features json\ncargo test --locked -p quasar-profile\ncargo test --locked -p quasar-profile --no-default-features\ncargo test --locked -p quasar-spatial\ncargo test --locked -p quasar-physics\ncargo test --locked -p quasar-physics --features dim3\ncargo test --locked -p quasar-physics --no-default-features --features dim2\ncargo test --locked -p quasar-physics --no-default-features --features dim3\ncargo test --locked -p quasar-physics --no-default-features --features dim2,dynamics\ncargo test --locked -p quasar-physics --no-default-features --features dim2,dim3,dynamics,spatial\ncargo test --locked -p quasar-animation\ncargo test --locked -p quasar-animation --all-features\ncargo test --locked -p quasar-nav\ncargo test --locked -p quasar-nav --all-features\ncargo test --locked -p quasar-ui\ncargo test --locked -p quasar-ui --no-default-features\ncargo test --locked -p quasar-ui --all-features\ncargo test --locked -p quasar-ui --features input\ncargo test --locked -p quasar-ui --features config\ncargo test --locked -p quasar-ui --features text\ncargo test --locked -p quasar-ui --features text-render2d\ncargo test --locked -p quasar-render-2d --features text\ncargo test --locked -p quasar-text\ncargo test --locked -p quasar-text --no-default-features\ncargo test --locked -p quasar-text --features serde\ncargo test --locked -p starfall\ncargo test --locked -p starfall --no-default-features\ncargo test --locked -p falling-sand\ncargo test --locked -p falling-sand --no-default-features\ncargo test --locked -p physics-playground\ncargo test --locked -p physics-playground --no-default-features\ncargo check --locked -p quasar-window --features diagnostics\ncargo check --locked -p quasar-audio --features diagnostics\ncargo check --locked -p quasar-frame --features diagnostics\ncargo clippy --locked -p quasar-input --all-targets --features gilrs,diagnostics -- -D warnings\ncargo clippy --locked -p quasar-assets --all-targets --features async,hot-reload,serde,storage,diagnostics -- -D warnings\ncargo clippy --locked -p quasar-diagnostics --all-targets --features timing,serde -- -D warnings\ncargo clippy --locked -p quasar-app --all-targets -- -D warnings\ncargo clippy --locked -p quasar-app --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-jobs --all-targets -- -D warnings\ncargo clippy --locked -p quasar-jobs --all-targets --features diagnostics -- -D warnings\ncargo clippy --locked -p quasar-sim --all-targets -- -D warnings\ncargo clippy --locked -p quasar-sim --all-targets --features serde,jobs -- -D warnings\ncargo clippy --locked -p quasar-config --all-targets -- -D warnings\ncargo clippy --locked -p quasar-config --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-config --all-targets --features storage -- -D warnings\ncargo clippy --locked -p quasar-config --all-targets --no-default-features --features toml -- -D warnings\ncargo clippy --locked -p quasar-config --all-targets --no-default-features --features json -- -D warnings\ncargo clippy --locked -p quasar-profile --all-targets -- -D warnings\ncargo clippy --locked -p quasar-profile --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-spatial --all-targets -- -D warnings\ncargo clippy --locked -p quasar-physics --all-targets -- -D warnings\ncargo clippy --locked -p quasar-physics --all-targets --features dim3 -- -D warnings\ncargo clippy --locked -p quasar-physics --all-targets --all-features -- -D warnings\ncargo clippy --locked -p quasar-physics --all-targets --no-default-features --features dim2 -- -D warnings\ncargo clippy --locked -p quasar-physics --all-targets --no-default-features --features dim3,spatial -- -D warnings\ncargo clippy --locked -p quasar-physics --all-targets --no-default-features --features dim2,dim3,dynamics,spatial -- -D warnings\ncargo clippy --locked -p quasar-animation --all-targets --all-features -- -D warnings\ncargo clippy --locked -p quasar-animation --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-nav --all-targets --all-features -- -D warnings\ncargo clippy --locked -p quasar-nav --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-ui --all-targets --all-features -- -D warnings\ncargo clippy --locked -p quasar-ui --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-text --all-targets --all-features -- -D warnings\ncargo clippy --locked -p quasar-text --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-render-2d --all-targets --features text -- -D warnings\ncargo clippy --locked -p starfall --all-targets -- -D warnings\ncargo clippy --locked -p starfall --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p falling-sand --all-targets -- -D warnings\ncargo clippy --locked -p falling-sand --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p physics-playground --all-targets -- -D warnings\ncargo clippy --locked -p physics-playground --all-targets --no-default-features -- -D warnings"
            ],
            [
                "Run native X11 identity and resize tests",
                "xvfb-run -a sh -c 'QUASAR_X11_TESTS=1 cargo test --locked -p quasar-window --test x11_native -- --test-threads=1'\nxvfb-run -a sh -c 'QUASAR_X11_TESTS=1 cargo test --locked -p quasar-window --features diagnostics --test x11_native -- --test-threads=1'"
            ],
            [
                "Verify native X11 properties with xprop and xdotool",
                "cargo build --locked -p quasar-window --example desktop_identity\nxvfb-run -a python3 tools/x11_window_smoke.py --timeout 15\nxvfb-run -a python3 tools/x11_window_smoke.py --transparent --timeout 15"
            ],
            [
                "Verify DPI-scaled physical metadata",
                "xvfb-run -a env WINIT_X11_SCALE_FACTOR=2 QUASAR_X11_TESTS=1 cargo test --locked -p quasar-window --test x11_native -- --test-threads=1\nxvfb-run -a env WINIT_X11_SCALE_FACTOR=2 python3 tools/x11_window_smoke.py --timeout 15"
            ],
            [
                "cargo clippy --workspace --all-targets --locked -- -D warnings",
                "cargo clippy --workspace --all-targets --locked -- -D warnings"
            ],
            [
                "cargo clippy -p quasar-audio --all-targets --all-features --locked -- -D warnings",
                "cargo clippy -p quasar-audio --all-targets --all-features --locked -- -D warnings"
            ],
            [
                "Check backend feature separation",
                "cargo check --locked -p vulkanic-gal --no-default-features\ncargo check --locked -p vulkanic-gal --no-default-features --features vulkan\ncargo check --locked -p vulkanic-gal --no-default-features --features opengl\ncargo check --locked -p vulkanic-gal --no-default-features --features wgpu\ncargo check --locked -p vulkanic-gal --no-default-features --features test-support\ncargo check --locked -p quasar-frame --no-default-features\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu,vulkan\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu,opengl\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu,wgpu\ncargo check --locked -p quasar-render-3d --no-default-features --features vulkan\ncargo check --locked -p quasar-render-3d --no-default-features --features opengl\ncargo check --locked -p quasar-window --no-default-features\ncargo check --locked -p quasar-input --no-default-features\ncargo check --locked -p quasar-assets --no-default-features\ncargo check --locked -p quasar-diagnostics --no-default-features\ncargo check --locked -p quasar-app --no-default-features\ncargo check --locked -p quasar-jobs --no-default-features\ncargo check --locked -p quasar-sim --no-default-features\ncargo check --locked -p quasar-config --no-default-features\ncargo check --locked -p quasar-profile --no-default-features\ncargo check --locked -p quasar-spatial --no-default-features\ncargo check --locked -p quasar-physics --no-default-features\ncargo check --locked -p quasar-physics --no-default-features --features dim2\ncargo check --locked -p quasar-physics --no-default-features --features dim3\ncargo check --locked -p quasar-animation --no-default-features\ncargo check --locked -p quasar-nav --no-default-features\ncargo check --locked -p starfall --no-default-features\ncargo check --locked -p falling-sand --no-default-features\ncargo check --locked -p physics-playground --no-default-features\ncargo clippy --locked -p quasar-window --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-sim --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-jobs --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-app --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-assets --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p quasar-diagnostics --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p vulkanic-gal --all-targets --no-default-features -- -D warnings\ncargo clippy --locked -p vulkanic-gal --all-targets --no-default-features --features opengl -- -D warnings\ncargo clippy --locked -p vulkanic-gal --all-targets --no-default-features --features wgpu -- -D warnings\ncargo clippy --locked -p vulkanic-gal --all-targets --features opengl,wgpu -- -D warnings\ncargo clippy --locked -p vulkanic-gal --all-targets --features opengl,test-support -- -D warnings\ncargo clippy --locked -p quasar-render-2d --all-targets --no-default-features --features gpu -- -D warnings\ncargo clippy --locked -p quasar-render-2d --all-targets --no-default-features --features gpu,opengl -- -D warnings\ncargo clippy --locked -p quasar-render-3d --all-targets --no-default-features --features opengl -- -D warnings"
            ],
            [
                "Verify spatial/physics/nav dependency isolation",
                "cargo tree --locked -p quasar-spatial -e normal | (! grep -Eq 'rapier[23]d|parry[23]d')\ncargo tree --locked -p quasar-physics --no-default-features -e normal | (! grep -Eq 'rapier[23]d|parry[23]d')\ncargo tree --locked -p quasar-physics --no-default-features --features dim2 -e normal | grep -q 'rapier2d'\n! cargo tree --locked -p quasar-physics --no-default-features --features dim2 -e normal | grep -q 'rapier3d'\ncargo tree --locked -p quasar-physics --no-default-features --features dim3 -e normal | grep -q 'rapier3d'\n! cargo tree --locked -p quasar-physics --no-default-features --features dim3 -e normal | grep -q 'rapier2d'\ncargo tree --locked -p quasar-nav --no-default-features --features tilemap -e normal | (! grep -Eq 'quasar-render-2d|vulkanic-gal')\ncargo tree --locked -p quasar-animation --features sprite -e normal | (! grep -Eq 'vulkanic-gal|shaderc')\ncargo tree --locked -p quasar-ui -e normal | (! grep -Eq 'quasar-render-2d|vulkanic-gal|quasar-input|quasar-config')\ncargo tree --locked -p quasar-ui --features render2d -e normal | grep -q 'quasar-render-2d'\ncargo tree --locked -p quasar-text -e normal | (! grep -Eq 'quasar-render-2d|vulkanic-gal|quasar-window|quasar-input|quasar-ui')\ncargo tree --locked -p quasar-render-2d -e normal | (! grep -q 'quasar-text')"
            ],
            [
                "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked",
                "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked"
            ]
        ]
    ],
    [
        "retro-2d",
        [
            [
                "Headless indexed rendering and incremental tilemap feature matrix",
                "cargo test --locked -p quasar-render-2d --no-default-features\ncargo test --locked -p quasar-render-2d --no-default-features --features assets\ncargo test --locked -p quasar-tilemap --no-default-features\ncargo test --locked -p quasar-tilemap --no-default-features --features render-2d\ncargo test --locked -p quasar-tilemap --no-default-features --features png\ncargo test --locked -p tilequest --no-default-features\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu,vulkan\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu,wgpu\ncargo check --locked -p quasar-render-2d --no-default-features --features gpu,opengl"
            ],
            [
                "Native Vulkan indexed pixels, palette lifetimes and shared atlases",
                "VULKANIC_BACKEND=vulkan cargo test --locked -p quasar-render-2d --all-features -p quasar-composition -p tilequest --lib -- --include-ignored --test-threads=1"
            ],
            [
                "wgpu indexed pixels, palette lifetimes and shared atlases",
                "VULKANIC_BACKEND=wgpu cargo test --locked -p quasar-render-2d --all-features -p quasar-composition -p tilequest --lib -- --include-ignored --test-threads=1"
            ],
            [
                "CPU-only retro example and GPU example parity",
                "cargo run --locked -p tilequest --no-default-features --example retro -- /tmp/quasar-retro-cpu.png\ncargo run --locked -p tilequest --example retro -- --gpu /tmp/quasar-retro-gpu.png"
            ]
        ]
    ],
    [
        "gpu-composition",
        [
            [
                "Run GPU lifetime and pixel tests",
                "xvfb-run -a cargo test --locked -p quasar-composition -p quasar-render-2d -p tilequest -p vulkanic-gal -p pong --lib -- --include-ignored --test-threads=1"
            ],
            [
                "Test quasar-ui shared-GAL rendering",
                "xvfb-run -a cargo test --locked -p quasar-ui --all-features --test render -- --ignored --test-threads=1"
            ],
            [
                "Run OpenGL backend and cross-backend conformance tests",
                "xvfb-run -a cargo test --locked -p vulkanic-gal --no-default-features --features vulkan,opengl --lib -- --include-ignored --test-threads=1"
            ],
            [
                "Exercise native resize and failed-frame recovery",
                "xvfb-run -a cargo run --locked -p quasar-composition -- --frames 40 --exercise-window"
            ],
            [
                "Exercise native Pong window recovery",
                "xvfb-run -a cargo run --locked -p pong -- --autoplay --no-save --smoke-frames 30 --exercise-window"
            ],
            [
                "Exercise native tilemap window recovery",
                "xvfb-run -a cargo run --locked -p tilequest -- --autoplay --smoke-frames 40 --exercise-window"
            ],
            [
                "Exercise native quasar-app lifecycle",
                "xvfb-run -a cargo run --locked -p quasar-app --example desktop"
            ],
            [
                "Exercise quasar-app headless lifecycle",
                "cargo run --locked -p quasar-app --example desktop -- --headless"
            ],
            [
                "Exercise runner stress harness",
                "cargo run --locked -p app-stress -- --headless --ticks 120\nxvfb-run -a cargo run --locked -p app-stress -- --frames 40 --exercise-window"
            ],
            [
                "Exercise deterministic simulation replay",
                "cargo run --locked -p quasar-sim --example replay"
            ],
            [
                "Exercise Starfall headless replay and screenshot",
                "cargo run --locked -p starfall -- --headless --ticks 120 --record /tmp/starfall.json\ncargo run --locked -p starfall -- --headless --replay /tmp/starfall.json\ncargo run --locked -p starfall -- --headless --ticks 30 --screenshot /tmp/starfall.png"
            ],
            [
                "Test shared 3D+2D Starfall rendering",
                "xvfb-run -a cargo test --locked -p starfall --test render -- --ignored --test-threads=1"
            ],
            [
                "Exercise Starfall benchmark",
                "xvfb-run -a cargo run --locked -p starfall -- --benchmark --benchmark-frames 30 --benchmark-file /tmp/starfall-profile.json"
            ],
            [
                "Exercise Falling Sand headless simulation and screenshot",
                "cargo run --locked -p falling-sand -- --headless --ticks 240 --autoplay\ncargo run --locked -p falling-sand -- --headless --ticks 120 --autoplay --screenshot /tmp/falling-sand.png"
            ],
            [
                "Test Falling Sand shared-GAL rendering",
                "xvfb-run -a cargo test --locked -p falling-sand --test sand -- --ignored --test-threads=1"
            ],
            [
                "Exercise Falling Sand benchmark",
                "xvfb-run -a cargo run --locked -p falling-sand -- --benchmark --benchmark-frames 20 --benchmark-file /tmp/falling-sand-profile.json"
            ],
            [
                "Exercise Physics Playground headless simulation and screenshot",
                "cargo run --locked -p physics-playground -- --headless --ticks 240 --autoplay\ncargo run --locked -p physics-playground -- --headless --ticks 120 --autoplay --screenshot /tmp/physics-playground.png"
            ],
            [
                "Test Physics Playground shared-GAL rendering",
                "xvfb-run -a cargo test --locked -p physics-playground --test playground -- --ignored --test-threads=1"
            ],
            [
                "Exercise Physics Playground benchmark",
                "xvfb-run -a cargo run --locked -p physics-playground -- --benchmark --benchmark-frames 20 --benchmark-file /tmp/physics-playground-profile.json"
            ],
            [
                "Exercise native software-framebuffer presentation",
                "xvfb-run -a cargo run --locked -p framebuffer -- --frames 30 --exercise-window"
            ],
            [
                "Render a headless software-framebuffer screenshot",
                "xvfb-run -a cargo run --locked -p framebuffer -- --headless --screenshot /tmp/framebuffer.png"
            ],
            [
                "Render deterministic offline audio",
                "cargo run --locked -p audio-example -- --offline /tmp/audio-example.wav --benchmark /tmp/audio-benchmark.json"
            ]
        ]
    ]
]


def prepare(script: str) -> tuple[str | None, str]:
    """`script` runnable here, or None and the reason it is skipped."""
    # CI's `python` is this interpreter.
    script = "\n".join(
        sys.executable + line[len("python"):] if line.startswith("python ") else line
        for line in script.split("\n")
    )
    if "xvfb-run" not in script or shutil.which("xvfb-run"):
        return script, ""
    # A forced X11 scale factor is only meaningful on Xvfb's unmanaged display;
    # a desktop window manager constrains the scaled resizes it checks.
    if "WINIT_X11_SCALE_FACTOR" in script:
        return None, "needs xvfb-run"
    if os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"):
        return script.replace("xvfb-run -a ", ""), ""
    return None, "no display"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("-s", "--section", action="append", help="run only these sections")
    parser.add_argument("--keep-going", action="store_true", help="continue after a failing step")
    parser.add_argument("--list", action="store_true", help="list sections and steps")
    args = parser.parse_args()
    names = [name for name, _ in SECTIONS]
    for wanted in args.section or []:
        if wanted not in names:
            parser.error(f"unknown section {wanted!r}; choose from {', '.join(names)}")
    failed: list[str] = []
    skipped: list[str] = []
    for name, steps in SECTIONS:
        if args.section and name not in args.section:
            continue
        for label, script in steps:
            if args.list:
                print(f"{name}: {label}")
                continue
            runnable, reason = prepare(script)
            if runnable is None:
                skipped.append(f"{name}: {label} ({reason})")
                print(f"== skip ({reason}) {name}: {label}", flush=True)
                continue
            print(f"== {name}: {label}", flush=True)
            result = subprocess.run(["bash", "-euo", "pipefail", "-c", runnable], cwd=ROOT)
            if result.returncode != 0:
                failed.append(f"{name}: {label}")
                print(f"!! failed ({result.returncode}): {name}: {label}", flush=True)
                if not args.keep_going:
                    break
        if failed and not args.keep_going:
            break
    if args.list:
        return 0
    for step in skipped:
        print(f"skipped: {step}")
    for step in failed:
        print(f"FAILED: {step}")
    print("full checks passed" if not failed else f"{len(failed)} step(s) failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
