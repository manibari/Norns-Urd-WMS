//! Urd-WMS rules that do not depend on storage or screens.
//!
//! Everything here is pure or talks only to a camera / recognition service, so
//! it is tested on its own and the desktop app is a thin layer over it.

pub mod camera;
pub mod matching;
pub mod normalize;
pub mod recognition;
pub mod units;
