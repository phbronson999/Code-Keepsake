//! keepsake-core: everything Code Keepsake knows how to do, with no UI.
//!
//! The binary (`code-keepsake`) only renders what this crate returns, so a GUI
//! can be added later without touching any of this.

pub mod analysis;
pub mod config;
pub mod error;
pub mod fmt;
pub mod fsmeasure;
pub mod git;
pub mod jsonc;
pub mod parse;
pub mod plan;
pub mod safety;
pub mod sandbox;
pub mod scan;
pub mod size;
pub mod stub;
pub mod workspace;

pub use error::{Error, Result};
