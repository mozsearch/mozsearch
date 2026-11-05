//! JS environments: which globals a tree's scripts run in, with which other
//! scripts (see `discover`), for scip-typescript's analysis of the JS outside
//! TypeScript projects, and tests' cliques.

pub mod discover;
pub mod rules;
pub mod scan;
