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

#[derive(Debug)]
pub enum PersistenceError {
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidSnapshot(String),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "state snapshot I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "state snapshot JSON is invalid: {error}"),
            Self::InvalidSnapshot(message) => {
                write!(formatter, "state snapshot invariants failed: {message}")
            }
        }
    }
}

impl Error for PersistenceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::InvalidSnapshot(_) => None,
        }
    }
}

impl From<std::io::Error> for PersistenceError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
