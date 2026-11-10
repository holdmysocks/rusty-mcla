//! Physics: static world collision + arcade car model.

pub mod agro;
pub mod artic;
pub mod artic_build;
pub mod artic_collide;
#[cfg(test)]
pub mod artic_offline;
pub mod artic_pose;
pub mod artic_run;
pub mod camprobe;
pub mod car;
pub mod collision;
pub mod drivetrain;
pub mod exact;
pub mod exact_inv4;
pub mod exact_mat;
pub mod fluid;
pub mod gjk;
pub mod headon;
pub mod wipeout;
pub mod hull;
pub mod loose;
pub mod loose_words;
pub mod vehsim_words;
#[cfg(test)]
mod vehsim_small_tests;
#[cfg(test)]
mod hyd_refresh_tests;
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
#[cfg(test)] mod hyd_ref_tests;

pub use car::{CarInput, CarParams, CarState};
pub use collision::{SectorMesh, WorldCollider};
pub mod vehsim;
