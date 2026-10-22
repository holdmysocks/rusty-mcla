//! The stretches of a race route that leave the lane network (alleys, car parks, plazas, the
//! shortcuts the race scripts list as a few widely spaced nodes), and the navigation mesh.
//!
//! The computer racers find their way over the mesh with the original's quickest-path query
//! (brain.rs, `NavMesh::quickest_path`). The route line itself is only read by the port's own
//! helpers (racers slid along it outside the streamed city, the test autopilot, the old driver
//! behind `MCLA_AI_OLD`, the plain fallback map): an off-road stretch is a straight hop between
//! its two nodes. (The port's own mesh and grid planners - invented costs, snap and margin -
//! were retired on 2026-10-22, MAP.md "Navigation mesh queries".)

use crate::physics::collision::WorldCollider;
use glam::Vec3;
use mcla_formats::rage::navmesh::NavMesh;

/// The city's navigation mesh (`city.xsm`), loaded once.
pub fn mesh() -> Option<&'static NavMesh> {
    static MESH: std::sync::OnceLock<Option<NavMesh>> = std::sync::OnceLock::new();
    MESH.get_or_init(|| {
        // (the data directory as main.rs takes it: --data, else the default)
        let args: Vec<String> = std::env::args().collect();
        let dir = args.iter().position(|a| a == "--data").and_then(|i| args.get(i + 1).cloned()).or_else(|| std::env::var("MCLA_DATA_DIR").ok()).unwrap_or_else(|| "D:/MCLA/extracted/cache".to_string());
        let m = crate::assets::load_resource_file(&std::path::Path::new(&dir).join("resources/city/sc/0x69d17d57.r26")).ok().and_then(|r| NavMesh::parse(&r));
        match m.as_ref() {
            Some(m) => log::info!("off-road: navigation mesh, {} polygons ({} for cars)", m.polys.len(), m.polys.iter().filter(|p| p.drivable()).count()),
            None => log::warn!("off-road: no navigation mesh"),
        }
        m
    })
    .as_ref()
}

/// The line through the given points of an off-road stretch: the points themselves, straight
/// from one to the next (always "through").
pub fn plan(_col: &WorldCollider, _solid: &[Vec3], pts: &[Vec3]) -> Option<(Vec<Vec3>, bool)> {
    (pts.len() >= 2).then(|| (pts.to_vec(), true))
}
