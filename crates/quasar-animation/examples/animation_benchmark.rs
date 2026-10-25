//! Local animation sampling/state-machine workload probe.
//!
//! ```sh
//! cargo run --release -p quasar-animation --example animation_benchmark
//! ```
use quasar_animation::{
    Clip, Condition, Interpolation, Keyframe, LoopMode, Marker, ParameterValue, Player, Pose,
    StateMachine, Track, Transition, Value,
};
use std::{hint::black_box, sync::Arc, time::Instant};

fn main() {
    let mut tracks = Vec::new();
    for track in 0..128 {
        let keyframes = (0..128)
            .map(|key| {
                Keyframe::new(
                    key as f32 / 127.0,
                    Value::Float((track * key) as f32 * 0.001),
                )
            })
            .collect();
        tracks.push(
            Track::new(
                format!("property.{track}"),
                Interpolation::Linear,
                keyframes,
            )
            .unwrap(),
        );
    }
    let clip = Arc::new(Clip::new("bench", 1.0, tracks, vec![]).unwrap());
    let mut player = Player::new(clip.clone(), LoopMode::Loop);

    let start = Instant::now();
    let mut checksum = 0.0f32;
    for index in 0..200_000 {
        player.advance(1.0 / 240.0).unwrap();
        let pose = player.sample();
        checksum += pose.f32("property.0").unwrap_or(0.0) + index as f32 * 1e-12;
        black_box(&pose);
    }
    println!(
        "sample tracks=128 keys=128 samples=200000 elapsed_ms={:.3} checksum={checksum:.3}",
        start.elapsed().as_secs_f64() * 1_000.0
    );

    let idle = Arc::new(Clip::new("idle", 1.0, vec![], vec![Marker::new(0.0, "loop")]).unwrap());
    let run = Arc::new(Clip::new("run", 0.8, vec![], vec![Marker::new(0.0, "loop")]).unwrap());
    let mut machine = StateMachine::new();
    let layer = machine.add_layer("base");
    machine
        .add_state(layer, "idle", idle, LoopMode::Loop, 1.0)
        .unwrap();
    machine
        .add_state(layer, "run", run, LoopMode::Loop, 1.5)
        .unwrap();
    machine
        .add_transition(
            layer,
            Transition::new(
                Some("idle".into()),
                "run",
                0.15,
                Condition::Float {
                    parameter: "speed".into(),
                    compare: quasar_animation::Compare::Greater,
                    value: 0.1,
                },
            )
            .unwrap(),
        )
        .unwrap();
    machine.set_parameter("speed", ParameterValue::Float(0.0));
    let mut reused = Pose::new();
    let start = Instant::now();
    for _ in 0..200_000 {
        player.advance(1.0 / 240.0).unwrap();
        player.sample_into(&mut reused);
        checksum += reused.f32("property.0").unwrap_or(0.0);
        black_box(&reused);
    }
    println!(
        "sample-into tracks=128 keys=128 samples=200000 elapsed_ms={:.3} checksum={checksum:.3}",
        start.elapsed().as_secs_f64() * 1_000.0
    );

    let start = Instant::now();
    for index in 0..200_000 {
        if index % 1_000 == 0 {
            machine.set_parameter(
                "speed",
                ParameterValue::Float(if index % 2_000 == 0 { 0.0 } else { 1.0 }),
            );
        }
        machine.update(1.0 / 240.0).unwrap();
        black_box(machine.pose());
    }
    println!(
        "state-machine updates=200000 elapsed_ms={:.3} state={:?}",
        start.elapsed().as_secs_f64() * 1_000.0,
        machine.current_state(layer)
    );
}
