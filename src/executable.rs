use std::ffi::{OsString};
use std::fmt;
use std::path::{PathBuf};

use crate::filehandling::ActualFile;


/// This is the core definition of an operation that will be run.  This can be
/// considered to be the template: a generic specification of describing the
/// target executable.  To actually run the defined operation in a specific
/// scenario, a `SubProcOperation` should be initialized from it and
/// customized with the needed operational parameters.
///
/// Note that the `FunctionOperation` defines an alternative to running an
/// executable application.

#[derive(Debug,Clone)]
pub struct Executable {
    pub exe_file : PathBuf,
    base_args : Vec<String>,
    inp_file : ExeFileSpec,
    out_file : ExeFileSpec,
}

// These get_xxx functions are accessors used _within_ this crate to access the
// non-public fields of the [Executable] struct.  These accessors are not
// intended to be exported outside of this crate.

pub fn get_base_args(exe: &Executable) -> &Vec<String> {
    &exe.base_args
}

pub fn get_inpfile(exe: &Executable) -> ExeFileSpec {
    exe.inp_file.clone()
}

pub fn get_outfile(exe: &Executable) -> ExeFileSpec {
    exe.out_file.clone()
}

/// Specifies the manner in which a file is provided to an Executable command.
/// Both input and output files are specified in this manner.  There is no
/// provision for handling stdin, stdout, and stderr.  It is assumed that an
/// executable consumes a file specified on the command line, and writes a file
/// that is also specified on the command line.
#[derive(Clone,Default)]
pub enum ExeFileSpec {
    /// No file provided or needed
    NoFileUsed,

    /// Append the file to the command string.  If both the input and the output
    /// file are specified in this manner, the input file is provided before the
    /// output file.
    #[default]
    Append,

    /// Specify the file using this option, which will be followed by the
    /// filename.  If the option ends in a '=' character, then the filename(s) is
    /// appended and it is presented as a single argument; otherwise the file is
    /// presented as the next argument.
    ///
    /// Examples:
    ///
    ///  * `Option("-f")` to specify "CMD -f FILE"
    ///
    ///  * `Option("-file=")` to specify "CMD --file=FILE"
    Option(String),

    /// The file is added to the arguments list by a special function.  The
    /// function specified here is called with the argument list and the named
    /// file; it should add the named file to the arguments list in some manner
    /// appropriate to the command.
    ///
    /// Note that at execution time, the current working directory may be
    /// different than the directory at the time this call is made, and the file
    /// specified will be relative to that working directory.  The working
    /// directory at execution time is provided to this function in case it needs
    /// to access the actual file location, but it should *not* include that
    /// directory specification in the argument added to the arguments list.
    ViaCall(fn(&mut Vec<OsString>,
               &Option<PathBuf>,
               &ActualFile) -> anyhow::Result<()>),
}

impl fmt::Debug for ExeFileSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ExeFileSpec::NoFileUsed => "<none>".fmt(f),
            ExeFileSpec::Append => "append".fmt(f),
            ExeFileSpec::Option(o) => format!("option({})", o).fmt(f),
            ExeFileSpec::ViaCall(_) => "via function call".fmt(f),
        }
    }
}

impl ExeFileSpec {

    /// Constructs the Option ExeFileSpec with automatic argument conversion
    pub fn option<'a, T: ?Sized>(optname : &'a T) -> ExeFileSpec
    where T: ToString
    {
        ExeFileSpec::Option(optname.to_string())
    }
}

impl Executable {

    /// Creates a new Executable to describe how to execute the corresponding
    /// process command and how that command is provided with input and output
    /// filenames.
    pub fn new<'a, T: ?Sized>(exe : &'a T,
                              inp_file : ExeFileSpec,
                              out_file : ExeFileSpec)
                              -> Executable
    where &'a T: Into<PathBuf>
    {
        Executable {
            exe_file : exe.into(),
            base_args : Vec::new(),
            inp_file : inp_file.clone(),
            out_file : out_file.clone(),
        }
    }

    /// Adds a command-line argument to use when executing the command.
    #[inline]
    pub fn push_arg<T>(&self, arg: T) -> Executable
    where T: Into<String>
    {
        Executable {
            base_args : { let mut tmp = self.base_args.clone();
                          tmp.push(arg.into());
                          tmp
            },
            ..self.clone()
        }
    }

    /// Specifies the name of the executable file
    #[inline]
    pub fn set_exe<T>(&self, exe: T) -> Executable
    where T: Into<PathBuf>
    {
        Executable {
            exe_file : exe.into(),
            ..self.clone()
        }
    }

}
