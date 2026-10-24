//! Player for the game's `rage::swf` movies: display list and timeline
//! ([`player`]), conversion of the display list into textured triangle batches
//! the way the original renderer emits them ([`draw`]), and a small software
//! rasteriser for headless checks ([`raster`]).

pub mod draw;
pub mod player;
pub mod raster;
pub mod textures;
pub mod value;
/// The rasteriser as it was before the 2026-10-23 speed-ups, kept as the reference the tests hold it to.
#[cfg(test)]
mod raster_ref;
#[cfg(test)]
mod raster_ref_test;

pub use mcla_formats::rage::swf;
