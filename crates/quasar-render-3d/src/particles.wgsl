// GPU particles: one compute step integrates every slot, one instanced
// draw renders live slots as camera-facing quads (6 vertices per particle).
struct Particle {
    // xyz position, w age in seconds
    position_age: vec4<f32>,
    // xyz velocity, w lifetime (<= 0: empty slot)
    velocity_life: vec4<f32>,
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    // x start size, y end size (world units, full width)
    size: vec4<f32>,
};

struct Step {
    // xyz gravity, w seconds this step
    gravity_dt: vec4<f32>,
    // x linear drag per second, y slot count
    drag_count: vec4<f32>,
};

struct View {
    view_projection: mat4x4<f32>,
    right: vec4<f32>,
    // xyz camera up, w 1 when a sprite texture is bound
    up: vec4<f32>,
};

// --- simulate ---
@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> sim: Step;

@compute @workgroup_size(64)
fn simulate(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= u32(sim.drag_count.y)) {
        return;
    }
    var p = particles[index];
    if (p.velocity_life.w <= 0.0 || p.position_age.w >= p.velocity_life.w) {
        return;
    }
    let dt = sim.gravity_dt.w;
    var velocity = (p.velocity_life.xyz + sim.gravity_dt.xyz * dt) * max(1.0 - sim.drag_count.x * dt, 0.0);
    p.position_age = vec4<f32>(p.position_age.xyz + velocity * dt, p.position_age.w + dt);
    p.velocity_life = vec4<f32>(velocity, p.velocity_life.w);
    particles[index] = p;
}

// --- draw ---
@group(0) @binding(0) var<storage, read> live: array<Particle>;
@group(0) @binding(2) var<uniform> view: View;
@group(0) @binding(3) var sprite: texture_2d<f32>;
@group(0) @binding(4) var sprite_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOutput {
    var out: VertexOutput;
    let p = live[instance];
    let life = p.velocity_life.w;
    if (life <= 0.0 || p.position_age.w >= life) {
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0); // outside the clip volume
        return out;
    }
    let t = clamp(p.position_age.w / life, 0.0, 1.0);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex];
    let half = mix(p.size.x, p.size.y, t) * 0.5;
    let world = p.position_age.xyz + (view.right.xyz * corner.x + view.up.xyz * corner.y) * half;
    out.clip = view.view_projection * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.color = mix(p.color_start, p.color_end, t);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var shape = 1.0;
    if (view.up.w > 0.5) {
        shape = textureSample(sprite, sprite_sampler, in.uv * vec2<f32>(0.5, -0.5) + 0.5).a;
    } else {
        // Procedural soft disc.
        let r2 = dot(in.uv, in.uv);
        shape = clamp(1.0 - r2, 0.0, 1.0);
        shape = shape * shape;
    }
    let alpha = in.color.a * shape;
    if (alpha <= 0.002) {
        discard;
    }
    return vec4<f32>(in.color.rgb, alpha);
}
