use std::path::{Path,PathBuf};
use std::rc::Rc;
use filesprep_derive::*;

use crate::filehandling::*;
use crate::errors::*;
use crate::operations::generic::*;
use crate::execution::{OsRun,OsRunResult::*};


/// This structure represents a single command that is performed via a local code
/// function instead of running an `Executable` in a `SubProcOperation`.  This
/// can be used for operations in a chain that the local program performs and
/// avoids the need to create multiple chains around this functionality.  For
/// example, a chain of operations that midway through creates a tar file could
/// use a `SubProcOperation` with the `Executable("tar")` or it could use a
/// `FunctionOperation` that uses the Rust `tar::Builder` to generate the tar
/// file via Rust functionality.
///
/// The first argument to the called function is the reference directory, the
/// second is the input file(s) that should be processed, and the last is the
/// output file that should be generated.
///
/// The reference directory would be the current directory for the command had it
/// been a `SubProcOperation`. The actual current directory for this process is
/// *not* set to this reference directory; handling of the reference directory is
/// left up to the called function.
#[derive(Clone,FilesTransformationPrep)]
pub struct FunctionOperation {
    name : String,  // for informational purposes only
    call : Rc<dyn Fn(&Path, &ActualFile, &ActualFile) -> anyhow::Result<()>>,
               // n.b. Would prefer this to be an FnOnce, but that breaks move
               // semantics when trying to call it while it's a part of an
               // enclosing Enum.
    files : FileTransformation,
}

impl std::fmt::Debug for FunctionOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
    {
        format!("Local function call '{}' {:?}", self.name, self.files).fmt(f)
    }
}

impl FunctionOperation {

    /// Creates a new FunctionOperation that will call a local function instead of
    /// executing a command in a sub-process.  This is useful for interleaving
    /// local processing into the command chain where that local processing is
    /// executed in proper sequence with the other commands.  The local function
    /// is provided with the "argument list" that would have been passed on the
    /// command-line; this argument list will contain any input or output
    /// filenames that should be used by the function.
    ///
    /// A local function execution in the chain can only pass an output file to
    /// the subsequent operation in the chain; more complex data exchange would
    /// need to be serialized into that output file and appropriately consumed by
    /// the next stage. This might initially seem awkward, but makes sense when
    /// you consider that most operations are executions in subprocesses that are
    /// in a separate address space already.
    pub fn calling<T>(n: &str, f: T) -> FunctionOperation
    where T: Fn(&Path, &ActualFile, &ActualFile) -> anyhow::Result<()> + 'static
    {
        FunctionOperation {
            name : n.to_string(),
            call : Rc::new(f),
            files : FileTransformation::new(),
        }
    }

    fn run_with_files<Exec, P>(&self,
                               executor: &mut Exec,
                               cwd: &Option<P>,
                               inpfiles: ActualFile,
                               outfile: ActualFile)
                               -> anyhow::Result<ActualFile>
    where P: AsRef<Path>, Exec: OsRun
    {
        let fromdir: Option<PathBuf> =
            match cwd {
                Some(root) => match &self.files.in_dir {
                    Some(sub) => Some(root.as_ref().to_path_buf().join(sub)),
                    None => Some(root.as_ref().to_path_buf()),
                },
                None => self.files.in_dir.clone(),
            };
        match executor.run_function(self.name.as_str(), &self.call,
                                    &inpfiles, &outfile, &fromdir) {
            Good => Ok(outfile),
            ExecFailed(e) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorCmdSetup(format!("{:?}", self),
                                                Vec::new(), e,
                                                fromdir))),
            RunError(e) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorExecuting(format!("{:?}", self),
                                                 Vec::new(), e,
                                                 fromdir))),
            ExecError(c,s) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorRunningCmd(
                        format!("{:?}", self), Vec::new(),
                        c, fromdir, s))),
            BadDirectory(p,e) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorBadDirectory(
                        format!("{:?}", self), p, e))),
        }
    }
}

impl OpInterface for FunctionOperation {

    fn label(&self) -> String { self.name.clone() }

    fn set_label(&mut self, new_label: &str) -> &mut Self {
        self.name = new_label.to_string();
        self
    }

    fn execute<Exec, P>(&mut self, executor: &mut Exec, cwd: &Option<P>)
                        -> anyhow::Result<ActualFile>
    where P: AsRef<Path>, Exec: OsRun
    {
        let inpfiles =
            self.files.inp_filenames.iter().try_fold(
                ActualFile::NoActualFile,
                |dfs, inpf|
                setup_file(executor, inpf,
                           || Ok(ActualFile::NoActualFile)
                ).and_then(|df| Ok(dfs.extend(df)))
        )?;
        let outfile = setup_file(executor, &self.files.out_filename,
                                 || Ok(ActualFile::NoActualFile),
        )?;
        self.run_with_files(executor, cwd, inpfiles, outfile)
    }
}


// ----------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {

    use super::*;
    use crate::execution::*;
    use std::rc::Rc;
    use std::ffi::OsString;

    #[derive(Debug, PartialEq)]
    struct RunFunc{
        fname: String,
        inpfiles: Vec<PathBuf>,
        outfile: Option<PathBuf>,
        dir: Option<PathBuf>
    }
    #[derive(Debug, PartialEq)]
    struct CallCollector(Vec<RunFunc>);

    impl OsRun for CallCollector {
        fn run_executable(&mut self,
                          label: &str,
                          exe_file: &Path,
                          _args: &Vec<OsString>,
                          _fromdir: &Option<PathBuf>) -> OsRunResult
        {
            RunError(anyhow::anyhow!("run_executable {:?}: {:?} not implemented for CallCollector",
                                     label, exe_file))
        }
        fn run_function(&mut self,
                        name : &str,
                        _call : &Rc<dyn Fn(&Path, &ActualFile, &ActualFile) -> anyhow::Result<()>>,
                        inpfiles: &ActualFile,
                        outfile: &ActualFile,
                        fromdir: &Option<PathBuf>) -> OsRunResult
        {
            self.0.push(RunFunc{ fname: name.to_string(),
                                 inpfiles: inpfiles.to_paths::<PathBuf>(&None).unwrap(),
                                 outfile: outfile.to_path::<PathBuf>(&None).ok(),
                                 dir: fromdir.clone()
            });
            Good
        }
        fn glob_search(&mut self, _globpat: &String) -> anyhow::Result<Vec<PathBuf>>
        {
            Err(anyhow::anyhow!("glob_search not implemented for CallCollector"))
        }
        fn mk_tempfile(&mut self, suffix: &String) -> anyhow::Result<tempfile::NamedTempFile>
        {
            Executor::DryRun.mk_tempfile(suffix)
        }
    }

    fn test_callee(_indir: &Path,
                   _inpfiles: &ActualFile,
                   _outfile: &ActualFile) -> anyhow::Result<()>
    {
        todo!("test_callee not implemented")
    }


    #[test]
    fn test_func_with_files() -> () {
        let mut op = FunctionOperation::calling("f1", test_callee)
            .set_input_file(&FileArg::loc("inpfile.txt"))
            .set_output_file(&FileArg::temp(".out"))
            .clone();

        let mut executor = CallCollector(vec![]);
        let result = execute_here(&mut op, &mut executor);
        assert!(
            match result {
                Ok(ActualFile::SingleFile(FileRef::TempFile(ref tf))) =>
                    tf.path().exists(),
                _ => false
            }, "Unexpected result: {:?}", result);
        assert_eq!(executor.0.len(), 1);
        assert_eq!(executor.0[0].fname, "f1");
        assert!(
            match &executor.0[0].outfile {
                Some(pb) => pb.exists(),
                _ => false
            }, "Unexpected outfiles: {:?}", executor.0[0].outfile);
        let out1 = &executor.0[0].outfile.clone().unwrap();
        executor.0[0].outfile = None;
        assert_eq!(executor.0,
                   vec![ RunFunc { fname: "f1".into(),
                                   inpfiles: vec![PathBuf::from("inpfile.txt")],
                                   outfile: None,
                                   dir: None,
                                   },
                   ]);

        // Re-run op to make sure it can be re-used
        let mut ex2 = CallCollector(vec![]);
        let result2 = execute_here(&mut op, &mut ex2);
        assert!(
            match result2 {
                Ok(ActualFile::SingleFile(FileRef::TempFile(ref tf))) =>
                    tf.path().exists(),
                _ => false
            }, "Unexpected result: {:?}", result2);
        assert_eq!(ex2.0.len(), 1);
        assert_eq!(ex2.0[0].fname, "f1");
        assert!(
            match &ex2.0[0].outfile {
                Some(pb) => pb.exists() && pb != out1,
                _ => false
            }, "Unexpected outfiles: {:?}", ex2.0[0].outfile);
        ex2.0[0].outfile = None;

        assert_eq!(executor, ex2);
    }
}
