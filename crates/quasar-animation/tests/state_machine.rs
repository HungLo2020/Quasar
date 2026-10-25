use quasar_animation::{
    Clip, Compare, Condition, Interpolation, Keyframe, LoopMode, Marker, ParameterValue,
    StateMachine, Track, Transition, Value,
};
use std::sync::Arc;

fn clip(name: &str, value: f32) -> Arc<Clip> {
    Arc::new(
        Clip::new(
            name,
            1.0,
            vec![
                Track::new(
                    "x",
                    Interpolation::Step,
                    vec![Keyframe::new(0.0, Value::Float(value))],
                )
                .unwrap(),
            ],
            vec![Marker::new(0.5, "half")],
        )
        .unwrap(),
    )
}

fn machine() -> (StateMachine, usize) {
    let mut machine = StateMachine::new();
    let layer = machine.add_layer("base");
    machine
        .add_state(layer, "idle", clip("idle", 0.0), LoopMode::Loop, 1.0)
        .unwrap();
    machine
        .add_state(layer, "run", clip("run", 10.0), LoopMode::Loop, 1.0)
        .unwrap();
    machine
        .add_transition(
            layer,
            Transition::new(
                Some("idle".into()),
                "run",
                0.2,
                Condition::Float {
                    parameter: "speed".into(),
                    compare: Compare::Greater,
                    value: 0.5,
                },
            )
            .unwrap(),
        )
        .unwrap();
    (machine, layer)
}

#[test]
fn transitions_crossfade_and_switch_state() {
    let (mut machine, layer) = machine();
    assert_eq!(machine.current_state(layer), Some("idle"));
    machine.set_parameter("speed", ParameterValue::Float(0.0));
    machine.update(0.05).unwrap();
    assert_eq!(machine.current_state(layer), Some("idle"));

    machine.set_parameter("speed", ParameterValue::Float(1.0));
    machine.update(0.01).unwrap();
    assert_eq!(machine.current_state(layer), Some("run"));
    assert!(machine.is_fading(layer));
    let blended = machine.value("x").unwrap().as_f32().unwrap();
    assert!((0.0..10.0).contains(&blended), "blend={blended}");
    machine.update(0.3).unwrap();
    assert!(!machine.is_fading(layer));
    assert_eq!(machine.value("x"), Some(Value::Float(10.0)));
}

#[test]
fn triggers_fire_once_and_conditions_are_validated() {
    let mut machine = StateMachine::new();
    let layer = machine.add_layer("base");
    machine
        .add_state(layer, "idle", clip("idle", 0.0), LoopMode::Loop, 1.0)
        .unwrap();
    machine
        .add_state(layer, "attack", clip("attack", 5.0), LoopMode::Loop, 1.0)
        .unwrap();
    machine
        .add_transition(
            layer,
            Transition::new(
                Some("idle".into()),
                "attack",
                0.0,
                Condition::Trigger("fire".into()),
            )
            .unwrap(),
        )
        .unwrap();
    machine.trigger("fire");
    machine.update(0.01).unwrap();
    assert_eq!(machine.current_state(layer), Some("attack"));
    assert_eq!(machine.parameter("fire"), Some(ParameterValue::Bool(false)));
    assert!(
        !Condition::Float {
            parameter: "x".into(),
            compare: Compare::Greater,
            value: f32::NAN
        }
        .is_valid()
    );
    assert!(Transition::new(None, "missing", 0.0, Condition::Always).is_ok());
}

#[test]
fn layers_blend_by_normalized_weight() {
    let mut machine = StateMachine::new();
    let base = machine.add_layer("base");
    let overlay = machine.add_layer("overlay");
    machine
        .add_state(base, "a", clip("a", 0.0), LoopMode::Loop, 1.0)
        .unwrap();
    machine
        .add_state(overlay, "b", clip("b", 10.0), LoopMode::Loop, 1.0)
        .unwrap();
    machine.set_layer_weight(base, 1.0).unwrap();
    machine.set_layer_weight(overlay, 1.0).unwrap();
    machine.update(0.01).unwrap();
    assert_eq!(machine.value("x"), Some(Value::Float(5.0)));
    machine.set_layer_weight(overlay, 0.0).unwrap();
    assert_eq!(machine.value("x"), Some(Value::Float(0.0)));
    assert_eq!(machine.layer_name(base), Some("base"));
    assert!(machine.set_layer_weight(99, 1.0).is_err());
}

#[test]
fn state_machine_updates_are_deterministic() {
    fn run() -> (Option<&'static str>, Option<Value>, f32) {
        let (mut first, layer) = machine();
        let mut values = Vec::new();
        for tick in 0..200u32 {
            if tick.is_multiple_of(37) {
                first.set_parameter(
                    "speed",
                    ParameterValue::Float(if tick % 2 == 0 { 1.0 } else { 0.0 }),
                );
            }
            first.update(1.0 / 60.0).unwrap();
            if let Some(value) = first.value("x") {
                values.push(value);
            }
        }
        // Leak the state name only for the test comparison; both sides produce
        // the same deterministic sequence.
        let name = match first.current_state(layer) {
            Some("idle") => Some("idle"),
            Some("run") => Some("run"),
            _ => None,
        };
        let last = values.last().copied();
        let time = first.state_time(layer).unwrap_or(-1.0);
        (name, last, time)
    }
    assert_eq!(run(), run());
}
