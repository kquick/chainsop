use anyhow::Context;
use std::ffi::{OsString};
use std::path::{Path, PathBuf};
use filesprep_derive::*;

use crate::filehandling::*;
use crate::executable::*;
use crate::errors::*;
use crate::operations::generic::*;
use crate::execution::{OsRun, OsRunResult::*};



/// This structure represents a single command to run as a sub-process, the
/// command's arguments, and the input and output files for that sub-process.
/// The structure itself is public but the fields are private
/// (i.e. implementation specific); the impl section below defines the visible
/// operations that can be performed on this structure.
#[derive(Clone,Debug,FilesTransformationPrep)]
pub struct SubProcOperation {
    name : String,
    exec : Executable,
    args : Vec<OsString>,
    files : FileTransformation,
}



impl SubProcOperation {

    /// This is the primary method of initializing a SubProcOperation.
    pub fn new(executing : &Executable) -> SubProcOperation
    {
        SubProcOperation {
            name : (executing.exe_file
                    .clone()
                    .into_os_string()
                    .into_string()
                    .unwrap_or("{an-exe}".to_string())),
            exec : executing.clone(),
            args : get_base_args(&executing).iter().map(|x| x.into()).collect(),
            files : FileTransformation::new(),
        }
    }


    /// Changes the name of the command to execute.
    #[inline]
    pub fn set_executable<T>(&mut self, exe: T) -> &mut SubProcOperation
    where T: Into<PathBuf>
    {
        self.exec = self.exec.set_exe(exe);
        self.name =
            self.exec.exe_file
            .clone()
            .into_os_string()
            .into_string()
            .unwrap_or("{an-exe}".to_string());
        self
    }


    /// Adds an argument to use when executing the operation.  This can, for
    /// example, be used for specifying command-line option arguments when
    /// running a subprocess Executable operation.  Each operation type and
    /// instance can determine how it will handle any specified arguments.
    #[inline]
    pub fn push_arg<T>(&mut self, arg: T) -> &mut Self
    where T: Into<OsString>
    {
        self.args.push(arg.into());
        self
    }

    /// Prepares the final/actual argument list that is to be presented to the
    /// command, including lookup and preparation of files that are referenced by
    /// the command.  This function is normally only used internally by the
    /// execute() operation, but it is exposed for testing purposes.
    fn finalize_args<Exec, P>(&self,
                              executor: &Exec,
                              cwd: &Option<P>)
                              -> anyhow::Result<(Vec<OsString>,
                                                 (ActualFile, ActualFile))>
    where Exec: OsRun, P: AsRef<Path>
    {
        let mut args = self.args.clone();
        let files = self.cmd_file_setup(executor, &mut args, cwd)?;
        Ok((args, files))
    }

    // Sets up file references for running a command.  Note that these are
    // relative to the cwd specified for this operation, which might not yet be
    // the current working directory.
    fn cmd_file_setup<Exec, P>(&self, executor: &Exec,
                               args: &mut Vec<OsString>,
                               cwd: &Option<P>)
                               -> anyhow::Result<(ActualFile, ActualFile)>
    where Exec: OsRun, P: AsRef<Path>
    {
        let inpfiles;
        let outfile;
        let missing_file_err = ||
            Err(anyhow::Error::new(ChainsopError::ErrorMissingFile));
        let errctxt = |w| move || format!("Setting {} file for {:?}", w, self.exec);

        // Note: order of file specification is important below because
        // setup_file has side-effects of modifying the args.
        if self.emit_output_file_first() {
            outfile = self.setup_exe_file(executor,
                                          args,
                                          &cwd,
                                          &get_outfile(&self.exec),
                                          &self.files.out_filename,
                                          missing_file_err)
                .with_context(errctxt("output (first)"))?;
            inpfiles = self.files.inp_filenames.iter()
                .try_fold(ActualFile::NoActualFile,
                          |dfs, inpf|
                          self.setup_exe_file(executor,
                                              args,
                                              &cwd,
                                              &get_inpfile(&self.exec),
                                              &inpf,
                                              missing_file_err)
                          .and_then(|df| Ok(dfs.extend(df))))
                .with_context(errctxt("output (append)"))?;
        } else {
            inpfiles = self.files.inp_filenames.iter()
                .try_fold(ActualFile::NoActualFile,
                          |dfs, inpf|
                          self.setup_exe_file(executor,
                                              args,
                                              &cwd,
                                              &get_inpfile(&self.exec),
                                              &inpf,
                                              missing_file_err)
                          .and_then(|df| Ok(dfs.extend(df))))
                .with_context(errctxt("output (append)"))?;
            outfile = self.setup_exe_file(executor,
                                          args,
                                          &cwd,
                                          &get_outfile(&self.exec),
                                          &self.files.out_filename,
                                          missing_file_err)
                .with_context(errctxt("output (append)"))?;
        }
        Ok((inpfiles, outfile))
    }

    /// Output option arguments before positional arguments because some
    /// command's parsers are limited in this way.  This function returns true if
    /// the output file should be specified before the input file; the normal
    /// order is input file and then output file (e.g. "cp inpfile outfile").
    fn emit_output_file_first(&self) -> bool
    {
        if let ExeFileSpec::Option(_) = get_outfile(&self.exec) {
            if let ExeFileSpec::Append = get_inpfile(&self.exec) {
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Resolves a FileSpec and inserts the actual named file into the argument
    /// list.  This also returns the file; the file may be a temporary file
    /// object which will delete the file at the end of its lifetime, so the
    /// returned value should be held until the file is no longer needed.
    fn setup_exe_file<E, Exec, P>(&self,
                                  executor: &Exec,
                                  args: &mut Vec<OsString>,
                                  cwd: &Option<P>,
                                  spec: &ExeFileSpec,
                                  candidate: &FileArg,
                                  on_missing: E)
                                  -> anyhow::Result<ActualFile>
    where E: Fn() -> anyhow::Result<ActualFile>,
          Exec: OsRun,
          P: AsRef<Path>
    {
        match spec {
            ExeFileSpec::NoFileUsed => Ok(ActualFile::NoActualFile),
            ExeFileSpec::Append => {
                let sf = setup_file(executor, candidate, on_missing)?;
                let pths = sf.to_paths::<PathBuf>(&None)?;
                for pth in pths {
                    args.push(OsString::from(pth.clone().into_os_string()));
                }
                Ok(sf)
            }
            ExeFileSpec::Option(optflag) => {
                let sf = setup_file(executor, candidate, on_missing)?;
                let pths = sf.to_paths::<PathBuf>(&None)?;
                let fnames = pths.iter()
                    .map(|x| x.to_str().unwrap()).collect::<Vec<_>>();
                if optflag.ends_with("=") {
                    args.push(OsString::from(optflag.to_owned() +
                                             &fnames.join(",")));
                } else {
                    args.push(OsString::from(optflag));
                    args.push(OsString::from(fnames.join(",")));
                };
                Ok(sf)
            }
            ExeFileSpec::ViaCall(userfun) => {
                let sf = setup_file(executor, candidate, on_missing)?;
                userfun(args,
                        &(cwd.as_ref().map(|p| p.as_ref().to_path_buf())),
                        &sf)?;
                Ok(sf)
            }
        }
    }

    /// After the files are setup, this performs the actual run.  See the
    /// documentation for `OpInterface::execute()` above for a description of the
    /// handling of the `cwd` parameter.
    fn run_cmd<Exec, P>(&self,
                        executor: &Exec,
                        cwd: &Option<P>,
                        outfile : ActualFile,
                        args : Vec<OsString>)
                        -> anyhow::Result<ActualFile>
    where P: AsRef<Path>, // T: Clone,
          Exec: OsRun
    {
        let fromdir: Option<PathBuf> =
            match cwd {
                Some(root) => match &self.files.in_dir {
                    Some(sub) => Some(root.as_ref().to_path_buf().join(sub)),
                    None => Some(root.as_ref().to_path_buf()),
                },
                None => self.files.in_dir.clone(),
            };
        match executor.run_executable(&self.label(),
                                      &self.exec.exe_file, &args, &fromdir) {
            Good => Ok(outfile),
            RunError(e) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorExecuting(format!("{:?}", self.exec),
                                                 args, e, fromdir))),
            ExecFailed(e) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorCmdSetup(format!("{:?}", self.exec),
                                                args, e, fromdir))),
            ExecError(c,s) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorRunningCmd(
                        format!("{:?}", self.exec), args,
                        c, fromdir, s))),
            BadDirectory(p,e) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorBadDirectory(
                        format!("{:?}", self.exec), p, e))),
        }
    }
}


impl OpInterface for SubProcOperation {

    fn label(&self) -> String { self.name.clone() }

    fn set_label(&mut self, new_label: &str) -> &mut Self {
        self.name = new_label.to_string();
        self
    }

    fn execute<Exec, P>(&mut self, executor: &Exec, cwd: &Option<P>)
                        -> anyhow::Result<ActualFile>
    where P: AsRef<Path>,
          Exec: OsRun
    {
        let (args, (_inpfiles, outfile)) = self.finalize_args(executor, cwd)?;
        self.run_cmd(executor, cwd, outfile, args)
    }

}


// ----------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {

    use super::*;
    use crate::execution::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug, PartialEq)]
    struct RunExec {
        name: String,
        exe: PathBuf,
        args: Vec<OsString>,
        dir: Option<PathBuf>
    }
    struct ArgCollector(RefCell<Vec<RunExec>>);
    impl ArgCollector {
        pub fn new() -> ArgCollector {
            ArgCollector(RefCell::new(vec![]))
        }
    }

    impl OsRun for ArgCollector {
        fn run_executable(&self,
                          label: &str,
                          exe_file: &Path,
                          args: &Vec<OsString>,
                          fromdir: &Option<PathBuf>) -> OsRunResult
        {
            self.0.borrow_mut()
                .push(RunExec{ name: String::from(label),
                               exe: PathBuf::from(exe_file),
                               args: args.clone(),
                               dir: fromdir.clone()
            });
            Good
        }
        fn run_function(&self,
                        name : &str,
                        _call : &Rc<dyn Fn(&Path, &ActualFile, &ActualFile) -> anyhow::Result<()>>,
                        _inpfiles: &ActualFile,
                        _outfile: &ActualFile,
                        _fromdir: &Option<PathBuf>) -> OsRunResult
        {
            RunError(anyhow::anyhow!("run_function {} not implemented for ArgCollector", name))
        }
        fn glob_search(&self, _globpat: &String) -> anyhow::Result<Vec<PathBuf>>
        {
            Err(anyhow::anyhow!("glob_search not implemented for ArgCollector"))
        }
        fn mk_tempfile(&self, suffix: &String) -> anyhow::Result<tempfile::NamedTempFile>
        {
            Executor::DryRun.mk_tempfile(suffix)
        }
    }

    #[test]
    fn test_append_append() -> () {
        let exe = Executable::new(&"test-cmd",
                                  ExeFileSpec::Append,
                                  ExeFileSpec::Append);
        let mut op = SubProcOperation::new(&exe)
            .set_input_file(&FileArg::loc("inpfile.txt"))
            .set_output_file(&FileArg::temp(".out"))
            .push_arg("-a")
            .push_arg("a-arg-value")
            .push_arg("-b")
            .clone();

        let mut executor = ArgCollector::new();
        let result = execute_here(&mut op, &mut executor);
        assert!(
            match result {
                Ok(ActualFile::SingleFile(FileRef::TempFile(ref tf))) =>
                    tf.path().exists(),
                _ => false
            }, "Unexpected result: {:?}", result);
        let mut collected = executor.0.into_inner();
        assert_eq!(collected.len(), 1);
        // The last arg is an assigned tempfile
        let output_tmpfile = PathBuf::from(&collected[0].args.last().unwrap());
        assert!(output_tmpfile.exists());
        collected[0].args.pop();
        assert_eq!(collected,
                   vec![ RunExec { name: "test-cmd".into(),
                                   exe: "test-cmd".into(),
                                   args: ["-a",
                                          "a-arg-value",
                                          "-b",
                                          "inpfile.txt",
                                   ].map(Into::<OsString>::into).to_vec(),
                                   dir: None,
                   },
                   ]);
    }

    #[test]
    fn test_append_option() -> () {
        let exe = Executable::new(&"test-cmd",
                                  ExeFileSpec::Append,
                                  ExeFileSpec::Option("-o".into()));
        let mut op = SubProcOperation::new(&exe)
            .set_input_file(&FileArg::loc("inpfile.txt"))
            .set_output_file(&FileArg::loc("outfile.out"))
            .set_dir("sub/dir")
            .push_arg("-a")
            .push_arg("a-arg-value")
            .add_input_file(&FileArg::loc("inp2.foo"))
            .push_arg("-b")
            .clone();

        let mut executor = ArgCollector::new();
        let result = op.execute(&mut executor, &Some("/other/location"));
        assert!(match result {
            Ok(ActualFile::SingleFile(FileRef::StaticFile(p))) =>
                p == PathBuf::from("outfile.out"),
            _ => false
        });
        let collected = executor.0.into_inner();
        assert_eq!(collected,
                   vec![ RunExec { name: "test-cmd".into(),
                                   exe: "test-cmd".into(),
                                   args: ["-a",
                                          "a-arg-value",
                                          "-b",
                                          "-o",
                                          "outfile.out",
                                          "inpfile.txt",
                                          "inp2.foo",
                                   ].map(Into::<OsString>::into).to_vec(),
                                   dir: Some(PathBuf::from("/other/location/sub/dir")),
                   }]);

        // Re-run op to make sure it can be re-used
        let mut exec2 = ArgCollector::new();
        let result2 = op.execute(&mut exec2, &Some("loc"));
        assert!(match result2 {
            Ok(ActualFile::SingleFile(FileRef::StaticFile(p))) =>
                p == PathBuf::from("outfile.out"),
            _ => false
        });
        let collected = exec2.0.into_inner();
        assert_eq!(collected,
                   vec![ RunExec { name: "test-cmd".into(),
                                   exe: "test-cmd".into(),
                                   args: ["-a",
                                          "a-arg-value",
                                          "-b",
                                          "-o",
                                          "outfile.out",
                                          "inpfile.txt",
                                          "inp2.foo",
                                   ].map(Into::<OsString>::into).to_vec(),
                                   dir: Some(PathBuf::from("loc/sub/dir")),
                   }]);
    }

    #[test]
    fn test_path_and_new_exe() -> () {
        let mut op = SubProcOperation::new(&Executable::new(&"test-cmd",
                                                            ExeFileSpec::NoFileUsed,
                                                            ExeFileSpec::NoFileUsed))
            .set_dir("sub/dir")
            .push_arg("-a")
            .clone();
        op.set_executable(&"simple");

        let mut executor = ArgCollector::new();
        let result = op.execute(&mut executor, &None::<PathBuf>);
        assert!(match result {
            Ok(ActualFile::NoActualFile) => true,
            _ => false
        });
        let collected = executor.0.into_inner();
        assert_eq!(collected,
                   vec![ RunExec { name: "simple".into(),
                                   exe: "simple".into(),
                                   args: ["-a",
                                   ].map(Into::<OsString>::into).to_vec(),
                                   dir: Some(PathBuf::from("sub/dir")),
                   }]);
    }


}
