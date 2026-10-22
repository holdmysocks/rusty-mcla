//! Physics: static world collision + arcade car model.

pub mod car;
pub mod collision;
pub mod drivetrain;
pub mod exact;
pub mod gjk;
pub mod hull;
pub mod loose;
pub mod pad;
pub mod replay;
pub mod solver;
pub mod sst;
mod engine;
mod engine_curve;
mod transmission;
mod tyre;

pub use car::{CarInput, CarParams, CarState};
pub use collision::{SectorMesh, WorldCollider};
pub mod vehsim;
