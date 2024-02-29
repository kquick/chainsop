use std::path::{Path,PathBuf};
use tempfile;


/// Designates a type of file that can be identified by name on the command line.
#[derive(Clone, Debug, PartialEq)]
pub enum FileArg {
    /// Actual file path (may or may not currently exist).
    Loc(PathBuf),

    /// Glob search in specified dir for all matching files.
    GlobIn(PathBuf, String),

    /// Create a temporary file; str is suffix to give temporary filename.
    Temp(String),

    /// Allowed on initial construction, but causes a runtime error on the call
    /// to execute an operation if it has not been converted to one of the other
    /// forms before the execute call.
    TBD
}

impl FileArg {
    /// Generates the designation indicating the need for a temporary file with
    /// the specified suffix.  If no particular suffix is needed, a blank suffix
    /// value should be specified.
    pub fn temp<T>(suffix: T) -> FileArg
    where T: Into<String>
    {
        FileArg::Temp(suffix.into())
    }

    /// Generates a reference to an actual file
    pub fn loc<T>(fpath: T) -> FileArg
    where T: Into<PathBuf>
    {
        FileArg::Loc(fpath.into())
    }

    /// Generates a reference to files identified by a file-globbing
    /// specification.
    pub fn glob_in<T,U>(dpath: T, glob: U) -> FileArg
    where T: Into<PathBuf>, U: Into<String>
    {
        FileArg::GlobIn(dpath.into(), glob.into())
    }
}

// ----------------------------------------------------------------------

/// This interface defines a standard set of operations that can be used to
/// prepare for an operation by specifying the input file, output file, and
/// directory in which the execution is to be performed.
pub trait FilesPrep {

    /// Sets the directory from which the operation will be performed.  The
    /// caller is responsible for ensuring any `FileArg::Loc` paths are
    /// valid when operating from that directory.  This directory is usually a
    /// relative directory and is interpreted from the current working directory
    /// or the directory provided to the `OpInterface::execute` method.
    fn set_dir<T>(&mut self, tgtdir: T) -> &mut Self
    where T: AsRef<Path>;

    /// Sets the input file for the operation, overriding any previous input file
    /// specification.
    fn set_input_file(&mut self, fname: &FileArg) -> &mut Self;

    /// Appends the additional input file to the list of input files for this
    /// operation.
    fn add_input_file(&mut self, fname: &FileArg) -> &mut Self;

    /// Returns true if one or more input files have been specified for this
    /// operation.
    fn has_input_file(&self) -> bool;

    /// Sets the output file for the command, overriding any previous output file
    /// specification.
    fn set_output_file(&mut self, fname: &FileArg) -> &mut Self;

    /// Returns true if the output file has been explicitly specified as a
    /// location (instead of being a TBD, a Glob match, or a Temp file).
    fn has_explicit_output_file(&self) -> bool;
}


#[derive(Clone)]
pub struct FileTransformation {
    pub inp_filenames : Vec<FileArg>,
    pub out_filename : FileArg,
    pub in_dir : Option<PathBuf>,
}

impl std::fmt::Debug for FileTransformation {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
    {
        format!("transforming {:?} into {:?} in {:?}",
                self.inp_filenames,
                self.out_filename,
                self.in_dir)
            .fmt(f)
    }
}

impl FileTransformation {
    pub fn new() -> FileTransformation {
        FileTransformation {
            inp_filenames : vec![],
            out_filename : FileArg::TBD,
            in_dir : None,
        }
    }
}

impl FilesPrep for FileTransformation {
    fn set_dir<T>(&mut self, tgtdir: T) -> &mut Self
    where T: AsRef<Path>
    {
        self.in_dir = Some(tgtdir.as_ref().to_path_buf());
        self
    }

    fn set_input_file(&mut self, fname: &FileArg) -> &mut Self
    {
        self.inp_filenames = vec![fname.clone()];
        self
    }

    fn add_input_file(&mut self, fname: &FileArg) -> &mut Self
    {
        self.inp_filenames.push(fname.clone());
        self
    }

    fn has_input_file(&self) -> bool
    {
        ! self.inp_filenames.is_empty()
    }

    fn set_output_file(&mut self, fname: &FileArg) -> &mut Self
    {
        self.out_filename = fname.clone();
        self
    }

    fn has_explicit_output_file(&self) -> bool
    {
        match self.out_filename {
            FileArg::Loc(_) => true,
            _ => false,
        }
    }

}

// ----------------------------------------------------------------------

/// The ActualFile is the actual file(s) to use for the input or output of a
/// SubProcOperation or FunctionOperation.  It is constructed from the FileArg
/// information provided to the overall operation and refers to 0 or more actual
/// (or intended actual) files (each of which is identified by the underlying
/// FileRef object contained in the ActualFile).
#[derive(Debug)]
pub enum ActualFile {
    NoActualFile,
    SingleFile(FileRef),
    MultiFile(Vec<FileRef>),
}

/// The FileRef is a reference to a single file, with possible resource
/// management scope and responsibilities.
#[derive(Debug)]
pub enum FileRef {
    StaticFile(PathBuf),

    /// References a temporary file, which will cease to exist when this value is
    /// garbage collected.
    TempFile(tempfile::NamedTempFile)
}
