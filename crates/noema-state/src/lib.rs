//! Versioned, transactional world state for Noema.
//!
//! A [`CandidateGeneration`] owns a cloned state snapshot. Mutating it cannot
//! affect the current generation. Only [`GenerationStore::commit`] moves the
//! candidate snapshot into the current-state slot.

mod error;
mod event;
mod model;
mod store;

pub use error::{PersistenceError, StateError};
pub use event::{StateEvent, StateEventKind};
pub use model::{Workload, WorldState};
pub use store::{CandidateGeneration, GenerationStore, MemoryGenerationStore};
