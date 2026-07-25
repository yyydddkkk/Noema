use std::{error::Error, fmt};

use noema_ir::{DesiredWorkloadState, GenerationId, ObservedWorkloadState, WorkloadId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    StaleBase {
        current: GenerationId,
        requested: GenerationId,
    },
    StaleCandidate {
        current: GenerationId,
        based_on: GenerationId,
    },
    GenerationExhausted,
    EmptyCandidate,
    WorkloadAlreadyExists(WorkloadId),
    WorkloadNotFound(WorkloadId),
    InvalidCreateState(DesiredWorkloadState),
    WorkloadNotAbsent {
        workload: WorkloadId,
        desired: DesiredWorkloadState,
        observed: ObservedWorkloadState,
    },
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleBase { current, requested } => write!(
                formatter,
                "proposal is based on generation {requested}, but current is {current}"
            ),
            Self::StaleCandidate { current, based_on } => write!(
                formatter,
                "candidate is based on generation {based_on}, but current is {current}"
            ),
            Self::GenerationExhausted => formatter.write_str("generation identifier exhausted"),
            Self::EmptyCandidate => formatter.write_str("candidate contains no state changes"),
            Self::WorkloadAlreadyExists(workload) => {
                write!(formatter, "workload '{workload}' already exists")
            }
            Self::WorkloadNotFound(workload) => {
                write!(formatter, "workload '{workload}' does not exist")
            }
            Self::InvalidCreateState(state) => {
                write!(
                    formatter,
                    "cannot create a workload with desired state {state:?}"
                )
            }
            Self::WorkloadNotAbsent {
                workload,
                desired,
                observed,
            } => write!(
                formatter,
                "cannot purge workload '{workload}' while desired={desired:?} and observed={observed:?}"
            ),
        }
    }
}

impl Error for StateError {}
