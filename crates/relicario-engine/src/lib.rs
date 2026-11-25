//! Motor de decisão do Relicário. Núcleo puro, sem I/O.
//!
//! - [`graph`]: a árvore de snapshots e a relação entre dois heads.
//! - [`plan`]: o que fazer com um jogo dado o disco local e os heads das outras máquinas.
//! - [`retention`]: quais snapshots podem ter os dados liberados.
//! - [`guard`]: proteção contra save corrompido.

pub mod graph;
pub mod guard;
pub mod plan;
pub mod retention;

pub use graph::{Graph, Node};
pub use plan::{Action, Plan, PlanInput, plan};
