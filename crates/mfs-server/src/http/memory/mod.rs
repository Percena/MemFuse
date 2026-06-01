//! Memory handlers — split into sub-modules by domain.
use super::*;

mod admin;
mod consolidate;
mod context;
mod search;

pub(in crate::http) use admin::*;
pub(in crate::http) use consolidate::*;
pub(in crate::http) use context::*;
pub(in crate::http) use search::*;
