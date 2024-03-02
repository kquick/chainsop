use anyhow::Context;
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::ffi::{OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::RwLock;

use crate::filehandling::*;
use crate::errors::*;
use crate::operations::generic::*;
use crate::operations::subproc::*;
use crate::operations::function::*;
use crate::execution::{OsRun};


/// Each entry in Chained operations can refer to either a sub-process operation
/// or a function operation; the RunnableOp is a wrapper to allow Chained to
/// homomorphically refer to these chained operations.  The impls for a
/// RunnableOp simply pass the method through to the corresponding method in the
/// underlying operation.
#[derive(Debug)]
enum RunnableOp {
    Exec(SubProcOperation),
    Call(FunctionOperation),
    // n.b. could be extended to allow nesting of ChainedOps as well if desired
}

macro_rules! runnable_passthru_call {
    ($me:ident, $method:ident with $($arg:ident),*) => {
        match $me {
            Self::Exec(sp) => sp.$method($($arg,)*),
            Self::Call(fp) => fp.$method($($arg,)*),
        }
    };
    (mutable $me:ident, $method:ident with $arg:ident) => {
        match *$me {
            Self::Exec(ref mut sp) => { sp.$method($arg); },
            Self::Call(ref mut fp) => { fp.$method($arg); },
        }
    }
}

macro_rules! runnable_op_passthru {
    ($method:ident, $argty:ty) => {
        fn $method(&mut self, arg: $argty) -> &mut Self
        {
            runnable_passthru_call!(mutable self, $method with arg);
            self
        }
    };
    ($method:ident returning $rty:ty) => {
        fn $method(&self) -> $rty
        {
            runnable_passthru_call!(self, $method with)
        }
    }
}

impl FilesPrep for RunnableOp {
    fn set_dir<T>(&mut self, tgtdir: T) -> &mut Self
    where T: AsRef<Path>
    {
        runnable_passthru_call!(mutable self, set_dir with tgtdir);
        self
    }
    runnable_op_passthru!(set_input_file, &FileArg);
    runnable_op_passthru!(add_input_file, &FileArg);
    runnable_op_passthru!(has_input_file returning bool);
    runnable_op_passthru!(set_output_file, &FileArg);
    runnable_op_passthru!(has_explicit_output_file returning bool);
}

impl OpInterface for RunnableOp {

    fn label(&self) -> String {
        runnable_passthru_call!(self, label with) // no args
    }

    fn set_label(&mut self, new_label: &str) -> &mut Self {
        runnable_passthru_call!(mutable self, set_label with new_label);
        self
    }

    fn execute<Exec, P>(&mut self, executor: &Exec, cwd: &Option<P>)
                        -> anyhow::Result<ActualFile>
    where P: AsRef<Path>, Exec: OsRun
    {
        runnable_passthru_call!(self, execute with executor, cwd)
    }
}

impl RunnableOp {
    fn push_arg<T>(&mut self, arg: T) -> &mut Self
    where T: Into<OsString>
    {
        match *self {
            Self::Exec(ref mut sp) => { sp.push_arg(arg); },
            Self::Call(_) => {
                // No args supported for FunctionOperation; just ignore this.
            },
        };
        self
    }
}

// ----------------------------------------------------------------------
/// Chained sub-process operations
///
/// General notes about structure organization:
///
///   The ChainedOps is the core structure that contains the list of
///   operations that should be chained together, along with the initial input
///   file and final output file.
///
///   When adding an operation to ChainedOps (via .push_op()) the
///   return value should allow subsequent examination/manipulation of that
///   specific operation in the chain (the ChainedOpRef struct).
//    To support this access and honor Rust's ownership rules, this means that
//   the result references the underlying ChainedOpsInternals via a
//   reference counted (Rc) cell (RefCell) to maintain a single copy via the Rc
//   but allow updates of that object via the RefCell.
//
//   To hide the complexity of the Rc<RefCell<ChainedOpsInternals>> from the
//   user, this value is wrapped internally in the ChainedOps struct.
//
//   User API operations are therefore primarily defined for the ChainedOps
//   and ChainedOpRef structs.
///
///   The typical API usage:
///
///    let all_ops = ChainedOps::new()
///    let op1 = all_ops.push_op(
///               SubProcOperation::new("command",
///                                     ExeFileSpec::...,
///                                     ExeFileSpec::...))
///    let op2 = all_ops.push_op(
///               SubProcOperation::new("next-command",
///                                     ExeFileSpec::...,
///                                     ExeFileSpec::...))
///    ...
///    op1.push_arg("-x")
///    op2.push_arg("-f")
///    op2.push_arg(filename)
///    op2.disable()
///    ...
///    all_ops.set_input_file(input_filename)
///    all_ops.set_output_file(output_filename)
///    let mut executor = Executor::NormalRun;
///    all_ops.execute_here(&mut executor)?;
///
pub struct ChainedOps {

    // chops is a smart pointer to a RefCell allowing borrow or borrow_mut
    // accesses to the main structure.  This is done so that the ChainedOpRef
    // (which references a single element in the chain) can reference and modify
    // the same data (constrained to that element).  Note that it is possible to
    // nest .borrow() calls, but not .borrow_mut(), and this is a *runtime*
    // error, so careful use of chops is needed since Rust's borrow checking
    // cannot validate the code.
    chops : Rc<RefCell<ChainedOpsInternals>>,

    // execlock is a lock that is used to ensure that this ChainedOps is *not*
    // being executed in parallel because that would cause conflicting
    // modifications to the chain elements (input files).  We do not use a RwLock
    // for chops, because this should be shared with ChainedOpRef and there is
    // more expense associated with the lock.  Since the locking is only needed
    // around the main .execute() method call, it is easy to acquire and release
    // this as a distinct entity.  The value held by the lock is unimportant:
    // here we will use it as a counter of the number of .execute() calls.
    chlock : RwLock<u32>,
}


/// Internal structure managing the chain of operations
#[derive(Debug)]
struct ChainedOpsInternals {
    name : String,

    // The chain of operations to execute.
    chain : Vec<RunnableOp>,

    // The input and output files, and directory for the entire chain.
    files : FileTransformation,

    // The activation state of entries in the chain (hash key == chain index).
    // If there is no hash entry for a specific chain entry, then that entry is
    // Active by default.
    opstate : HashMap<usize, Activation>,

    // Identifies which chain operations have preset input files (and thus their
    // input should *not* be set to the output of the previous operation during
    // execution).
    preset_inputs : Vec<usize>,
}


/// This is returned when a RunnableOp is added to the ChainedOps/ChainedIntOps
/// and serves as a proxy for the RunnableOp as it exists in the chain.  This
/// supports additional customization actions on the contained RunnableOp via the
/// FilesPrep trait.
#[derive(Clone,Debug)]
pub struct ChainedOpRef {
    opidx : usize,
    chop : Rc<RefCell<ChainedOpsInternals>>  // cloned from ChainedOps.chops
}


impl fmt::Debug for ChainedOps {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.chops.borrow(), f)
    }
}

// TODO: Iterator impl on ChainedOps returning ChainedOpRef by: usize? label?


impl ChainedOps {
    // The result is Rc'd so that the ChainedOpRef instances can have a
    // reference to the target as well.
    pub fn new<T: Into<String> + Clone>(label: T) -> ChainedOps
    {
        ChainedOps {
            chops :
            Rc::new(
                RefCell::new(
                    ChainedOpsInternals { name: label.clone().into(),
                                          chain : Vec::new(),
                                          files : FileTransformation::new(),
                                          opstate : HashMap::new(),
                                          preset_inputs : Vec::new(),

                    }
                )
            ),
            chlock : RwLock::new(0),
        }
    }

    /// Adds a new SubProcOperation operation to the end of the chain.  Returns a
    /// reference for modifying that operation.
    ///
    /// When the chain is executed, each operation in the chain will be executed
    /// in a sequence, stopping if any operation returns an error.  The output of
    /// each operation is set as the input to the next operation *unless* the
    /// input for that next operation has already been manually set (via the
    /// [FilesPrep::set_input_file] trait method).
    pub fn push_op(self: &ChainedOps, op: &SubProcOperation) -> ChainedOpRef
    {
        let opidx = {
            let mut ops: RefMut<_> = self.chops.borrow_mut();
            ops.chain.push(RunnableOp::Exec(op.clone()));
            let opidx = ops.chain.len() - 1;
            if op.has_input_file() {
                ops.preset_inputs.push(opidx);
            }
            opidx
        };
        ChainedOpRef { opidx,
                       chop : Rc::clone(&self.chops)
        }
    }

    /// Adds a new FunctionOperation operation to the end of the chain.  Returns
    /// a reference for modifying that operation.
    pub fn push_call(self: &ChainedOps, op: &FunctionOperation) -> ChainedOpRef
    {
        let opidx = {
            let mut ops: RefMut<_> = self.chops.borrow_mut();
            ops.chain.push(RunnableOp::Call(op.clone()));
            let opidx = ops.chain.len() - 1;
            if op.has_input_file() {
                ops.preset_inputs.push(opidx);
            }
            opidx
        };
        ChainedOpRef { opidx,
                       chop : Rc::clone(&self.chops)
        }
    }
}

impl FilesPrep for ChainedOps
{
    /// Sets the current directory for the entire chain.  Individual operations
    /// can specify a relative directory that will be make relative to this
    /// location, or they can specify an absolute directory that will override
    /// this location.  If this call is not made, the operations are executed in
    /// the current process directory.
    #[inline]
    fn set_dir<T: AsRef<Path>>(&mut self, tgtdir: T) -> &mut Self
    {
        {
            let mut ops: RefMut<_> = self.chops.borrow_mut();
            ops.files.set_dir(tgtdir);
        }
        self
    }

    /// Sets the input file(s) for the entire chain
    #[inline]
    fn set_input_file(&mut self, fname: &FileArg) -> &mut Self
    {
        {
            let mut ops: RefMut<_> = self.chops.borrow_mut();
            ops.files.set_input_file(fname);
        }
        self
    }

    /// Adds an input file(s) to the entire chain
    #[inline]
    fn add_input_file(&mut self, fname: &FileArg) -> &mut Self
    {
        {
            let mut ops: RefMut<_> = self.chops.borrow_mut();
            ops.files.add_input_file(fname);
        }
        self
    }

    fn has_input_file(&self) -> bool
    {
        self.chops.borrow().files.has_input_file()
    }

    #[inline]
    fn set_output_file(&mut self, fname: &FileArg) -> &mut Self
    {
        {
            let mut ops: RefMut<_> = self.chops.borrow_mut();
            ops.files.set_output_file(fname);
        }
        self
    }

    fn has_explicit_output_file(&self) -> bool
    {
        self.chops.borrow().files.has_explicit_output_file()
    }

}

impl OpInterface for ChainedOps
{
    fn label(&self) -> String
    {
        self.chops.borrow().name.clone()
    }


    fn set_label(&mut self, new_label: &str) -> &mut Self {
        self.chops.borrow_mut().name = new_label.to_string();
        self
    }

    /// Executes all the enabled operations in this chain sequentially, updating
    /// the input file of each operation to be the output file from the previous
    /// operation.  On success, returns the number of operations executed.
    ///
    /// The directory parameter specifies the default directory from which the
    /// chained operations will be performed.  Each chained operation might
    /// operate from a separate directory if the SubProcOperation::set_dir() or
    /// ChainedOpRef::set_dir() function has been called for this operation,
    /// which overrides the default directory passed to this command.
    fn execute<Exec, P>(&mut self, executor: &Exec, cwd: &Option<P>)
                        -> anyhow::Result<ActualFile>
    where P: AsRef<Path>, Exec: OsRun
    {
        // Lock this chain to ensure it is not run in parallel, which would
        // create conflicts with the internal chain element input file updates.
        // The lock is automatically released at the end of this method scope.
        let mut locked = self.chlock.write().unwrap();
        *locked += 1;

        let mut chops = self.chops.borrow_mut();

        // Some chain elements might be marked as disabled.  Rather than
        // requiring a test of each chain element each time it is to be
        // considered, we instead build a vec of the enabled element indices.
        // Build it in reverse so the operations can simply .pop() the next index
        // off the end.
        let mut enabled_opidxs : Vec<usize> = chops.chain.iter()
            .enumerate()
            .filter(|(i,_op)|
                    chops.opstate.get(i).unwrap_or(&Activation::Enabled) == &Activation::Enabled)
            .map(|(i,_op)| i)
            .rev()
            .collect();

        if enabled_opidxs.is_empty() {
            // This is a non-functional chain: it is either empty or every
            // operation in the chain is disabled.  No output file was generated.
            return Ok(ActualFile::NoActualFile);
        }

        let first_op = enabled_opidxs[enabled_opidxs.len()-1];
        let last_op = enabled_opidxs[0];
        let chain_inps = chops.files.inp_filenames.clone();

        if ! chain_inps.is_empty() {
            chops.chain[first_op].set_input_file(&chain_inps[0]);
            for f in &chain_inps[1..] {
                chops.chain[first_op].add_input_file(f);
            }
        }
        let tgtdir = match cwd {
            None => chops.files.in_dir.clone(),
            Some(d) =>
                match &chops.files.in_dir {
                    Some(od) => Some(d.as_ref().join(od)),
                    None => Some(d.as_ref().into()),
            }
        };
        if chops.files.has_explicit_output_file() {
            let main_out_file = chops.files.out_filename.clone();
            chops.chain[last_op].set_output_file(&main_out_file);
        }

        let pinp = chops.preset_inputs.clone();
        execute_chain(executor, &mut chops.chain, &pinp, &tgtdir,
                      &mut enabled_opidxs)
    }
}

fn execute_chain(executor: &impl OsRun,
                 chops: &mut Vec<RunnableOp>,
                 preset_inputs: &Vec<usize>,
                 cwd: &Option<PathBuf>,
                 mut op_idxs: &mut Vec<usize>) -> anyhow::Result<ActualFile>
{
    let op_idx = op_idxs.pop().unwrap();
    let spo = &mut chops[op_idx];
    let outfile = spo.execute(executor, cwd)?;
    if op_idxs.is_empty() {
        // This was the last operation, execution of the chain is completed.
        return Ok(outfile);
    }
    match outfile.to_paths::<PathBuf>(&None).with_context(
        || format!("Output file for chained operation {}", spo.label()))
    {
        Ok(mut ps) => {
            // If no output files, just let next chained element's input be what
            // it was originally set to.
            if !ps.is_empty() {
                // Otherwise, set the inputs of the next operation to the output
                // of the just-completed operation (unless the inputs are already
                // pre-set).
                //
                // n.b. OK to unwrap here for last_idx: size matches ps and
                // therefore cannot be empty
                let last_idx = op_idxs.last().unwrap();
                if ! preset_inputs.contains(last_idx) {
                    chops[*last_idx].set_input_file(&FileArg::Loc(ps.pop().unwrap()));
                    for p in ps {
                        chops[*last_idx].add_input_file(&FileArg::Loc(p.clone()));
                    }
                }
            }
        }
        Err(e) => match &e.root_cause().downcast_ref::<ChainsopError>() {
            Some(ChainsopError::ErrorMissingFile) => {
                // This is OK here because the following operation may be setup
                // for not needing an input file specification; if it does need
                // one then presumably some subsequent runtime check will
                // validate its (lack of) existence and signal a useful failure.
            }
            _ => { return Err(e); }
        },
    };
    execute_chain(executor, chops, preset_inputs, cwd, &mut op_idxs)
}

/// This enumerates the possible active conditions for each operation in the
/// chain.  This is used as the argumement to the [ChainedOpRef::active] method
/// to determine how the associated operation should be treated during execution
/// of the chain.
#[derive(Clone, Debug, PartialEq)]
pub enum Activation {
    /// The associated operation is performed during execution of the chain.
    Enabled,
    /// The associated operation is skipped (not performed) during execution of
    /// the chain.
    Disabled,
}

impl ChainedOpRef {

    /// Add an argument to this operation in the chain
    #[inline]
    pub fn push_arg<T>(&mut self, arg: T) -> &mut ChainedOpRef
    where T: Into<OsString>
    {
        {
            let mut ops: RefMut<_> = self.chop.borrow_mut();
            ops.chain[self.opidx].push_arg(arg);
        }
        self
    }

    /// Sets the "active" status of this operation in the chain.  An individual
    /// operation in the chain can be skipped or executed normally based on the
    /// [Activation] value set by this method.  When initially added to the
    /// chain, all operations are set to [Activation::Enabled] by default.
    #[inline]
    pub fn active(&mut self, state: &Activation) -> &mut ChainedOpRef
    {
        {
            let mut ops: RefMut<_> = self.chop.borrow_mut();
            match state {
                Activation::Enabled => ops.opstate.remove(&self.opidx),
                Activation::Disabled => ops.opstate.insert(self.opidx, state.clone()),
            };
        }
        self
    }

}


impl FilesPrep for ChainedOpRef {

    /// Sets the directory in which this operation is performed.  If this is a
    /// relative directory, it will be relative to the directory of the entire
    /// ChainedOps.  If not set, the ChainedOps directory is used, and if set to
    /// an absolute directory, it will override the ChainedOps directory setting.
    fn set_dir<T>(&mut self, tgtdir: T) -> &mut ChainedOpRef
    where T: AsRef<Path>
    {
        {
            let mut ops: RefMut<_> = self.chop.borrow_mut();
            ops.chain[self.opidx].set_dir(tgtdir);
        }
        self
    }

    /// Sets the input file for this operation in the chain.  This is normally
    /// not used for chained operations, since the input file will default to the
    /// output file of the previous operation in the chain (and the chain's input
    /// file will become the input file for the first operation in the chain).
    ///
    /// * If this is the first operation in the chain, this will be ignored
    ///   unless the chain itself has no input file specified.
    ///
    /// * If this is not the first operation in the chain, this will override the
    ///   default chain behaviour of setting the input file to the output of the
    ///   previous element.
    fn set_input_file(&mut self, inp_fname : &FileArg) -> &mut ChainedOpRef
    {
        {
            let mut ops: RefMut<_> = self.chop.borrow_mut();
            ops.chain[self.opidx].set_input_file(inp_fname);
            if ! ops.preset_inputs.contains(&self.opidx) {
                ops.preset_inputs.push(self.opidx);
            }
        }
        self
    }

    /// Appends an additional input file for this operation.  Has the same
    /// behavior characteristics as the [ChainedOpRef::set_input_file] method.
    fn add_input_file(&mut self, inp_fname : &FileArg) -> &mut ChainedOpRef
    {
        {
            let mut ops: RefMut<_> = self.chop.borrow_mut();
            ops.chain[self.opidx].add_input_file(inp_fname);
            if ! ops.preset_inputs.contains(&self.opidx) {
                ops.preset_inputs.push(self.opidx);
            }
        }
        self
    }

    fn has_input_file(&self) -> bool
    {
        self.chop.borrow().chain[self.opidx].has_input_file()
    }

    /// Specifies the output file for this operation in the chain. This will also
    /// inform the input file setting for the subsequent operation in the chain
    /// unless that operation has an explicit input file setting.
    ///
    /// * If this is the last element of the chain, this is ignored if an output
    ///   file for the chain itself is set.
    fn set_output_file(&mut self, out_fname : &FileArg) -> &mut ChainedOpRef
    {
        {
            let mut ops: RefMut<_> = self.chop.borrow_mut();
            ops.chain[self.opidx].set_output_file(out_fname);
        }
        self
    }

    fn has_explicit_output_file(&self) -> bool
    {
        self.chop.borrow().chain[self.opidx].has_explicit_output_file()
    }

}


// ----------------------------------------------------------------------
// TESTS
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {

    // Tested functionality (summarized here for overview and completeness
    // determination, then marked throughout where verified by a test):
    //
    // * [TC1] empty chain
    // * [TC2] chain of one element
    // * [TC3] multi-element chain (both SubProcOperation and FunctionOperation),
    //         including setting input file of one op to the output file of the
    //         previous op
    // * [TC4] multi-element chain skips disabled individual ops
    // * [TC5] multi-element chain does not override a chain op's explicit
    //         input file
    // * [TC6] Set chain output file *after* final operation output file
    // * [TC7] Set chain output file *before* final operation output file
    // * [TC8] Set chain input file *after* final operation output file
    // * [TC9] Set chain input file *before* final operation output file
    // * [TC10] first and last op set input and output, no chain-level spec.
    // * [TC11] Default chain directory is current directory
    // * [TC12] Chain operation without directory inherits chain directory
    // * [TC13] Chain operation relative directory is relative to chain directory
    // * [TC14] Chain operation absolute directory ignores chain directory
    // * [TC15] Chain specification can be executed multiple times
    // * [TC16] Verify Temp op output file
    // * [TC17] Verify function ViaCall (function-specified) op output file
    // * [TC18] Verify closure ViaCall (function-specified) op output file
    // * [TC19] Verify Loc (explicitly specified) op output file
    // * [TC20] Verify chained ops set_dir does not affect filenames
    // * [TC21] Absolute chain directory overrides execute directory
    // * [TC22] Absolute chain directory combines with relative op directory

    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use crate::executable::*;
    use crate::execution::*;

    #[derive(Clone, Debug, PartialEq)]
    struct RunExec {
        name: String,
        exe: PathBuf,
        args: Vec<OsString>,
        dir: Option<PathBuf>
    }
    #[derive(Debug, PartialEq)]
    struct RunFunc{
        fname: String,
        inpfiles: Vec<PathBuf>,
        outfile: Option<PathBuf>,
        dir: Option<PathBuf>
    }
    #[derive(Debug, PartialEq)]
    enum TestOp {
        SPO(RunExec),
        FO(RunFunc)
    }
    struct TestCollector(RefCell<Vec<TestOp>>);
    impl TestCollector {
        pub fn new() -> TestCollector {
            TestCollector(RefCell::new(vec![]))
        }
    }

    impl OsRun for TestCollector {
        fn run_executable(&self,
                          label: &str,
                          exe_file: &Path,
                          args: &Vec<OsString>,
                          fromdir: &Option<PathBuf>) -> OsRunResult
        {
            self.0.borrow_mut()
                .push(TestOp::SPO(RunExec{ name: String::from(label),
                                           exe: PathBuf::from(exe_file),
                                           args: args.clone(),
                                           dir: fromdir.clone()
            }));
            OsRunResult::Good
        }
        fn run_function(&self,
                        name : &str,
                        _call : &Rc<dyn Fn(&Path, &ActualFile, &ActualFile) -> anyhow::Result<()>>,
                        inpfiles: &ActualFile,
                        outfile: &ActualFile,
                        fromdir: &Option<PathBuf>) -> OsRunResult
        {
            self.0.borrow_mut()
                .push(TestOp::FO(RunFunc{ fname: name.to_string(),
                                          inpfiles: inpfiles.to_paths::<PathBuf>(&None).unwrap(),
                                          outfile: outfile.to_path::<PathBuf>(&None).ok(),
                                          dir: fromdir.clone()
            }));
            OsRunResult::Good
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

    fn test_callee(_indir: &Path,
                   _inpfiles: &ActualFile,
                   _outfile: &ActualFile) -> anyhow::Result<()>
    {
        // n.b. the FunctionalOp entries in the chains below are disabled, so
        // this is not actually needed.  If it is, a more complete body should be
        // supplied.
        todo!("test_callee not implemented")
    }

    fn add_finish_outarg(args: &mut Vec<OsString>,
                         _cwd: &Option<PathBuf>,
                         outf: &ActualFile)
                         -> anyhow::Result<()>
    {
        for pth in outf.to_paths::<PathBuf>(&None)? {
            args.push(format!("out:{:?}", pth).into());
        }
        Ok(())
    }

    #[test]
    // n.b. this pretty much tests all the things as far as chainedops is
    // concerned; more focused tests to identify individual failure modes may be
    // helpful when tracking down issues.
    fn test_chain() -> anyhow::Result<()> {
        let mut ops = ChainedOps::new("test chain");
        ops.set_input_file(&FileArg::loc("orig.inp"));   // [TC9]
        ops.set_output_file(&FileArg::loc("final.out"));  // [TC7]

        let exe = Executable::new(&"test-cmd",
                                  ExeFileSpec::Append,
                                  ExeFileSpec::Append);
        let op = SubProcOperation::new(&exe)
            // set the input file, but the chain will override this
            .set_input_file(&FileArg::loc("inpfile.txt"))  // [TC9]
            .set_output_file(&FileArg::temp(".out")) // [TC16]
            .push_arg("-a")
            .push_arg("a-arg-value")
            .push_arg("-b")
            .clone();

        ops.push_op(&op);  // [TC12]
        ops.add_input_file(&FileArg::loc("other_inpfile")); // [TC9]

        let mut op2 = ops.push_op(SubProcOperation::new(
            &Executable::new(&"cmd2",
                             ExeFileSpec::Option("--input".into()),
                             ExeFileSpec::Append))
                              .set_output_file(&FileArg::temp(".o2")) // [TC16]
                              .push_arg("-D")
                              .push_arg("DVAL")
        );
        op2.push_arg("-a2");
        op2.set_dir(".build");

        let mut op3 = ops.push_op(&op);
        op3.active(&Activation::Disabled);
        op3.active(&Activation::Enabled).active(&Activation::Disabled); // [TC4]

        ops.push_call(FunctionOperation::calling("a func", test_callee)
                      .set_input_file(&FileArg::loc("not replaced")) // [TC5]
                      .set_output_file(&FileArg::temp(".fnout"))); // [TC16]

        // Verify ViaCall can take a closure
        ops.push_op(SubProcOperation::new(
            &Executable::new(&"finish",
                             ExeFileSpec::option("-i"),
                             ExeFileSpec::ViaCall(|args, cwd, outf| { // [TC18]
                                 args.push(format!("+({:?}){:?}", cwd, outf).into());
                                 Ok(())
                             })))
                    .set_output_file(&FileArg::TBD)
                    .push_arg("--last")
                    .set_dir(".other")
        ).active(&Activation::Disabled); // [TC4]

        let mut op5 = ops.push_op(SubProcOperation::new(
            &Executable::new(&"finish",
                             ExeFileSpec::option("-i"),
                             ExeFileSpec::ViaCall(add_finish_outarg))) // [TC17]
                                  // The output file setting here will be ignored
                                  // because the output file for the chain itself
                                  // is specified.
                              .set_output_file(&FileArg::loc("ignored")) // [TC7]
                              .push_arg("--last")
                              .set_dir(".other")
        );
        op5.push_arg("op").set_dir(".build");  // [TC13]

        let mut xor = TestCollector::new();
        let result = ops.execute(&mut xor, &Some("target/loc"));
        assert!(match result {
            Ok(ActualFile::SingleFile(FileRef::StaticFile(sf))) =>
                sf == PathBuf::from("final.out"),
            _ => false,
        });
        let mut collected = xor.0.into_inner();
        assert_eq!(collected.len(), 4);

        // The last arg of the first op is an assigned output tempfile
        let output0_tmpfile = match &mut collected[0] {
            TestOp::SPO(re) => {
                let outf = PathBuf::from(&re.args.last().unwrap());
                assert!(!outf.exists(), // [TC16]
                        "intermediate temp file #0 {:?} did not get cleaned up!",
                        outf);
                assert_eq!(outf.extension(), Some(OsString::from("out")).as_deref()); // [TC16]
                re.args.pop();
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected first op to be a SubProcOperation");
            }
        };

        // The penultimate arg of the second op should be the input tempfile,
        // which should be the output tempfile of the previous op
        match &mut collected[1] {
            TestOp::SPO(re) => {
                let inpf = PathBuf::from(&re.args[re.args.len()-2]);
                assert_eq!(inpf, output0_tmpfile);
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };
        // The last arg of the second op is another assigned output tempfile
        let output1_tmpfile = match &mut collected[1] {
            TestOp::SPO(re) => {
                let outf = PathBuf::from(&re.args.last().unwrap());
                assert!(!outf.exists(), // [TC16]
                        "intermediate temp file #1 {:?} did not get cleaned up!",
                        outf);
                assert_eq!(outf.extension(), Some(OsString::from("o2")).as_deref()); // [TC16]
                re.args.pop();  // remove output file
                re.args.pop();  // remove input file
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };

        // The input file of the third op should be the tempfile output of the
        // second op, and the output file of the third op should be an assigned
        // tempfile.
        let output2_tmpfile = match &mut collected[2] {
            TestOp::SPO(_) => {
                panic!("Expected third op to be a FunctionOperation");
            }
            TestOp::FO(rf) => {
                // There was an explicit input file set for this operation, so it
                // should *not* have been overwritten by the output of the
                // previous operation.
                assert_ne!(rf.inpfiles, vec![output1_tmpfile.clone()]);
                assert_eq!(rf.inpfiles, vec![PathBuf::from("not replaced")]); // [TC5]
                let outf = rf.outfile.clone().unwrap();
                assert!(!outf.exists(), // [TC16]
                        "intermediate temp file #2 {:?} did not get cleaned up!",
                        outf);
                assert_eq!(outf.extension(), Some(OsString::from("fnout")).as_deref()); // [TC16]
                rf.inpfiles = vec![];
                rf.outfile = None;
                outf
            }
        };

        // The penultimate arg of the third op should be the input tempfile,
        // which should be the output tempfile of the previous op
        match &mut collected[3] {
            TestOp::SPO(re) => {
                let inpf = PathBuf::from(&re.args[re.args.len()-2]);
                assert_eq!(inpf, output2_tmpfile);
                let outf = re.args.clone().last().unwrap().clone();
                re.args.pop();  // remove output filespec
                re.args.pop();  // remove input file
                re.args.push(outf.clone()); // re-add output filespec
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };

        assert_eq!(collected,
                   vec![ TestOp::SPO(RunExec { name: "test-cmd".into(),
                                               exe: "test-cmd".into(),
                                               args: ["-a",
                                                      "a-arg-value",
                                                      "-b",
                                                      "orig.inp", // [TC9]
                                                      "other_inpfile", // [TC9]
                                                      // output file removed above
                                               ].map(Into::<OsString>::into).to_vec(),
                                               dir: Some(PathBuf::from("target/loc"))}), // [TC12]
                         TestOp::SPO(RunExec { name: "cmd2".into(),
                                               exe: "cmd2".into(),
                                               args: ["-D",
                                                      "DVAL",
                                                      "-a2",
                                                      "--input",
                                                      // input file removed above
                                                      // output file removed above
                                               ].map(Into::<OsString>::into).to_vec(),
                                               dir: Some(PathBuf::from("target/loc/.build"))}),
                         // [TC4]
                         TestOp::FO(RunFunc { fname: "a func".into(),
                                              inpfiles: vec![], // input files removed above
                                              outfile: None, // output file removed above
                                              dir: Some(PathBuf::from("target/loc"))}),
                         // [TC4]
                         TestOp::SPO(RunExec { name: "finish".into(),
                                               exe: "finish".into(),
                                               args: ["--last",
                                                      "op",
                                                      "-i",
                                                      // input file removed above
                                                      "out:\"final.out\"", // [TC7], [TC17]
                                               ].map(Into::<OsString>::into).to_vec(),
                                               dir: Some(PathBuf::from("target/loc/.build"))}),
                   ]);

        // -------------------------------------------------------------------
        // Execute the chainedops *again* to verify they can be re-used and are
        // actually executed again.
        let mut xor2 = TestCollector::new();
        let result2 = ops.execute(&mut xor2, &Some("/other"));  // [TC15]

        assert!(match result2 {
            Ok(ActualFile::SingleFile(FileRef::StaticFile(sf))) =>
                sf == PathBuf::from("final.out"),
            _ => false,
        });
        let mut collected2 = xor2.0.into_inner();
        assert_eq!(collected2.len(), 4);

        // The last arg of the first op is an assigned output tempfile
        let output0_tmpfile2 = match &mut collected2[0] {
            TestOp::SPO(re) => {
                let outf = PathBuf::from(&re.args.last().unwrap());
                assert!(!outf.exists(),
                        "intermediate second temp file #0 {:?} did not get cleaned up!",
                        outf);
                re.args.pop();
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected first op to be a SubProcOperation");
            }
        };
        assert_ne!(output0_tmpfile, output0_tmpfile2);  // each run uses new tempfiles

        // The penultimate arg of the second op should be the input tempfile,
        // which should be the output tempfile of the previous op
        match &mut collected2[1] {
            TestOp::SPO(re) => {
                let inpf = PathBuf::from(&re.args[re.args.len()-2]);
                assert_eq!(inpf, output0_tmpfile2);
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };
        // The last arg of the second op is another assigned output tempfile
        let output1_tmpfile2 = match &mut collected2[1] {
            TestOp::SPO(re) => {
                let outf = PathBuf::from(&re.args.last().unwrap());
                assert!(!outf.exists(),
                        "intermediate second temp file #1 {:?} did not get cleaned up!",
                        outf);
                re.args.pop();  // remove output file
                re.args.pop();  // remove input file
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };
        assert_ne!(output1_tmpfile, output1_tmpfile2);  // each run uses new tempfiles

        // The input file of the third op should be the tempfile output of the
        // second op, and the output file of the third op should be an assigned
        // tempfile.
        let output2_tmpfile2 = match &mut collected2[2] {
            TestOp::SPO(_) => {
                panic!("Expected third op to be a FunctionOperation");
            }
            TestOp::FO(rf) => {
                // There was an explicit input file set for this operation, so it
                // should *not* have been overwritten by the output of the
                // previous operation.
                assert_ne!(rf.inpfiles, vec![output1_tmpfile.clone()]);
                assert_eq!(rf.inpfiles, vec![PathBuf::from("not replaced")]);
                let outf = rf.outfile.clone().unwrap();
                assert!(!outf.exists(),
                        "intermediate second temp file #2 {:?} did not get cleaned up!",
                        outf);
                rf.inpfiles = vec![];
                rf.outfile = None;
                outf
            }
        };
        assert_ne!(output2_tmpfile, output2_tmpfile2);  // each run uses new tempfiles

        // The penultimate arg of the third op should be the input tempfile,
        // which should be the output tempfile of the previous op
        match &mut collected2[3] {
            TestOp::SPO(re) => {
                let inpf = PathBuf::from(&re.args[re.args.len()-2]);
                assert_eq!(inpf, output2_tmpfile2);
                let outf = re.args.clone().last().unwrap().clone();
                re.args.pop();  // remove output filespec
                re.args.pop();  // remove input file
                re.args.push(outf.clone()); // re-add output filespec
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };

        assert_eq!(collected2,  // almost `collected` but main directory changed
                   vec![ TestOp::SPO(RunExec { name: "test-cmd".into(),
                                               exe: "test-cmd".into(),
                                               args: ["-a",
                                                      "a-arg-value",
                                                      "-b",
                                                      "orig.inp",
                                                      "other_inpfile",
                                                      // output file removed above
                                               ].map(Into::<OsString>::into).to_vec(),
                                               dir: Some(PathBuf::from("/other"))}),
                         TestOp::SPO(RunExec { name: "cmd2".into(),
                                               exe: "cmd2".into(),
                                               args: ["-D",
                                                      "DVAL",
                                                      "-a2",
                                                      "--input",
                                                      // input file removed above
                                                      // output file removed above
                                               ].map(Into::<OsString>::into).to_vec(),
                                               dir: Some(PathBuf::from("/other/.build"))}),
                         TestOp::FO(RunFunc { fname: "a func".into(),
                                              inpfiles: vec![], // input files removed above
                                              outfile: None, // output file removed above
                                              dir: Some(PathBuf::from("/other"))}),
                         TestOp::SPO(RunExec { name: "finish".into(),
                                               exe: "finish".into(),
                                               args: ["--last",
                                                      "op",
                                                      "-i",
                                                      // input file removed above
                                                      "out:\"final.out\"",
                                               ].map(Into::<OsString>::into).to_vec(),
                                               dir: Some(PathBuf::from("/other/.build"))}),
                   ]);
        Ok(())
    }

    #[test]
    fn test_chain_empty() -> anyhow::Result<()> {
        let mut ops = ChainedOps::new("test empty chain");
        // [TC1]
        let mut ex = TestCollector::new();
        let result = ops.execute(&mut ex, &Some("target/loc"));
        match result {
            Ok(ActualFile::NoActualFile) => (),
            _ => assert!(false,
                         "Expected single static file 'final.out' but got {:?}",
                         result),
        };
        let collected = ex.0.into_inner();
        assert_eq!(collected.len(), 0);
        Ok(())
    }

    #[test]
    fn test_chain_single_op() -> anyhow::Result<()> {
        let mut ops = ChainedOps::new("test chain single");

        let exe = Executable::new(&"test-cmd",
                                  ExeFileSpec::Append,
                                  ExeFileSpec::Append);
        ops.push_op(SubProcOperation::new(&exe)  // [TC2]
                    // Set the input file.  The chain should *not* override this
                    // because the chain itself is not given an input file.  The
                    // usual expectation is that the chain *will* be given an
                    // input file, but this verifies the behavior of the unusual
                    // scenario.
                    .set_input_file(&FileArg::loc("override-in"))
                    .set_output_file(&FileArg::loc("override-out"))
                    .push_arg("-b"));
        ops.set_input_file(&FileArg::loc("real-in"));  // [TC8]
        ops.set_output_file(&FileArg::loc("real-out")); // [TC6]

        let mut ex = TestCollector::new();
        let result = execute_here(&mut ops, &mut ex);
        match result {
            Ok(ActualFile::SingleFile(FileRef::StaticFile(sf))) =>
                assert_eq!(sf, PathBuf::from("real-out")), // [TC6]
            _ => assert!(false,
                         "Expected single static file 'final.out' but got {:?}",
                         result),
        };
        let collected = ex.0.into_inner();
        assert_eq!(collected.len(), 1); // [TC2]

        assert_eq!(collected,
                   vec![ TestOp::SPO(RunExec { name: "test-cmd".into(),
                                               exe: "test-cmd".into(),
                                               args: ["-b",
                                                      "real-in", // [TC8]
                                                      "real-out", // [TC6]
                                                      // output file removed above
                                               ].map(Into::<OsString>::into).to_vec(),
                                               dir: None}), // [TC11]
                   ]);
        Ok(())
    }


    #[test]
    fn test_chain_op_settings() -> anyhow::Result<()> {
        let mut ops = ChainedOps::new("test chain");
        ops.set_dir("/ops/run/here");  // [TC20], [TC14]
            // [TC10]
        let exe = Executable::new(&"test-cmd",
                                  ExeFileSpec::Append,
                                  ExeFileSpec::Append);
        ops.push_op(SubProcOperation::new(&exe)
                    // Set the input file.  The chain should *not* override this
                    // because the chain itself is not given an input file.  The
                    // usual expectation is that the chain *will* be given an
                    // input file, but this verifies the behavior of the unusual
                    // scenario.
                    .set_input_file(&FileArg::loc("inpfile.txt")) // [TC10]
                    .set_output_file(&FileArg::temp(".out"))
                    .push_arg("-a")
                    .push_arg("a-arg-value")
                    .push_arg("-b"));
        ops.push_op(SubProcOperation::new(
            &Executable::new(&"cmd2",
                             ExeFileSpec::Option("--input".into()),
                             ExeFileSpec::Append))
                                  .set_output_file(&FileArg::temp(".o2"))
                                  // Verify individual ops can change dirs
                                  .set_dir("sub/dir") // [TC13]
                                  .push_arg("-D")
                                  .push_arg("DVAL")
        );
        ops.push_call(&FunctionOperation::calling(
            "fop",
            |_in_dir, _inpfiles, _outfile| todo!("not called during test")))
            .set_output_file(&FileArg::loc("fop.done"));
        ops.push_call(&FunctionOperation::calling(
            "flop",
            |_in_dir, _inpfiles, _outfile| todo!("not called during test")))
            .set_output_file(&FileArg::loc("flop.done"));
        ops.push_op(SubProcOperation::new(
            &Executable::new(&"cmdexe3",
                             ExeFileSpec::Option("--input".into()),
                             ExeFileSpec::Append))
                                  .set_output_file(&FileArg::loc("final.out")) // [TC19]
                                  // Verify absolute dir override works
                                  .set_dir("/abs/dir") // [TC14]
        );

        // No chain-level input or output files were set, so the individual
        // operation's settings should apply.  [TC10]

        let mut ex = TestCollector::new();
        let result = ops.execute(&mut ex, &Some("target/loc"));
        match result {
            Ok(actual) => {
                match actual {
                    ActualFile::SingleFile(FileRef::StaticFile(ref sf)) =>
                        assert_eq!(sf, &PathBuf::from("final.out")),
                    _ => assert!(false,
                                 "Expected single static file but got {:?}",
                                 actual),
                };
                assert_eq!(PathBuf::from("/abs/dir/final.out"),
                           actual.to_path(&Some("/abs/dir"))?)
            },
            Err(e) => assert!(false, "Err result: {:?}", e),
        };

        let mut collected = ex.0.into_inner();

        // The last arg of the first op is an assigned output tempfile
        let output0_tmpfile = match &mut collected[0] {
            TestOp::SPO(re) => {
                let outf = PathBuf::from(&re.args.last().unwrap());
                assert!(!outf.exists(),
                        "intermediate temp file #0 {:?} did not get cleaned up!",
                        outf);
                re.args.pop();
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected first op to be a SubProcOperation");
            }
        };


        // The penultimate arg of the second op should be the input tempfile,
        // which should be the output tempfile of the previous op
        match &mut collected[1] {
            TestOp::SPO(re) => {
                let inpf = PathBuf::from(&re.args[re.args.len()-2]);
                assert_eq!(inpf, output0_tmpfile);  // [TC3]
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };
        // The last arg of the second op is another assigned output tempfile
        let output1_tmpfile = match &mut collected[1] {
            TestOp::SPO(re) => {
                let outf = PathBuf::from(&re.args.last().unwrap());
                assert!(!outf.exists(),
                        "intermediate temp file #1 {:?} did not get cleaned up!",
                        outf);
                re.args.pop();  // remove output file
                re.args.pop();  // remove input file
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected second op to be a SubProcOperation");
            }
        };

        // The output file of the fifth (and last) op should be as specified by
        // the individual op.
        match &mut collected[4] {
            TestOp::SPO(re) => {
                let outf = PathBuf::from(&re.args.last().unwrap());
                assert_eq!(outf, PathBuf::from("final.out"));  // [TC10]
                assert!(!outf.exists(),
                        "intermediate temp file #1 {:?} did not get cleaned up!",
                        outf);
                outf
            }
            TestOp::FO(_) => {
                panic!("Expected fifth op to be a SubProcOperation");
            }
        };

        assert_eq!(
            collected, vec!
                [
                    TestOp::SPO(RunExec { name: "test-cmd".into(),
                                          exe: "test-cmd".into(),
                                          args: ["-a",
                                                 "a-arg-value",
                                                 "-b",
                                                 "inpfile.txt",  // [TC10]
                                                 // output file removed above
                                          ].map(Into::<OsString>::into).to_vec(),
                                          dir: Some(PathBuf::from("/ops/run/here"))}), // [TC20]
                    TestOp::SPO(RunExec { name: "cmd2".into(),
                                          exe: "cmd2".into(),
                                          args: ["-D",
                                                 "DVAL",
                                                 "--input",
                                                 // input file removed above
                                                 // output file removed above
                                          ].map(Into::<OsString>::into).to_vec(),
                                          dir: Some(PathBuf::from("/ops/run/here/sub/dir"))}), // [TC22]
                    TestOp::FO(RunFunc { fname: "fop".to_string(),
                                         inpfiles: vec![
                                             output1_tmpfile
                                         ],
                                         outfile: Some(PathBuf::from("fop.done")),
                                         dir: Some("/ops/run/here".into()) }), // [TC21]
                    TestOp::FO(RunFunc { fname: "flop".to_string(),
                                         inpfiles: vec![
                                             "fop.done".into(), // [TC20]
                                         ],
                                         outfile: Some(PathBuf::from("flop.done")),
                                         dir: Some("/ops/run/here".into()) }), // [TC21]
                    TestOp::SPO(RunExec { name: "cmdexe3".into(),
                                          exe: "cmdexe3".into(),
                                          args: ["--input",
                                                 "flop.done",   // [TC20]
                                                 "final.out",   // [TC10]
                                          ].map(Into::<OsString>::into).to_vec(),
                                          dir: Some(PathBuf::from("/abs/dir"))}),
                ]);
        Ok(())
    }

}
