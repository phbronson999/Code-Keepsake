use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse workspace file {path}: {message}")]
    Workspace { path: PathBuf, message: String },

    #[error("`git {args}` failed in {repo}: {stderr}")]
    Git {
        repo: PathBuf,
        args: String,
        stderr: String,
    },

    #[error("left {path} alone: {message}")]
    Refused { path: PathBuf, message: String },

    #[error("git was not found on PATH")]
    GitMissing,
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
