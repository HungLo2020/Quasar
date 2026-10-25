//! Optional navigation-mesh support backed by the `navmesh` crate.
//!
//! Quasar does not generate navigation meshes itself. Authoring/generation can
//! use an external pipeline or `navmesh`'s types; these helpers only adapt
//! array coordinates and expose path results with deterministic point order.
pub use navmesh;
use navmesh::{NavMesh, NavPathMode, NavQuery, NavVec3};

/// Finds a navmesh path, returning `[x, y, z]` points.
pub fn find_path(
    mesh: &NavMesh,
    from: [f32; 3],
    to: [f32; 3],
    query: NavQuery,
    mode: NavPathMode,
) -> Option<Vec<[f32; 3]>> {
    if !from.iter().chain(&to).all(|value| value.is_finite()) {
        return None;
    }
    mesh.find_path(
        NavVec3::new(from[0], from[1], from[2]),
        NavVec3::new(to[0], to[1], to[2]),
        query,
        mode,
    )
    .map(|path| {
        path.into_iter()
            .map(|point| [point.x, point.y, point.z])
            .collect()
    })
}

/// Finds a navmesh path with a caller-provided connection filter.
pub fn find_path_custom(
    mesh: &NavMesh,
    from: [f32; 3],
    to: [f32; 3],
    query: NavQuery,
    mode: NavPathMode,
    filter: impl FnMut(f32, usize, usize) -> bool,
) -> Option<Vec<[f32; 3]>> {
    if !from.iter().chain(&to).all(|value| value.is_finite()) {
        return None;
    }
    mesh.find_path_custom(
        NavVec3::new(from[0], from[1], from[2]),
        NavVec3::new(to[0], to[1], to[2]),
        query,
        mode,
        filter,
    )
    .map(|path| {
        path.into_iter()
            .map(|point| [point.x, point.y, point.z])
            .collect()
    })
}

/// Converts a triangle-mesh vertex list and triangle indices into a `NavMesh`.
pub fn mesh_from_triangles(
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
) -> Result<NavMesh, navmesh::Error> {
    let vertices = vertices
        .into_iter()
        .map(|point| NavVec3::new(point[0], point[1], point[2]))
        .collect();
    let triangles = triangles
        .into_iter()
        .map(|indices| navmesh::NavTriangle::from((indices[0], indices[1], indices[2])))
        .collect();
    NavMesh::new(vertices, triangles)
}
