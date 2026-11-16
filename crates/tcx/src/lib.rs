//! `tcx` - TREX ConteXt.
//!
//! Out-of-repo context store for AI-assisted development.
//!
//! This library is deliberately independent of the `tg` guard: the guard stays
//! frozen C++, and nothing here is imported by it. Dependencies point one way
//! only.

pub mod briefing;
pub mod find;
pub mod help;
pub mod id;
pub mod map;
pub mod import;
pub mod notes;
pub mod prime;
pub mod project;
pub mod store;
