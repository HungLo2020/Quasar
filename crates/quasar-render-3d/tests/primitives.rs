//! CPU mesh primitives: closed, counter-clockwise outward winding, normals
//! and UVs. No GPU needed.
use quasar_render_3d::{Mat4, MeshData};

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Every triangle faces away from the centroid, agrees with its vertex
/// normals, and the mesh encloses a positive volume (divergence theorem).
fn assert_closed_outward(name: &str, data: &MeshData, expected_volume: Option<f32>) {
    assert!(
        !data.indices.is_empty() && data.indices.len().is_multiple_of(3),
        "{name}"
    );
    assert!(
        data.indices
            .iter()
            .all(|i| (*i as usize) < data.vertices.len()),
        "{name}"
    );
    let mut volume = 0.0f32;
    for (index, tri) in data.indices.chunks_exact(3).enumerate() {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| data.vertices[i as usize]);
        let face = cross(sub(b.position, a.position), sub(c.position, a.position));
        let area = dot(face, face).sqrt();
        if area < 1e-7 {
            continue;
        }
        let centroid = [
            (a.position[0] + b.position[0] + c.position[0]) / 3.0,
            (a.position[1] + b.position[1] + c.position[1]) / 3.0,
            (a.position[2] + b.position[2] + c.position[2]) / 3.0,
        ];
        assert!(
            dot(face, centroid) > -1e-5,
            "{name} triangle {index} faces inward"
        );
        for v in [a, b, c] {
            assert!(
                dot(face, v.normal) > 0.0,
                "{name} triangle {index} normal mismatch"
            );
            assert!(
                (dot(v.normal, v.normal) - 1.0).abs() < 1e-4,
                "{name} unnormalized"
            );
            assert!(v.uv.iter().all(|u| (0.0..=1.0).contains(u)), "{name} uv");
        }
        volume += dot(a.position, cross(b.position, c.position)) / 6.0;
    }
    assert!(volume > 0.0, "{name} volume {volume}");
    if let Some(expected) = expected_volume {
        assert!(
            (volume - expected).abs() < expected * 0.03,
            "{name} volume {volume} vs {expected}"
        );
    }
}

#[test]
fn closed_primitives_wind_outward_and_enclose_their_volume() {
    let pi = std::f32::consts::PI;
    assert_closed_outward("cube", &MeshData::cube(), Some(8.0));
    assert_closed_outward("cuboid", &MeshData::cuboid([1.0, 0.5, 2.0]), Some(8.0));
    assert_closed_outward(
        "sphere",
        &MeshData::sphere(2.0, 24, 48),
        Some(4.0 / 3.0 * pi * 8.0),
    );
    assert_closed_outward(
        "cylinder",
        &MeshData::cylinder(1.0, 1.5, 64),
        Some(pi * 3.0),
    );
    assert_closed_outward("cone", &MeshData::cone(1.0, 3.0, 64), Some(pi));
    assert_closed_outward("frustum", &MeshData::frustum(2.0, 1.0, 1.0, 64), None);
    // Degenerate segment counts are clamped rather than producing nothing.
    assert_closed_outward("triangle prism", &MeshData::cylinder(1.0, 1.0, 0), None);
}

#[test]
fn plane_faces_up_and_bounds_match_extents() {
    let plane = MeshData::plane(2.0, 3.0, 4);
    assert_eq!(plane.vertices.len(), 25);
    assert_eq!(plane.indices.len(), 4 * 4 * 6);
    for tri in plane.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| plane.vertices[i as usize].position);
        assert!(cross(sub(b, a), sub(c, a))[1] > 0.0);
    }
    assert_eq!(plane.bounds(), Some(([-2.0, 0.0, -3.0], [2.0, 0.0, 3.0])));
    let (min, max) = MeshData::sphere(1.5, 8, 16).bounds().unwrap();
    assert!((max[1] - 1.5).abs() < 1e-5 && (min[1] + 1.5).abs() < 1e-5);
    assert_eq!(MeshData::default().bounds(), None);
}

#[test]
fn transforms_keep_winding_and_normals_even_when_mirrored() {
    let moved = MeshData::cube()
        .transformed(Mat4::translation(10.0, 0.0, 0.0) * Mat4::scale(1.0, 2.0, 1.0));
    assert_eq!(moved.bounds(), Some(([9.0, -2.0, -1.0], [11.0, 2.0, 1.0])));
    let back = moved.transformed(Mat4::translation(-10.0, 0.0, 0.0));
    assert_closed_outward("scaled cube", &back, Some(16.0));
    let mirrored = MeshData::sphere(1.0, 8, 16).transformed(Mat4::scale(-1.0, 1.0, 1.0));
    assert_closed_outward("mirrored sphere", &mirrored, None);
    let mut merged = MeshData::cube();
    merged.append(&MeshData::cube().transformed(Mat4::translation(5.0, 0.0, 0.0)));
    assert_eq!(merged.vertices.len(), 48);
    assert!(merged.indices[36..].iter().all(|i| *i >= 24));
    let (vertices, indices): (Vec<_>, Vec<_>) = merged.into();
    assert_eq!((vertices.len(), indices.len()), (48, 72));
}
