# App Stress

Internal integration harness for `quasar-app`. It is not a general engine or a
user-facing demo; it exists to keep the runner's phase, input, timing and
lifecycle contract exercised end to end.

The application combines:

- a fixed 60 Hz simulation `tick` and a variable-rate `frame_update` camera;
- a background `quasar-jobs` task and an asynchronous asset channel;
- explicit action consumption for UI/pause/fullscreen input;
- `Flow::Deferred` pause/resume without replaying paused backlog;
- synthetic headless input (pause at frame 8, resume at frame 12, periodic
  toggle) and deterministic state hashing;
- native resize and fullscreen exercise;
- ordered GPU teardown and a single `shutdown` call.

```sh
cargo run -p app-stress -- --headless --ticks 120
cargo run -p app-stress -- --frames 40
cargo run -p app-stress -- --frames 40 --exercise-window
cargo run -p app-stress -- --headless --ticks 5 --fail-setup   # returns nonzero
```

The headless output includes the setup/suspend/shutdown counts, frame and tick
totals, and an FNV-style hash of the simulation state. Two runs with the same
arguments must print the same hash.

With `--no-default-features` the binary is excluded via `required-features`, so
the workspace feature matrix stays clean while the native path requires
`desktop`.
