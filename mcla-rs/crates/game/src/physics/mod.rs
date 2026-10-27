//! Physics: static world collision + arcade car model.

pub mod agro;
pub mod camprobe;
pub mod car;
pub mod collision;
pub mod drivetrain;
pub mod exact;
pub mod gjk;
pub mod headon;
pub mod hull;
pub mod loose;
pub mod pad;
pub mod replay;
pub mod softpush;
pub mod solver;
pub mod sst;
mod engine;
mod engine_curve;
mod transmission;
mod tyre;
#[cfg(test)]
mod original_math_tests;

pub use car::{CarInput, CarParams, CarState};
pub use collision::{SectorMesh, WorldCollider};
pub mod vehsim;
