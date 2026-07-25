use noema_ir::{
    DesiredWorkloadState, GenerationId, Mutation, ObservedWorkloadState, TransactionId, WorkloadId,
};

use crate::{StateError, StateEvent, StateEventKind, Workload, WorldState};

pub trait GenerationStore {
    fn current(&self) -> &WorldState;

    fn events(&self) -> &[StateEvent];

    /// Starts an isolated candidate based on the exact current generation.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::StaleBase`] when `base_generation` is not current,
    /// or [`StateError::GenerationExhausted`] when no identifier remains.
    fn begin(
        &mut self,
        transaction_id: TransactionId,
        base_generation: GenerationId,
    ) -> Result<CandidateGeneration, StateError>;

    /// Atomically promotes a non-empty candidate to current state.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::StaleCandidate`] if another candidate committed
    /// first, or [`StateError::EmptyCandidate`] when no state changed.
    fn commit(&mut self, candidate: CandidateGeneration) -> Result<GenerationId, StateError>;

    fn abort(&mut self, candidate: CandidateGeneration);
}

#[derive(Debug)]
pub struct CandidateGeneration {
    id: GenerationId,
    based_on: GenerationId,
    transaction_id: TransactionId,
    state: WorldState,
    pending_events: Vec<StateEventKind>,
}

impl CandidateGeneration {
    #[must_use]
    pub const fn id(&self) -> GenerationId {
        self.id
    }

    #[must_use]
    pub const fn based_on(&self) -> GenerationId {
        self.based_on
    }

    #[must_use]
    pub const fn state(&self) -> &WorldState {
        &self.state
    }

    /// Applies a model-authored desired-state mutation to this isolated copy.
    ///
    /// # Errors
    ///
    /// Returns an error when the mutation conflicts with the candidate's
    /// current object state. No event is appended for a rejected mutation.
    pub fn apply_mutation(&mut self, mutation: &Mutation) -> Result<(), StateError> {
        match mutation {
            Mutation::CreateWorkload {
                id,
                artifact,
                desired,
                health,
                restart_policy,
            } => {
                if *desired == DesiredWorkloadState::Absent {
                    return Err(StateError::InvalidCreateState(*desired));
                }
                if let Some(existing) = self.state.workload(id) {
                    if existing.matches_declaration(artifact, *desired, health, *restart_policy) {
                        return Ok(());
                    }
                    return Err(StateError::WorkloadAlreadyExists(id.clone()));
                }

                let workload = Workload::new(
                    artifact.clone(),
                    *desired,
                    health.clone(),
                    *restart_policy,
                    self.id,
                );
                self.state.insert_workload(id.clone(), workload);
                self.pending_events.push(StateEventKind::WorkloadCreated {
                    workload: id.clone(),
                });
            }
            Mutation::SetDesiredState { workload, state } => {
                self.set_desired(workload, *state)?;
            }
            Mutation::RemoveWorkload { workload } => {
                if self.state.workload(workload).is_some() {
                    self.set_desired(workload, DesiredWorkloadState::Absent)?;
                }
            }
        }
        Ok(())
    }

    /// Records a trusted local observation without changing desired state.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::WorkloadNotFound`] for an unknown workload.
    pub fn observe_workload(
        &mut self,
        workload: &WorkloadId,
        observed: ObservedWorkloadState,
    ) -> Result<(), StateError> {
        let entry = self
            .state
            .workload_mut(workload)
            .ok_or_else(|| StateError::WorkloadNotFound(workload.clone()))?;
        let previous = entry.observed;
        if previous == observed {
            return Ok(());
        }
        entry.observed = observed;
        entry.last_changed_in = self.id;
        self.pending_events
            .push(StateEventKind::WorkloadObservedChanged {
                workload: workload.clone(),
                from: previous,
                to: observed,
            });
        Ok(())
    }

    /// Removes a fully absent workload from the object graph.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::WorkloadNotFound`] for an unknown workload, or
    /// [`StateError::WorkloadNotAbsent`] until both desired and observed state
    /// are absent.
    pub fn purge_workload(&mut self, workload: &WorkloadId) -> Result<(), StateError> {
        let entry = self
            .state
            .workload(workload)
            .ok_or_else(|| StateError::WorkloadNotFound(workload.clone()))?;
        if entry.desired() != DesiredWorkloadState::Absent
            || entry.observed() != ObservedWorkloadState::Absent
        {
            return Err(StateError::WorkloadNotAbsent {
                workload: workload.clone(),
                desired: entry.desired(),
                observed: entry.observed(),
            });
        }
        self.state.remove_workload(workload);
        self.pending_events.push(StateEventKind::WorkloadPurged {
            workload: workload.clone(),
        });
        Ok(())
    }

    fn set_desired(
        &mut self,
        workload: &WorkloadId,
        desired: DesiredWorkloadState,
    ) -> Result<(), StateError> {
        let entry = self
            .state
            .workload_mut(workload)
            .ok_or_else(|| StateError::WorkloadNotFound(workload.clone()))?;
        let previous = entry.desired;
        if previous == desired {
            return Ok(());
        }
        entry.desired = desired;
        entry.last_changed_in = self.id;
        self.pending_events
            .push(StateEventKind::WorkloadDesiredChanged {
                workload: workload.clone(),
                from: previous,
                to: desired,
            });
        Ok(())
    }
}

pub struct MemoryGenerationStore {
    current: WorldState,
    next_generation: u64,
    next_event_sequence: u64,
    events: Vec<StateEvent>,
}

impl MemoryGenerationStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            current: WorldState::initial(),
            next_generation: 1,
            next_event_sequence: 1,
            events: Vec::new(),
        }
    }

    fn append_event(
        &mut self,
        transaction_id: &TransactionId,
        generation: GenerationId,
        kind: StateEventKind,
    ) {
        let event = StateEvent::new(
            self.next_event_sequence,
            transaction_id.clone(),
            generation,
            kind,
        );
        self.next_event_sequence += 1;
        self.events.push(event);
    }
}

impl Default for MemoryGenerationStore {
    fn default() -> Self {
        Self::new()
    }
}

impl GenerationStore for MemoryGenerationStore {
    fn current(&self) -> &WorldState {
        &self.current
    }

    fn events(&self) -> &[StateEvent] {
        &self.events
    }

    fn begin(
        &mut self,
        transaction_id: TransactionId,
        base_generation: GenerationId,
    ) -> Result<CandidateGeneration, StateError> {
        if base_generation != self.current.generation() {
            return Err(StateError::StaleBase {
                current: self.current.generation(),
                requested: base_generation,
            });
        }

        let candidate_id = GenerationId(self.next_generation);
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(StateError::GenerationExhausted)?;
        let state = self.current.for_candidate(candidate_id);
        self.append_event(
            &transaction_id,
            candidate_id,
            StateEventKind::CandidateStarted {
                based_on: base_generation,
            },
        );

        Ok(CandidateGeneration {
            id: candidate_id,
            based_on: base_generation,
            transaction_id,
            state,
            pending_events: Vec::new(),
        })
    }

    fn commit(&mut self, candidate: CandidateGeneration) -> Result<GenerationId, StateError> {
        if candidate.based_on != self.current.generation() {
            self.append_event(
                &candidate.transaction_id,
                candidate.id,
                StateEventKind::CandidateCommitRejected {
                    based_on: candidate.based_on,
                    current: self.current.generation(),
                },
            );
            return Err(StateError::StaleCandidate {
                current: self.current.generation(),
                based_on: candidate.based_on,
            });
        }
        if candidate.pending_events.is_empty() {
            self.append_event(
                &candidate.transaction_id,
                candidate.id,
                StateEventKind::CandidateAborted {
                    based_on: candidate.based_on,
                },
            );
            return Err(StateError::EmptyCandidate);
        }

        for kind in candidate.pending_events {
            self.append_event(&candidate.transaction_id, candidate.id, kind);
        }
        let previous = self.current.generation();
        self.current = candidate.state;
        self.append_event(
            &candidate.transaction_id,
            candidate.id,
            StateEventKind::CandidateCommitted { previous },
        );
        Ok(candidate.id)
    }

    fn abort(&mut self, candidate: CandidateGeneration) {
        for kind in candidate.pending_events {
            self.append_event(&candidate.transaction_id, candidate.id, kind);
        }
        self.append_event(
            &candidate.transaction_id,
            candidate.id,
            StateEventKind::CandidateAborted {
                based_on: candidate.based_on,
            },
        );
    }
}
