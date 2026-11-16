// GPU frustum culling: one thread per draw record writes its indirect
// arguments, with instance_count 0 for culled records. Draw order is the
// record order, so culling never reorders geometry.

struct CullRecord {
    min: vec4<f32>,
    max: vec4<f32>,
    // x: first vertex, y: vertex count, z: instance, w: grid
    draw: vec4<u32>,
};
struct GridData {
    model: mat4x4<f32>,
};
struct Planes {
    planes: array<vec4<f32>, 6>,
    // x: record count, y: plane count
    info: vec4<u32>,
};
struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
};

@group(0) @binding(0) var<storage, read> records: array<CullRecord>;
@group(0) @binding(1) var<storage, read> grids: array<GridData>;
@group(0) @binding(2) var<uniform> frustum: Planes;
@group(0) @binding(3) var<storage, read_write> args: array<DrawArgs>;

@compute @workgroup_size(64)
fn cull(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= frustum.info.x) {
        return;
    }
    let r = records[i];
    let model = grids[r.draw.w].model;
    var inside = true;
    for (var p = 0u; p < frustum.info.y; p++) {
        let plane = frustum.planes[p];
        var any_in = false;
        for (var c = 0u; c < 8u; c++) {
            let local = vec3<f32>(
                select(r.min.x, r.max.x, (c & 1u) != 0u),
                select(r.min.y, r.max.y, (c & 2u) != 0u),
                select(r.min.z, r.max.z, (c & 4u) != 0u),
            );
            let world = (model * vec4<f32>(local, 1.0)).xyz;
            if (dot(plane.xyz, world) + plane.w >= 0.0) {
                any_in = true;
                break;
            }
        }
        if (!any_in) {
            inside = false;
            break;
        }
    }
    var out: DrawArgs;
    out.vertex_count = r.draw.y;
    out.instance_count = select(0u, 1u, inside);
    out.first_vertex = r.draw.x;
    out.first_instance = r.draw.z;
    args[i] = out;
}
