use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Something the user can fix: bad input, invalid config, wrong state.
    #[error("{0}")]
    Usage(String),
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("git {command} failed: {message}")]
    Git { command: String, message: String },
    /// Applying stopped and nothing switched; the message says why.
    #[error("{0}")]
    Apply(String),
    /// Publish or Pull stopped; `sync.json` records why.
    #[error("{0}")]
    Sync(String),
    /// Signing in to GitHub or calling its API failed.
    #[error("{0}")]
    GitHub(String),
    /// The background service could not start or keep running.
    #[error("{0}")]
    Daemon(String),
    /// Downloading or extracting an external failed.
    #[error("{0}")]
    External(String),
    /// The review item is no longer listed.
    #[error("no review item {0}; run `rsmt review`")]
    NoItem(u32),
    /// The review item changed since it was listed.
    #[error("review item {0} changed since it was listed; run `rsmt review`")]
    Stale(u32),
    /// A template cannot render outside a terminal yet.
    #[error("{}", .0.describe())]
    Waiting(crate::dotfiles::review::WaitingFor),
}

impl Error {
    /// Usage errors exit with 2, everything else with 1.
    pub fn is_usage(&self) -> bool {
        matches!(
            self,
            Error::Usage(_) | Error::NoItem(_) | Error::Stale(_) | Error::Waiting(_)
        )
    }

    pub(crate) fn read(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
        let path = path.into();
        move |source| Error::Read { path, source }
    }

    pub(crate) fn write(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
        let path = path.into();
        move |source| Error::Write { path, source }
    }
}
