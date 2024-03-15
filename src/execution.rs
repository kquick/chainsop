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
use std::env::{current_dir, vars};
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
                      exe_env: &EnvSpec,
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


/// Specifies environment variables settings that should be available in the
/// environment for any [OsRun::run_executable] subprocess execution.  By
/// default, the environment is inherited from the parent process, but this
/// allows adding, removing, and adjusting environment variable values as well as
/// suppression of inheritance.
// All constructors of a Rust enum inherit the visibility of the enum itself.
// Because there is a normalization invariant that is expected to be maintained
// for this enum, the recursion is hidden by using the SubEnvSpec to wrap the
// recursion in a field only visible within this crate, which therefore requires
// the use of the EnvSpec methods to construct any recursive EnvSpec
// specification; those methods enforce the required invariants.
//
// Note that an EnvSpec is recursive and therefore in "reverse" order: the last
// element is the base setting and each preceeding element is applied before
// elements that preceed it but after the elements that follow.
//
// Invariants:
//
//  * No references to v after an EnvRemove(v)
//
//  * No references to v after an EnvAdd(v)
//
//  * Any similar modifications (EnvAppend, EnvPrepend) to the same var are
//    collapsed together; there should be only one EnvAppend or EnvPrepend for a
//    particular var
//
//  * Any modifications (EnvAppend, EnvPrepend) to a var that is EnvAdd are
//    collapsed directly into the EnvAdd.  Thus there is a single EnvAdd for a
//    specific variable, and any EnvAppend or EnvPrepend applies to an inherited
//    environment variable.

#[derive(Clone,Debug,PartialEq)]
pub enum EnvSpec {
    /// This is the default, which specifies that the parent process's
    /// environment is inherited when running the subprocess executable.
    StdEnv,

    /// This is the override that disables parent process inheritance: the
    /// subprocess executes with only the environment variables that are
    /// explicitly added after this statement.
    BlankEnv,

    /// Use [EnvSpec::add] to construct this.
    // Multiple EnvAdd for different environment variables could be commutative
    // in eq, but eq is only expected to be useful for testing, so we just use
    // the standard derivation of PartialEq.
    EnvAdd(String, String, SubEnvSpec),

    /// Use [EnvSpec::prepend] to construct this.
    EnvPrepend(String, String, String, SubEnvSpec),

    /// Use [EnvSpec::append] to construct this.
    EnvAppend(String, String, String, SubEnvSpec),

    /// Use [EnvSpec::rmv] to construct this.
    EnvRemove(String, SubEnvSpec)
}

#[derive(Clone,Debug,PartialEq)]
pub struct SubEnvSpec { pub(crate) se: Box<EnvSpec> }

enum Elide {
    All(String),
    ForAppend(String),
    ForPrepend(String),
}


impl EnvSpec {

    // Add self spec on top of other spec (left-biased except for the base).
    pub fn set_base(&self, base: &EnvSpec) -> EnvSpec {
        match self {
            EnvSpec::StdEnv => base.clone(),
            EnvSpec::BlankEnv => base.clone(),
            EnvSpec::EnvAdd(n, v, SubEnvSpec{se}) => se.set_base(base).add(n, v),
            EnvSpec::EnvRemove(n, SubEnvSpec{se}) => se.set_base(base).rmv(n),
            EnvSpec::EnvPrepend(n, v, s, SubEnvSpec{se}) => se.set_base(base).prepend(n, v, s),
            EnvSpec::EnvAppend(n, v, s, SubEnvSpec{se}) => se.set_base(base).append(n, v, s),
        }
    }

    fn elide(&self, what: &Elide) -> Box<EnvSpec>
    {
        match self {
            EnvSpec::EnvAdd(n, v, SubEnvSpec{se}) => {
                match what {
                    Elide::All(var_name) if n == var_name => (*se).clone(),
                    Elide::ForAppend(var_name) if n == var_name => (*se).clone(),
                    Elide::ForPrepend(var_name) if n == var_name => (*se).clone(),
                    _ => Box::new(EnvSpec::EnvAdd(n.clone(), v.clone(),
                                                  SubEnvSpec{se: se.elide(what)})),
                }
            }
            EnvSpec::EnvRemove(n, SubEnvSpec{se}) => {
                match what {
                    Elide::All(var_name) if n == var_name => (*se).clone(),
                    Elide::ForAppend(var_name) if n == var_name => (*se).clone(),
                    Elide::ForPrepend(var_name) if n == var_name => (*se).clone(),
                    _ => Box::new(EnvSpec::EnvRemove(n.clone(),
                                                     SubEnvSpec{se: se.elide(what)})),
                }
            }
            EnvSpec::EnvPrepend(n, v, s, SubEnvSpec{se}) => {
                match what {
                    Elide::All(var_name) if n == var_name => (*se).clone(),
                    Elide::ForPrepend(var_name) if n == var_name => (*se).clone(),
                    _ => Box::new(EnvSpec::EnvPrepend(n.clone(),
                                                      v.clone(),
                                                      s.clone(),
                                                      SubEnvSpec{se: se.elide(what)}))
                }
            }
            EnvSpec::EnvAppend(n, v, s, SubEnvSpec{se}) => {
                match what {
                    Elide::All(var_name) if n == var_name => (*se).clone(),
                    Elide::ForAppend(var_name) if n == var_name => (*se).clone(),
                    _ => Box::new(EnvSpec::EnvAppend(n.clone(),
                                                     v.clone(),
                                                     s.clone(),
                                                     SubEnvSpec{se: se.elide(what)}))
                }
            }
            EnvSpec::StdEnv => Box::new(EnvSpec::StdEnv),
            EnvSpec::BlankEnv => Box::new(EnvSpec::BlankEnv),
        }
    }

    fn join_prepend(&self, var: &String, value: &String, sep: &String)
                    -> Option<EnvSpec>
    {
        match self {
            EnvSpec::EnvAdd(n, v, SubEnvSpec{se}) => {
                if n == var {
                    Some(EnvSpec::EnvAdd(n.clone(),
                                         value.clone() + sep + v,
                                         SubEnvSpec{se: se.clone()}))
                } else {
                    se.join_prepend(var, value, sep)
                        .map(|t|
                             EnvSpec::EnvAdd(n.clone(), v.clone(),
                                             SubEnvSpec{se: Box::new(t)}))
                }
            }
            EnvSpec::EnvRemove(n, SubEnvSpec{se}) =>
                se.join_prepend(var, value, sep)
                .map(|t|
                     EnvSpec::EnvRemove(n.clone(), SubEnvSpec{se: Box::new(t)})),
            EnvSpec::EnvPrepend(n, v, s, SubEnvSpec{se}) =>
                if n == var {
                    Some(EnvSpec::EnvPrepend(n.clone(),
                                             value.clone() + sep + v,
                                             s.clone(),
                                             SubEnvSpec{se: se.clone()}))
                } else {
                    se.join_prepend(var, value, sep)
                        .map(|t|
                             EnvSpec::EnvPrepend(n.clone(), v.clone(), s.clone(),
                                                 SubEnvSpec{se: Box::new(t)}))
                }
            EnvSpec::EnvAppend(n, v, s, SubEnvSpec{se}) =>
                se.join_prepend(var, value, sep)
                .map(|t|
                     EnvSpec::EnvAppend(n.clone(), v.clone(), s.clone(),
                                        SubEnvSpec{se: Box::new(t)})),
            EnvSpec::StdEnv => None,
            EnvSpec::BlankEnv => None,
        }
    }

    fn join_append(&self, var: &String, value: &String, sep: &String)
                   -> Option<EnvSpec>
    {
        match self {
            EnvSpec::EnvAdd(n, v, SubEnvSpec{se}) => {
                if n == var {
                    Some(EnvSpec::EnvAdd(n.clone(),
                                         v.clone() + sep + value,
                                         SubEnvSpec{se: se.clone()}))
                } else {
                    se.join_append(var, value, sep)
                        .map(|t|
                             EnvSpec::EnvAdd(n.clone(), v.clone(),
                                             SubEnvSpec{se: Box::new(t)}))
                }
            }
            EnvSpec::EnvRemove(n, SubEnvSpec{se}) =>
                se.join_append(var, value, sep)
                .map(|t|
                     EnvSpec::EnvRemove(n.clone(), SubEnvSpec{se: Box::new(t)})),
            EnvSpec::EnvPrepend(n, v, s, SubEnvSpec{se}) =>
                se.join_append(var, value, sep)
                .map(|t|
                     EnvSpec::EnvPrepend(n.clone(), v.clone(), s.clone(),
                                         SubEnvSpec{se: Box::new(t)})),
            EnvSpec::EnvAppend(n, v, s, SubEnvSpec{se}) =>
                if n == var {
                    Some(EnvSpec::EnvAppend(n.clone(),
                                            v.clone() + sep + value,
                                            s.clone(),
                                            SubEnvSpec{se: se.clone()}))
                } else {
                    se.join_append(var, value, sep)
                        .map(|t|
                             EnvSpec::EnvAppend(n.clone(), v.clone(), s.clone(),
                                                SubEnvSpec{se: Box::new(t)}))
                }
            EnvSpec::StdEnv => None,
            EnvSpec::BlankEnv => None,
        }
    }

    /// Adds the environment variable with the specified name and value to the
    /// environment for the execution of the operation.  Replaces any previous
    /// setting for this environment variable, whether explicitly set via a
    /// previous [EnvSpec::add] operation or inherited from the parent
    /// environment.
    pub fn add<N,V>(&self, var_name: N, var_value: V) -> Self
    where N: Into<String>,
          V: Into<String>
    {
        let vname = var_name.into();
        EnvSpec::EnvAdd(vname.clone(),
                        var_value.into(),
                        SubEnvSpec{se: self.elide(&Elide::All(vname))})
    }

    /// Prepends a value (with the specified separator between the prepended
    /// value and any existing, non-blank value) to the specified environment
    /// variable.  If there was no previous setting for this environment
    /// variable, it is newly created with the specified value.  Useable for both
    /// explicit and inherited environment variables.
    pub fn prepend<N,V,S>(&self, var: N, value: V, sep: S) -> Self
    where N: Into<String>,
          V: Into<String>,
          S: Into<String>
    {
        let vname = var.into();
        let val = value.into();
        let s = sep.into();
        match self.join_prepend(&vname, &val, &s) {
            None => EnvSpec::EnvPrepend(vname.clone(), val, s,
                                        SubEnvSpec{se: self.elide(&Elide::ForPrepend(vname))}),
            Some(e) => e
        }
    }

    /// Appends a value (with the specified separator between any existing,
    /// non-blank value and the appended value) to the specified environment
    /// variable.  If there was no previous setting for this environment
    /// variable, it is newly created with the specified value.  Useable for both
    /// explicit and inherited environment variables.
    pub fn append<N,V,S>(&self, var: N, value: V, sep: S) -> Self
    where N: Into<String>,
          V: Into<String>,
          S: Into<String>
    {
        let vname = var.into();
        let val = value.into();
        let s = sep.into();
        match self.join_append(&vname, &val, &s) {
            None => EnvSpec::EnvAppend(vname.clone(), val, s,
                                       SubEnvSpec{se: self.elide(&Elide::ForAppend(vname))}),
            Some(e) => e,
        }
    }

    /// Removes the specified environment variable from the execution
    /// environment.  Removes both inherited or explicit environment variables,
    /// and is a no-op if no environment variable with the specified name exists.
    pub fn rmv<N>(&self, var_name: N) -> Self
    where N: Into<String>
    {
        let vname = var_name.into();
        EnvSpec::EnvRemove(vname.clone(),
                           SubEnvSpec{se: self.elide(&Elide::All(vname))})
    }
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

// Modifications to Command environment settings.  Expects the EnvSpec to be
// normalized to its invariants as specified in the [EnvSpec] documentation.
fn update_env<'a>(cmnd: &'a mut process::Command,
                  espec: &'a EnvSpec) -> &'a mut process::Command
{
    match espec {
        EnvSpec::StdEnv => cmnd,
        EnvSpec::BlankEnv => cmnd.env_clear(),
        EnvSpec::EnvRemove(n, SubEnvSpec{se}) =>
            update_env(cmnd.env_remove(n), se),
        EnvSpec::EnvAdd(n, v, SubEnvSpec{se}) =>
            update_env(cmnd.env(n, v), se),
        EnvSpec::EnvAppend(n, v, s, SubEnvSpec{se}) => {
            match vars().find(|(vn,_)| vn == n) {
                None => update_env(cmnd.env(n, v), se),
                Some((_,orig_val)) => {
                    let vnew = orig_val + s + v;
                    update_env(cmnd.env(n, vnew), se)
                }
            }
        }
        EnvSpec::EnvPrepend(n, v, s, SubEnvSpec{se}) => {
            match vars().find(|(vn,_)| vn == n) {
                None => update_env(cmnd.env(n, v), se),
                Some((_,orig_val)) => {
                    let vnew = v.to_owned() + s + &orig_val;
                    update_env(cmnd.env(n, vnew), se)
                }
            }
        }
    }
}

impl OsRun for Executor {

    fn run_executable(&self,
                      label: &str,
                      exe_file: &Path,
                      args: &Vec<OsString>,
                      exe_env: &EnvSpec,
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
                        match update_env(process::Command::new(&exe_file)
                                         .args(args)
                                         .current_dir(&tgtdir)
                                         .stdout(process::Stdio::piped())
                                         .stderr(process::Stdio::piped()),
                                         exe_env).spawn()
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


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_env_adds_deduplicate() {
        assert_eq!(
            EnvSpec::BlankEnv
                .add("foo", "fooval")
                .add("bar", "barval")
                .add("bar", "bar value")
                .add("moo", "cow")
                .add("foo", "foo value"),
            EnvSpec::BlankEnv
                .add("bar", "bar value")
                .add("moo", "cow")
                .add("foo", "foo value"))
    }

    #[test]
    fn test_env_removes_deduplicate() {
        assert_eq!(
            EnvSpec::BlankEnv
                .rmv("foo")
                .rmv("bar")
                .rmv("bar")
                .rmv("moo")
                .rmv("foo"),
            EnvSpec::BlankEnv
                .rmv("bar")
                .rmv("moo")
                .rmv("foo"))
    }

    #[test]
    fn test_env_prepends_combine() {
        assert_eq!(
            EnvSpec::BlankEnv
                .add("foo", "fooval")
                .prepend("bar", "barval", "*")
                .append("bar", "after-bar", "->")
                .prepend("bar", "prebar", ":")
                .prepend("foo", "moo", "+")
                .prepend("moo", "cow", "!!")
                .prepend("foo", "end", ""),
            EnvSpec::BlankEnv
                .add("foo", "endmoo+fooval")
                .prepend("bar", "prebar:barval", "*")
                .append("bar", "after-bar", "->")
                .prepend("moo", "cow", "!!"))
    }

    #[test]
    fn test_env_appends_combine() {
        assert_eq!(
            EnvSpec::BlankEnv
                .add("foo", "fooval")
                .append("bar", "barval", "*")
                .append("bar", "after-bar", "->")
                .prepend("bar", "into", "::")
                .append("foo", "moo", "+")
                .prepend("moo", "a", " ")
                .append("moo", "cow", "!!")
                .append("foo", "end", ""),
            EnvSpec::BlankEnv
                .add("foo", "fooval+mooend")
                .append("bar", "barval->after-bar", "*")
                .prepend("bar", "into", "::")
                .prepend("moo", "a", " ")
                .append("moo", "cow", "!!"))
    }

    #[test]
    fn test_env_adds_and_removes_deduplicate() {
        assert_eq!(
            EnvSpec::BlankEnv
                .rmv("foo")
                .add("bar", "barval")
                .rmv("bar")
                .rmv("moo")
                .add("moo", "cow")
                .add("foo", "fooval"),
            EnvSpec::BlankEnv
                .rmv("bar")
                .add("moo", "cow")
                .add("foo", "fooval"))
    }

    #[test]
    fn test_env_adds_and_removes_and_updates_deduplicate() {
        assert_eq!(
            EnvSpec::StdEnv
                .rmv("foo")
                .add("bar", "barval")
                .add("foo", "huh")
                .append("bar", "post-bar", ",")
                .prepend("bar", "pre-bar", ",,,,")
                .rmv("bar")
                .rmv("moo")
                .append("moo", "cow", "+")
                .prepend("foo", "what", " is ")
                .add("foo", "fooval"),
            EnvSpec::StdEnv
                .rmv("bar")
                .append("moo", "cow", "+")
                .add("foo", "fooval"))
    }

    #[test]
    fn test_env_set_base() {
        assert_eq!(
            EnvSpec::StdEnv
                .add("foo", "fooval")
                .add("bar", "barval")
                .append("dog", "bark", ", ")
                .rmv("cow")
                .prepend("quux", "capacitor", "**")
                .append("dog", "!", "")
                .set_base(&EnvSpec::BlankEnv
                          .add("foo", "first foo")
                          .append("bar", "first bar", "@")
                          .add("cow", "moo")
                          .add("dog", "bark")
                          .rmv("quux")
                          .prepend("dog", "says", ":")
                ),
            EnvSpec::BlankEnv
                .add("dog", "says:bark, bark!")
                .add("foo", "fooval")
                .add("bar", "barval")
                .rmv("cow")
                .prepend("quux", "capacitor", "**")
        )
    }
}
