//! This is the lowest level module that supports the chainsop library.  This
//! module is responsible for actually performing the subprocess or function
//! execution that has been configured and determined by the rest of the chainsop
//! library.
//!
//! This module helps with separation of concerns between the management and
//! determination of what should be done and in what sequence versus the actual
//! performance of those things.  This helps keep the rest of chainsop "pure" and
//! easy to test, isolating and minimizing the interactions with the OS.  This
//! also helps with potential efforts such as tracing/logging or test harnesses.
//!
//! Please note that the interaction between the other portions of chainsop and
//! this module is a sequence of back-and-forth interactions between those
//! portions and this module: the output or results of performing the OS
//! interactions handled by this module will potentially inform subsequent
//! activities determined by that core of chainsop.

use anyhow;
use glob;
use std::env::current_dir;
use std::ffi::{OsString};
use std::path::{Path, PathBuf};
use std::process;
use std::rc::Rc;

use crate::filehandling::defs::*;


/// The OsRun trait is used to define the interface to implementation that will
/// perform operations that should be executed.  The default implementation of
/// this trait is the [Executor] which will perform the specified actions on the
/// current system.
///
/// The use of this abstraction however allows the actual IO operations to be
/// controlled and possibly even simulated or controlled for various purposes
/// (e.g. testing, OS abstraction, etc.) instead of using the default [Executor].
///
/// When used, an implementation of the OsRun object is expected to be immutable;
/// if it needs to maintain internal state or make updates based on the
/// operations performed, it should use an internal RefCell for those mutable
/// portions.

pub trait OsRun {

    /// Run the specified executable with the specified arguments.  The default
    /// (NormalRun) behaviour is to use Command to perform this execution.
    fn run_executable(&self,
                      label: &str,
                      exe_file: &Path,
                      args: &Vec<OsString>,
                      fromdir: &Option<PathBuf>) -> OsRunResult;

    /// Call the specified function with the specified file arguments.  The
    /// default (NormalRun) behaviour is to actually perform the call.
    fn run_function(&self,
                    name : &str,
                    call : &Rc<dyn Fn(&Path, &ActualFile, &ActualFile) -> anyhow::Result<()>>,
                    inpfiles: &ActualFile,
                    outfile: &ActualFile,
                    fromdir: &Option<PathBuf>) -> OsRunResult;

    /// This function is called to perform a glob-style pattern match against a
    /// set of files.  The return is a vector of files that are found (when using
    /// the default NormalRun behavior).
    fn glob_search(&self, globpat: &String) -> anyhow::Result<Vec<PathBuf>>;

    /// This function is called to create a temporary file (when performed using
    /// the default NormalRun executor).  Note that the return value is provided
    /// by the tempfile crate and is actually a resource managing object: it
    /// cannot be obtained without actually creating a temporary file. This
    /// significantly restricts the capability of nullifying or replacing this
    /// operation with an alternative, and while there are various techniques
    /// that could be used to resolve this constraint, it is observed that since
    /// a tempfile is not intended to be a generally available resource and its
    /// existence is generally non-impactful to the system, it is relatively safe
    /// to allow the normal behavior even in simulation or testing scenarios.
    fn mk_tempfile(&self, suffix: &String) -> anyhow::Result<tempfile::NamedTempFile>;
}

/// The OsRunResult is the return value from the `run_executable` and
/// `run_function` methods.
pub enum OsRunResult {
    Good,
    ExecFailed(std::io::Error),
    ExecError(Option<i32>, String),
    RunError(anyhow::Error),
    BadDirectory(PathBuf, std::io::Error),
}


/// This is the default Executor defined by the chainsop create.  This Executor
/// provides three modes of operation, controlling echoing operations to stderr
/// and actually performing those operations.
///
/// It is also possible to use user-defined executors that implement the OsRun
/// trait.
pub enum Executor { NormalRun, NormalWithEcho, NormalWithLabel, DryRun }

impl Executor {
    fn get_dir<T: Into<PathBuf> + Clone>(fromdir: &Option<T>) -> Result<PathBuf, std::io::Error>
    {
        fromdir.as_ref().map(|p| Ok(p.clone().into())).unwrap_or_else(current_dir)
    }
}


impl OsRun for Executor {

    fn run_executable(&self,
                      label: &str,
                      exe_file: &Path,
                      args: &Vec<OsString>,
                      fromdir: &Option<PathBuf>) -> OsRunResult
    {
        match Executor::get_dir(fromdir) {
            Ok(tgtdir) => {
                match &self {
                    Executor::NormalRun => {}
                    Executor::NormalWithLabel => eprintln!("#=> {}", label),
                    Executor::NormalWithEcho |
                    Executor::DryRun =>
                        eprintln!("#: {} {} [in {}]",
                                  exe_file.display(),
                                  args.iter().map(|x| x.to_str().unwrap())
                                  .collect::<Vec<_>>().join(" "),
                                  tgtdir.display())
                }
                match &self {
                    Executor::NormalRun |
                    Executor::NormalWithLabel |
                    Executor::NormalWithEcho => {
                        match process::Command::new(&exe_file)
                            .args(args)
                            .current_dir(&tgtdir)
                            .stdout(process::Stdio::piped())
                            .stderr(process::Stdio::piped())
                            .spawn()
                        {
                            Ok(child) => {
                                match child.wait_with_output() {
                                    Ok(out) => {
                                        if !out.status.success() {
                                            OsRunResult::ExecError(
                                                out.status.code(),
                                                String::from_utf8_lossy(&out.stderr).into_owned())
                                        } else {
                                            OsRunResult::Good
                                        }
                                    }
                                    Err(e) => OsRunResult::ExecFailed(e)
                                }
                            }
                            Err(e) => OsRunResult::ExecFailed(e)
                        }
                    }
                    Executor::DryRun => OsRunResult::Good
                }
            }
            Err(e) => OsRunResult::BadDirectory(".".into(), e)
        }
    }

    fn run_function(&self,
                    name : &str,
                    call : &Rc<dyn Fn(&Path, &ActualFile, &ActualFile) -> anyhow::Result<()>>,
                    inpfiles: &ActualFile,
                    outfile: &ActualFile,
                    fromdir: &Option<PathBuf>) -> OsRunResult
    {
        match Executor::get_dir(fromdir) {
            Ok(tgtdir) => {
                match &self {
                    Executor::NormalRun => {}
                    Executor::NormalWithLabel => eprintln!("=> {}", name),
                    Executor::NormalWithEcho |
                    Executor::DryRun => {
                        eprintln!("Call {:?}, input={:?}, output={:?} [in {:?}]",
                                  name, inpfiles, outfile, tgtdir);
                    }
                }
                match &self {
                    Executor::NormalRun |
                    Executor::NormalWithLabel |
                    Executor::NormalWithEcho => {
                        match (call)(&tgtdir, &inpfiles, &outfile) {
                            Ok(()) => OsRunResult::Good,
                            Err(e) => OsRunResult::RunError(e)
                        }
                    }
                    Executor::DryRun => OsRunResult::Good
                }
            }
            Err(e) => OsRunResult::BadDirectory(".".into(), e)
        }
    }

    fn glob_search(&self, globpat: &String) -> anyhow::Result<Vec<PathBuf>>
    {
        match &self {
            Executor::NormalRun |
            Executor::NormalWithLabel |
            Executor::NormalWithEcho =>
                Ok(glob::glob(&globpat)?.filter_map(Result::ok).collect()),
            Executor::DryRun => Ok(vec![])
        }
    }

    fn mk_tempfile(&self, suffix: &String)
                   -> anyhow::Result<tempfile::NamedTempFile>
    {
        match &self {
            Executor::NormalWithEcho |
            Executor::NormalRun =>
                Ok(tempfile::Builder::new().suffix(suffix).tempfile()?),
            Executor::NormalWithLabel => {
                let tf = tempfile::Builder::new().suffix(suffix).tempfile()?;
                eprintln!("Created temp file {:?}", tf);
                Ok(tf)
            }
            Executor::DryRun =>
                // Go ahead and create a tempfile even during a DryRun.
                Ok(tempfile::Builder::new().suffix(suffix).tempfile()?),
        }
    }
}
