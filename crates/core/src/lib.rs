//! Cluster access for k8s-subduer, independent of any UI.

pub mod actions;
pub mod catalog;
pub mod cluster;
pub mod contexts;
pub mod helm;
pub mod history;
pub mod overview;
pub mod pod_details;
pub mod quantity;
pub mod safety;
pub mod table;
pub mod usage;
pub mod watch;

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
