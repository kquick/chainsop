use std::ffi::{OsString};
use std::path::{PathBuf};
use anyhow;
use thiserror;

use crate::filehandling::defs::FileArg;


#[derive(thiserror::Error,Debug)]
pub enum SubProcError {

    #[error("Missing file for operation")]
    ErrorMissingFile,

    #[error("Target directory {1:?} error running command {0:?}: {2:?}")]
    ErrorBadDirectory(String, PathBuf, std::io::Error),

    #[error("Error {2:?} running command {0:?} {1:?} in dir {3:?}\n{4:}")]
    ErrorRunningCmd(String, Vec<OsString>, Option<i32>, Option<PathBuf>, String),

    #[error("Error {2:?} setting up running command {0:?} {1:?} in dir {3:?}")]
    ErrorCmdSetup(String, Vec<OsString>, std::io::Error, Option<PathBuf>),

    #[error("Error {2:?} executing command {0:?} {1:?} in dir {3:?}")]
    ErrorExecuting(String, Vec<OsString>, anyhow::Error, Option<PathBuf>),

    #[error("Unsupported file for command {0:?}: {1:?}")]
    ErrorUnsupportedFile(String, FileArg),

    #[error("Invalid operation actual file specification: {0:?}")]
    ErrorUnsupportedActualFile(String),

    #[error("No valid operation specified")]
    ErrorInvalidOperation,
}
