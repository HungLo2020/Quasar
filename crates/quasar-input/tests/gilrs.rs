#![cfg(feature = "gilrs")]
//! Native backend smoke that does not require an attached controller.

use quasar_input::{GamepadHub, GilrsGamepads, PlayerId};

#[test]
fn native_backend_opens_and_polls_without_a_controller() {
    let mut backend = GilrsGamepads::new().expect("open gilrs backend");
    let mut hub = GamepadHub::new();
    hub.add_player(PlayerId::new(0)).unwrap();
    let _ = backend.connected(&mut hub);
    backend.poll(&mut hub).expect("poll gilrs backend");
    assert!(hub.devices().len() < 1024);
}
