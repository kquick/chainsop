use std::path::{Path, PathBuf};

use crate::filehandling::defs::{ActualFile};
use crate::execution::OsRun;


/// Defines the interface for an Operation that can be performed (where an
/// operation is something like running an executable in a subprocess or calling
/// a local function to process a file).
pub trait OpInterface {

    /// Returns a short identifier of this operation, usually used for
    /// user-presented identification purposes.
    fn label(&self) -> String;

    /// Allows the label for an operation to be updated.  The label is usually
    /// used for user-presentation identification purposes.
    fn set_label(&mut self, new_label: &str) -> &mut Self;

    /// Executes this command in a subprocess in the specified directory.  The
    /// input and output files will be determined and added to the command-line
    /// as indicated by their `NamedFile` values.  The successful result specifies
    /// the output file written (if any).
    ///
    /// The target directory in which to execute the command(s) is specified by
    /// the input cwd parameter; if the directory for this operation has been
    /// explicitly overridden by calls to its `.set_dir()` method then those are
    /// assumed to be relative to the target directory specified here (unless
    /// they are absolute paths).  This is useful for setting a default
    /// directory, but allowing a particular operation to move to a "build"
    /// directory or some other place in the target tree.
    ///
    /// Because the operation may generate or consume additional files or make
    /// use of other directory-specific elements, the execution of the operation
    /// is performed within that directory.  All input and output files specified
    /// for the operation are the plain filename as-provided and are *not*
    /// combined with the specified cwd; if the cwd was a relative path,
    /// combining it with the filenames would result in doubling the relative
    /// specification and be invalid.  The filename as provided by the caller may
    /// be an absolute or a relative specification and that path will be
    /// propagated to the operation (any relative specification must be valid
    /// relative to the cwd specified for the operation).
    //
    // There is a design decision here, leading to the use of `mut self` for this
    // interface.  The origin of this need is that the chainedop needs to
    // identify the output file of one stage as the input file of the next stage
    // during the execution of the chain.  The implementation choices for this:
    //
    // 1. Use FilesPrep::set_input_file on successive elements of the chain to
    //    specify the NamedFile::Actual input for that element.
    //
    //
    // 2. Add additional `execute` entrypoints that will accept input arguments
    //    to override the input file(s) used for an element of the chain.
    //
    // The second approach was initially utilized, but it added significant
    // complications to the logic and exposed an additional entrypoint in the
    // OpInterface API which increased the surface area of support requirements.
    //
    // The primary disadvantage of the first approach is that it requires a
    // mutable self, and will also mutate the elements of the chain.  Examined in
    // context, the chain will probably already be declared as mutable so that
    // the chain elements can be added to it.  In addition, there is no direct
    // method for the caller to observe the results of the input file
    // modifications being performed, so while the caller might not expect `self`
    // to need to be mutable, all the mutations will be unobservable.  There is,
    // however, one additional restriction that must be placed on execution due
    // to this mutation choice:
    ///
    /// Note that execution of operations via the [OpInterface] may use an
    /// internal lock to ensure that only one execution of a specific operation
    /// (or operation chain) can be performed at a time (although different
    /// operation instances _can_ be performed in parallel).

    fn execute<Exec, P>(&mut self, executor: &mut Exec, cwd: &Option<P>)
                        -> anyhow::Result<ActualFile>
        where P: AsRef<Path>, Exec: OsRun;
}

/// Convenience routine to execute an operation with a given [crate::Executor] in
/// the current directory.
pub fn execute_here<Op, Exec>(op: &mut Op, executor: &mut Exec)
                              -> anyhow::Result<ActualFile>
where Exec: OsRun, Op: OpInterface
{
    op.execute(executor, &None::<PathBuf>)
}
