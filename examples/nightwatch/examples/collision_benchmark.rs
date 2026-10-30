//! Local CPU collision/navigation probe. GPU rendering is excluded.
use nightwatch::world::{Input, World};
use std::{hint::black_box, time::Instant};
fn main() {
    let agents: usize = std::env::args()
        .nth(1)
        .map(|s| s.parse().expect("agent count"))
        .unwrap_or(6);
    assert!((1..=24).contains(&agents));
    let mut startup = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        let world = World::new(42);
        startup.push(start.elapsed().as_secs_f64() * 1000.);
        black_box(world);
    }
    let mut world = World::new(42);
    let initial = world.zombies.clone();
    world.zombies = initial.into_iter().cycle().take(agents).collect();
    let mut times = Vec::new();
    for tick in 0..6000 {
        world.health = 100.;
        let input = Input {
            forward: 1.,
            strafe: if tick % 240 < 120 { 0.6 } else { -0.6 },
            look: [1., 0.],
            sprint: tick % 120 < 60,
            jump: tick % 180 == 0,
            ..Default::default()
        };
        let start = Instant::now();
        world.update(input);
        times.push(start.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(f64::total_cmp);
    startup.sort_by(f64::total_cmp);
    let start = Instant::now();
    let mut hits = 0;
    for i in 0..20000 {
        hits += usize::from(
            world
                .raycast_world(
                    [0., 1.65, 10.],
                    [(i as f32 * 0.01).sin(), 0., -(i as f32 * 0.01).cos()],
                    60.,
                )
                .is_some(),
        );
    }
    println!(
        "{}",
        serde_json::json!({"scope":"release CPU; 6000 fixed ticks, living zombies, moving/jumping player, health reset each tick; no GPU","agents":agents,"startup_ms_p50":startup[5],"tick_us_p50":times[3000],"tick_us_p95":times[5700],"tick_us_max":times[5999],"rays":20000,"ray_us_mean":start.elapsed().as_secs_f64()*1e6/20000.,"ray_hits":hits,"props":world.props().len(),"colliders":world.collision().colliders.len(),"navigation_cells":world.navigation().grid().len(),"hash":format!("{:016x}",world.state_hash())})
    );
}
