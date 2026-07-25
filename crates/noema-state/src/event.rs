use noema_ir::{
    DesiredWorkloadState, GenerationId, ObservedWorkloadState, TransactionId, WorkloadId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StateEvent {
    sequence: u64,
    transaction_id: TransactionId,
    generation: GenerationId,
    kind: StateEventKind,
}

impl StateEvent {
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn generation(&self) -> GenerationId {
        self.generation
    }

    #[must_use]
    pub const fn kind(&self) -> &StateEventKind {
        &self.kind
    }

    #[must_use]
    pub const fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    pub(crate) fn new(
        sequence: u64,
        transaction_id: TransactionId,
        generation: GenerationId,
        kind: StateEventKind,
    ) -> Self {
        Self {
            sequence,
            transaction_id,
            generation,
            kind,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateEventKind {
    CandidateStarted {
        based_on: GenerationId,
    },
    WorkloadCreated {
        workload: WorkloadId,
    },
    WorkloadDesiredChanged {
        workload: WorkloadId,
        from: DesiredWorkloadState,
        to: DesiredWorkloadState,
    },
    WorkloadObservedChanged {
        workload: WorkloadId,
        from: ObservedWorkloadState,
        to: ObservedWorkloadState,
    },
    WorkloadPurged {
        workload: WorkloadId,
    },
    CandidateCommitted {
        previous: GenerationId,
    },
    CandidateAborted {
        based_on: GenerationId,
    },
    CandidateCommitRejected {
        based_on: GenerationId,
        current: GenerationId,
    },
}
