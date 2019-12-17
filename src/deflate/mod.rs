//! The deflate module provides tools for applying the permessage-deflate extension.

use libc;
use libz_sys as ffi;

mod context;
mod extension;

pub use self::extension::{DeflateBuilder, DeflateHandler, DeflateSettings};
