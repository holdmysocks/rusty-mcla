//! Physics: static world collision + arcade car model.

pub mod agro;
pub mod artic;
pub mod artic_build;
pub mod artic_collide;
pub mod artic_pose;
pub mod artic_run;
pub mod camprobe;
pub mod car;
pub mod collision;
pub mod drivetrain;
pub mod exact;
pub mod fluid;
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
#[cfg(test)] mod multicar_island;

pub use car::{CarInput, CarParams, CarState};
pub use collision::{SectorMesh, WorldCollider};
pub mod vehsim;
