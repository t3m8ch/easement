//! easement through its library: every command answers here with data, on
//! scratch projects indexed by the real indexers. `tests/cli.rs` pins only what
//! the binary adds: the arguments, the exit codes and what it prints.

#[path = "../common/mod.rs"]
mod common;
mod run;

mod aliases;
mod answers;
mod baseline;
mod cache;
mod commands;
mod components;
mod diff;
mod evidence;
mod examples;
mod exclude;
mod formats;
mod indexers;
mod languages;
mod packages;
mod policy;
mod propose;
mod reach;
mod reexports;
mod review;
mod rules;
mod suggest;
mod surfaces;
mod symbols;
mod unused;
mod warn;

mod augmentations;
