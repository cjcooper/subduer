//! Cluster access for k8s-subduer, independent of any UI.

pub mod actions;
pub mod capture;
pub mod catalog;
pub mod cluster;
pub mod config;
pub mod contexts;
pub mod cron;
pub mod download;
pub mod edit;
pub mod exec;
pub mod external_secrets;
pub mod forward;
pub mod helm;
pub mod history;
pub mod logs;
pub mod namespace;
pub mod network;
pub mod node;
pub mod overview;
pub mod pod_details;
pub mod quantity;
pub mod rbac;
pub mod rightsize;
pub mod safety;
pub mod storage;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod table;
pub mod tls_scan;
pub mod trace;
pub mod trace_file;
pub mod trace_report;
pub mod trace_review;
pub mod upload;
pub mod usage;
pub mod watch;
pub mod workload;

pub use kube;
pub use subduer_types as types;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kubeconfig: {0}")]
    Kubeconfig(#[from] kube::config::KubeconfigError),
    #[error(transparent)]
    Kube(#[from] kube::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_norway::Error),
    #[error("context {0:?} is read-only; unlock it to make changes")]
    Locked(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("context {0:?} not found in kubeconfig")]
    UnknownContext(String),
    /// A complete message for people, shown as is.
    #[error("{0}")]
    Message(String),
}

impl Error {
    /// A short message for people: the API server's own message for API
    /// errors (e.g. `pods "x" is forbidden: …`) instead of the full status dump.
    pub fn user_message(&self) -> String {
        match self {
            Self::Kube(kube::Error::Api(status)) if !status.message.is_empty() => status.message.clone(),
            other => other.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod coverage_tests;
