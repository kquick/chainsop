use std::ffi::{OsString};
use std::path::{Path, PathBuf};
use anyhow;

use crate::errors::ChainsopError;
use crate::filehandling::defs::*;
use crate::execution::OsRun;


impl ActualFile {

    pub fn extend(self, more: ActualFile) -> ActualFile
    {
        match self {
            ActualFile::NoActualFile => more,
            ActualFile::SingleFile(pb) =>
                match more {
                    ActualFile::NoActualFile => ActualFile::SingleFile(pb),
                    ActualFile::SingleFile(pb2) => ActualFile::MultiFile(vec![pb, pb2]),
                    ActualFile::MultiFile(mut pbs) =>
                        ActualFile::MultiFile({pbs.push(pb); pbs}),
                },
            ActualFile::MultiFile(mut pbs) =>
                match more {
                    ActualFile::NoActualFile => ActualFile::MultiFile(pbs),
                    ActualFile::SingleFile(pb2) =>
                        ActualFile::MultiFile({pbs.push(pb2); pbs}),
                    ActualFile::MultiFile(pbs2) =>
                        ActualFile::MultiFile({pbs.extend(pbs2); pbs}),
                }
        }
    }

    /// Gets the Path associated with a ActualFile or returns an error if there
    /// is no Path.  This expects there to be a single path and will generate an
    /// error if there is no file or there are multiple files.  The cwd is
    /// provided to determine the location for relative paths.
    pub fn to_path<P>(&self, cwd: &Option<P>) -> anyhow::Result<PathBuf>
    where P: AsRef<Path>
    {
        match self {
            ActualFile::SingleFile(fref) => Ok(Self::get_path(cwd, fref)),
            ActualFile::NoActualFile =>
                Err(anyhow::Error::new(ChainsopError::ErrorMissingFile)),
            ActualFile::MultiFile(_) =>
                Err(anyhow::Error::new(
                    ChainsopError::ErrorUnsupportedActualFile(
                        format!("{:?}", self)))),
        }
    }

    fn get_path<P: AsRef<Path>>(cwd: &Option<P>, fref: &FileRef) -> PathBuf {
        let mut tgt = PathBuf::new();
        match cwd {
            Some(d) => { tgt.push(d.as_ref()); }
            None => {}
        };
        match fref {
            FileRef::StaticFile(pb) => tgt.push(pb),
            FileRef::TempFile(tf) => tgt.push(tf.path()),
        };
        tgt
    }

    /// Gets the list of Paths (one or more) associated with a ActualFile or
    /// returns an error if there is no Path.  The `to_path` method should be
    /// used if only a single path is expected, and this method should be used
    /// when there is a valid potential for multiple paths to exist.
    pub fn to_paths<P>(&self, cwd: &Option<P>) -> anyhow::Result<Vec<PathBuf>>
    where P: AsRef<Path>
    {
        match self {
            ActualFile::SingleFile(fref) =>
                Ok(vec![Self::get_path(cwd, &fref)]),
            ActualFile::MultiFile(pbs) =>
                Ok(pbs.iter().map(|p| Self::get_path(cwd, p)).collect()),
            ActualFile::NoActualFile =>
                Err(anyhow::Error::new(ChainsopError::ErrorMissingFile)),
        }
    }

}

/// Resolves a FileSpec and insert the actual named file into the argument
/// list.  This also returns the file; the file may be a temporary file
/// object which will delete the file at the end of its lifetime, so the
/// returned value should be held until the file is no longer needed.
pub fn setup_file<Exec, E>(executor: &Exec,
                           candidate: &FileArg,
                           on_missing: E) -> anyhow::Result<ActualFile>
where E: Fn() -> anyhow::Result<ActualFile>,
      Exec: OsRun
{
    match candidate {
        FileArg::TBD => on_missing(),
        FileArg::Temp(sfx) => {
            let tf = executor.mk_tempfile(sfx)?;
            Ok(ActualFile::SingleFile(FileRef::TempFile(tf)))
        }
        FileArg::Loc(fpath) => {
            Ok(ActualFile::SingleFile(FileRef::StaticFile(fpath.clone())))
        }
        FileArg::GlobIn(dpath, glob) => {
            let mut fpaths = Vec::new();
            with_globbed_matches(
                executor, dpath, glob,
                |files| {
                    for each in files {
                        fpaths.push(FileRef::StaticFile(each.clone()));
                    };
                    Ok(())
                })?;
            Ok(ActualFile::MultiFile(fpaths))
        }
    }
}


fn with_globbed_matches<Exec, Do>(executor: &Exec,
                                  in_dir: &Path,
                                  for_glob: &str,
                                  mut do_with: Do)
                                  -> anyhow::Result<()>
where Do: FnMut(&Vec<PathBuf>) -> anyhow::Result<()>,
      Exec: OsRun
{
    let mut globpat = String::new();
    globpat.push_str(&OsString::from(in_dir).into_string().unwrap());
    globpat.push_str("/");
    globpat.push_str(for_glob);
    let glob_files = executor.glob_search(&globpat)?;
    do_with(&glob_files)
}

// ----------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {

    use super::*;
    use crate::execution::{Executor::*};
    use proptest::prelude::*;

    // Note: ActualFile testing is a bit tricky because ActualFile object
    // do not have the Clone trait (becauase FileRef::TempFile cannot be Cloned),
    // thus we cannot create a proptest Strategy that generates ActualFile
    // directly.

    fn num_files(inp: &ActualFile) -> usize {
        match inp {
            ActualFile::NoActualFile => 0,
            ActualFile::SingleFile(_) => 1,
            ActualFile::MultiFile(v) => v.len(),
        }
    }

    fn mkTestActualFile(n: usize) -> ActualFile {
        match n {
            0 => ActualFile::NoActualFile,
            1 => ActualFile::SingleFile(
                FileRef::StaticFile(PathBuf::from("one.file"))),
            2 => ActualFile::MultiFile(
                vec![FileRef::StaticFile(PathBuf::from("a.file")),
                     FileRef::StaticFile(PathBuf::from("b.txt")),
                ]),
            3 => ActualFile::MultiFile(
                vec![FileRef::StaticFile(PathBuf::from("a.1")),
                     FileRef::StaticFile(PathBuf::from("b.two")),
                     FileRef::StaticFile(PathBuf::from("c")),
                ]),
            _ => { assert!(false); ActualFile::NoActualFile },
        }
    }

    fn strategy_FileArg() -> impl Strategy<Value = FileArg> {
        prop_oneof![
            Just(FileArg::TBD),
            any::<PathBuf>().prop_map(FileArg::Loc),
            (any::<PathBuf>(), ".*").prop_map(|(p,n)| FileArg::GlobIn(p,n)),
            "[A-Za-z0-9_-]+".prop_map(FileArg::Temp),
        ]
    }

    proptest! {

        #[test]
        fn test_ActualFile_extend_count(n1 in (0usize..=3),
                                            n2 in (0usize..=3)) {
            // If one ActualFile is extended by another, the result should
            // have the combined number of files.
            let a = mkTestActualFile(n1);
            let b = mkTestActualFile(n2);
            prop_assert_eq!(num_files(&a), n1);
            prop_assert_eq!(num_files(&b), n2);
            let c = a.extend(b);
            prop_assert_eq!(num_files(&c), n1 + n2);
        }

        #[test]
        fn test_ActualFile_path_no_cwd(n in (0usize..=3)) {
            // A path can be extracted from a ActualFile of just one entry;
            // all other ActualFile sized should return an error.
            let a = mkTestActualFile(n);
            prop_assert_eq!(num_files(&a), n);
            match a.to_path::<PathBuf>(&None) {
                Ok(_p) => prop_assert_eq!(n, 1),
                Err(_) => prop_assert_ne!(n, 1),
            }
        }

        #[test]
        fn test_ActualFile_path_with_cwd(n in (0usize..=3)) {
            // A path can be extracted from a ActualFile of just one entry;
            // all other ActualFile sized should return an error.
            let a = mkTestActualFile(n);
            prop_assert_eq!(num_files(&a), n);
            match a.to_path(&Some("path/spec")) {
                Ok(_p) => prop_assert_eq!(n, 1),
                Err(_) => prop_assert_ne!(n, 1),
            }
        }

        #[test]
        fn test_ActualFile_paths_no_cwd(n in (0usize..=3)) {
            // A set of paths can be extracted from a ActualFile; the number
            // of extracted paths should match the size of the ActualFile,
            // and an error should be generated is there is no file actual.
            let a = mkTestActualFile(n);
            prop_assert_eq!(num_files(&a), n);
            match a.to_paths::<PathBuf>(&None) {
                Ok(ps) => prop_assert_eq!(n, ps.len()),
                Err(_) => prop_assert_eq!(n, 0),
            }
        }

        #[test]
        fn test_ActualFile_paths_with_cwd(n in (0usize..=3)) {
            // A set of paths can be extracted from a ActualFile; the number
            // of extracted paths should match the size of the ActualFile,
            // and an error should be generated is there is no file actual.
            let a = mkTestActualFile(n);
            prop_assert_eq!(num_files(&a), n);
            match a.to_paths(&Some("loc/path")) {
                Ok(ps) => prop_assert_eq!(n, ps.len()),
                Err(_) => prop_assert_eq!(n, 0),
            }
        }

        #[test]
        fn test_setup_file_dryrun(nfile in strategy_FileArg()) {
            // Verify the results from setup_file on various FileArg inputs,
            // using the DryRun executor
            match setup_file(&mut DryRun, &nfile,
                             || Err (anyhow::Error::new(
                                 ChainsopError::ErrorMissingFile))) {
                Ok(df) => match nfile {
                    FileArg::GlobIn(_,_) => prop_assert_eq!(num_files(&df), 0),
                    _ => prop_assert_eq!(num_files(&df), 1),
                }
                Err(e) => prop_assert_eq!(nfile.clone(), FileArg::TBD,
                                          "Expected error only on TBD but with {:?} got error {:?}",
                                          nfile, e),
            }
        }
    }

    #[test]
    fn test_paths_with_abs_cwd() {
        let inp = ActualFile::MultiFile(
            vec![FileRef::StaticFile(PathBuf::from("/abs/file/path")),
                 FileRef::StaticFile(PathBuf::from("rel/file/path"))
            ]);
        assert_eq!(inp.to_paths(&Some("/cwd/absloc")).unwrap(),
                   vec![PathBuf::from("/abs/file/path"),
                        PathBuf::from("/cwd/absloc/rel/file/path"),
                   ]);
    }

    #[test]
    fn test_paths_with_rel_cwd() {
        let inp = ActualFile::MultiFile(
            vec![FileRef::StaticFile(PathBuf::from("/abs/file/path")),
                 FileRef::StaticFile(PathBuf::from("rel/file/path"))
            ]);
        assert_eq!(inp.to_paths(&Some("cwd/relloc")).unwrap(),
                   vec![PathBuf::from("/abs/file/path"),
                        PathBuf::from("cwd/relloc/rel/file/path"),
                   ]);
    }

    #[test]
    fn test_globbed_matches_dry_run() -> () {
        let mut globfiles = vec![];
        let result = with_globbed_matches(
            &mut DryRun,
            &PathBuf::from("/rooted/path"),
            &String::from("*.rs"),
            |files| { globfiles = files.clone(); Ok(()) });
        assert!(result.is_ok());
        assert!(globfiles.is_empty(), "dry-run glob should be empty");
    }

    #[test]
    fn test_globbed_matches_normalrun() -> () {
        let mut globfiles = vec![];
        let result = with_globbed_matches(
            &mut NormalRun,
            &PathBuf::from("."),
            &String::from("C*.toml"),
            |files| { globfiles = files.clone(); Ok(()) });
        assert!(result.is_ok());
        // Assumes the cwd is the top-level directory for chainsop
        assert!(globfiles == vec![PathBuf::from("Cargo.toml")]);
    }
}
