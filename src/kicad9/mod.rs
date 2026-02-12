//! KiCad 9 footprint library reader.
//!
//! Parses `.kicad_mod` files in the KiCad 9 format. Supports:
//! - (footprint "NAME" ...) root
//! - fp_line, fp_rect, fp_circle, fp_arc (start/mid/end), fp_poly, fp_curve
//! - pad with circle, rect, oval, trapezoid, roundrect
//! - stroke (width)(type) for graphic elements

mod data;
mod error;
mod parse;
mod sexp;

pub use data::*;
pub use error::{Error, Result};
pub use parse::{load_footprint_dir, parse_footprint, read_footprint};
