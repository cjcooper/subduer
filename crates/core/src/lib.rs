//! Cluster access for k8s-subduer, independent of any UI.

pub mod catalog;
pub mod cluster;
pub mod contexts;
pub mod safety;
pub mod table;
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
    #[error("context {0:?} not found in kubeconfig")]
    UnknownContext(String),
}

pub type Result<T> = std::result::Result<T, Error>;
