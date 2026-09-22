//! Crate root: declares the five scan stages plus the spine every stage
//! shares (CLI surface, shared types, target resolution, file discovery,
//! and the pipeline that sequences the stages).

pub mod cli;
pub mod discover;
pub(crate) mod exec_lines;
#[cfg(test)]
pub(crate) mod golden;
pub(crate) mod hashing;
pub mod model;
pub mod pipeline;
pub mod target;

pub mod clones;
pub mod metrics;
pub mod parse;
pub mod report;
pub mod rules;
