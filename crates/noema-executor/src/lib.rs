//! Execution backends for trusted, locally generated Execution IR.
//!
//! M2 provides only an in-memory backend. It models runtime behavior without
//! spawning processes or touching the host operating system.

mod backend;
mod process;
mod simulation;

pub use backend::ExecutionBackend;
pub use process::ProcessBackend;
pub use simulation::{ExecutionFailure, SimulationBackend, SimulationFault};
