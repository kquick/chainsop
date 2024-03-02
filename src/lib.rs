//! This package provides functionality for running one or more executables as a
//! sub-process operation.  The sub-process operation is specified as the
//! command to run, the arguments to the command, and the input and output files.
//!
//! The first step is to define the executable that should be invoked.  This is
//! done by identifying the executable itself and the manner in which its input
//! and output files (if any) should be specified when running that executable.
//! The input and output files can be supplied to the executable in a number of
//! ways: by replacing a pattern in one or more of the args, or by simply
//! appending the file to the list of arguments (if both input and output files
//! are marked this way, the input file(s) are appended first, followed by the
//! output file.
//!
//! For example, a C compilation invokes the `cc` compiler, specifying the output
//! file via the `-o` flag and appending the input filename(s) on the command
//! line:
//!
//! ```
//! use chainsop::*;
//! let compile = Executable::new("cc",
//!                               ExeFileSpec::Append,         // input file(s)
//!                               ExeFileSpec::option("-o"));  // output file
//! ```
//!
//! > ------
//! >
//! > The [ExeFileSpec] has an [Option] constructor which takes a [String],
//! > but provides the [ExeFileSpec::option()] helper to take anything that
//! > can be converted to a [String].
//! >
//! > ------
//!
//! This allows a generic description of the Executable that describes how to
//! invoke it in general, but which is not specific to any particular invocation.
//! It is also possible to add a set of arguments that are supplied any time this
//! Executable is invoked.
//!
//! ```
//! # use chainsop::*;
//! let compile = Executable::new("cc",
//!                               ExeFileSpec::Append,         // input file(s)
//!                               ExeFileSpec::option("-o"))   // output file
//!               .push_arg("-c")
//!               .push_arg("-O0")
//!               .push_arg("-g")
//!               .push_arg("-X").push_arg("c");
//!
//! let link = Executable::new("cc",
//!                            ExeFileSpec::Append,         // input file(s)
//!                            ExeFileSpec::option("-o"))   // output file
//!               .push_arg("--print-map");
//! ```
//!
//! To actually invoke the Executable as a sub-process operation, a
//! [SubProcOperation] is defined to use the [Executable] along with specific
//! input and/or output files and any additional arguments that are specific to
//! that invocation:
//!
//! ```
//! # use chainsop::*;
//! # let compile = Executable::new("cc",
//! #                               ExeFileSpec::Append,         // input file(s)
//! #                               ExeFileSpec::option("-o"))  // output file
//! #               .push_arg("-c")
//! #               .push_arg("-O0")
//! #               .push_arg("-g")
//! #               .push_arg("-X").push_arg("c");
//! #
//! # let link = Executable::new("cc",
//! #                            ExeFileSpec::Append,         // input file(s)
//! #                            ExeFileSpec::option("-o"))  // output file
//! #               .push_arg("--print-map");
//! let mut compile_foo = SubProcOperation::new(&compile)
//!                       .set_dir("src/")
//!                       .set_input_file(&FileArg::loc("foo.c"))
//!                       .set_output_file(&FileArg::loc("../build/foo.o"))
//!                       .push_arg("-DDEBUG=1");
//! let mut compile_bar = SubProcOperation::new(&compile)
//!                       .set_dir("src/")
//!                       .set_input_file(&FileArg::loc("bar.c"))
//!                       .set_output_file(&FileArg::loc("../build/bar.o"));
//! let mut link_myapp = SubProcOperation::new(&link)
//!                      .set_dir("build/")
//!                      .set_input_file(&FileArg::loc("foo.o"))
//!                      .add_input_file(&FileArg::loc("bar.o"))
//!                      .set_output_file(&FileArg::loc("myapp.exe"));
//! let mut test_myapp = SubProcOperation::new(&Executable::new("bash",
//!                                                             ExeFileSpec::Append,
//!                                                             ExeFileSpec::NoFileUsed))
//!                      .set_dir("build/")
//!                      .set_input_file(&FileArg::loc("myapp.exe"))
//!                      .set_output_file(&FileArg::temp("test_out"));
//! let mut check_results = SubProcOperation::new(&Executable::new("grep",
//!                                                                ExeFileSpec::Append,
//!                                                                ExeFileSpec::NoFileUsed))
//!                         .push_arg("Passed")
//!                         .set_input_file(&FileArg::glob_in("build/", "*.test_out"));
//! ```
//!
//! > ------
//! >
//! > If you actually attempt to use the `compile_foo` or `compile_bar` above,
//! > you will get an error that the above statements create a temporary value
//! > that is freed at the end of the statement, and Rust will suggest that you
//! > let-bind to create a longer-lived value (which is confusing: these are
//! > let binds!). This occurs because the chained methods return a `&mut Self`
//! > for maximum flexibility, but Rust needs someone to take ownership of the
//! > reference or else it will be released.  There are two solutions:
//! >
//! > 1. Use two statements.  The first is a mutable let bind of just the
//! >    operation, and the second is the chained modifications of that
//! >    operation.
//! >
//! > 1. Add a `.clone()` to the end of the chain.
//! >
//! > Both of these solutions are shown below.
//! >
//! > Additional info: <https:://randompoison.github.io/posts/returning-self/>
//! >
//! > ------
//!
//! Note that the above have *defined* the operations, but not executed them. To
//! execute them, call the `execute` method for each [SubProcOperation] with an
//! [Executor].  The [Executor] is the lowest level of abstraction that
//! determines the actual manner in which the operations are performed.  For the
//! example below, the [Executor::DryRun] will be used, which echoes the
//! operations to `stderr` but does not execute them; there is also an
//! [Executor::NormalWithEcho] which prints operations to `stderr` just before
//! actually performing them, and an [Executor::NormalRun] which performs the
//! operations but does not display them.  It is also possible to define your own
//! Executors which implement the [OsRun] trait.
//!
//! ```
//! # use std::io;
//! # use chainsop::*;
//! # let compile = Executable::new("cc",
//! #                               ExeFileSpec::Append,         // input file(s)
//! #                               ExeFileSpec::option("-o"))  // output file
//! #               .push_arg("-c")
//! #               .push_arg("-O0")
//! #               .push_arg("-g")
//! #               .push_arg("-X").push_arg("c");
//! #
//! # let link = Executable::new("cc",
//! #                            ExeFileSpec::Append,         // input file(s)
//! #                            ExeFileSpec::option("-o"))  // output file
//! #               .push_arg("--print-map");
//! #
//! let mut compile_foo = SubProcOperation::new(&compile);
//! compile_foo.set_dir("src/")
//!            .set_input_file(&FileArg::loc("foo.c"))
//!            .set_output_file(&FileArg::loc("../build/foo.o"))
//!            .push_arg("-DDEBUG=1");
//! let mut compile_bar = SubProcOperation::new(&compile)
//!                       .set_dir("src/")
//!                       .set_input_file(&FileArg::loc("bar.c"))
//!                       .set_output_file(&FileArg::loc("../build/bar.o"))
//!                       .clone();
//! // Similar .clone() modifications to link_myapp, test_myapp, and check_results ...
//! # let mut link_myapp = SubProcOperation::new(&link)
//! #                      .set_dir("build/")
//! #                      .set_input_file(&FileArg::loc("foo.o"))
//! #                      .add_input_file(&FileArg::loc("bar.o"))
//! #                      .set_output_file(&FileArg::loc("myapp.exe"))
//! #                      .clone();
//! # let mut test_myapp = SubProcOperation::new(&Executable::new("bash",
//! #                                                             ExeFileSpec::Append,
//! #                                                             ExeFileSpec::NoFileUsed))
//! #                      .set_dir("build/")
//! #                      .set_input_file(&FileArg::loc("myapp.exe"))
//! #                      .set_output_file(&FileArg::temp("test_out"))
//! #                      .clone();
//! # let mut check_results = SubProcOperation::new(&Executable::new("grep",
//! #                                                                ExeFileSpec::Append,
//! #                                                                ExeFileSpec::NoFileUsed))
//! #                         .push_arg("Passed")
//! #                         .set_input_file(&FileArg::glob_in("build/", "*.test_out"))
//! #                         .clone();
//!
//! let mut executor = Executor::DryRun;
//!
//! println!("Compile is {:?}", compile_foo);
//! compile_foo.execute(&mut executor, &Some("/home/user/myapp-src"))?;
//! compile_bar.execute(&mut executor, &Some("/home/user/myapp-src"))?;
//! link_myapp.execute(&mut executor, &Some("/home/user/myapp-src"))?;
//! test_myapp.execute(&mut executor, &Some("/home/user/myapp-src"))?;
//! check_results.execute(&mut executor, &Some("/home/user/myapp-src"))?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! In the above, there was a lot of repetition in `execute()` argument
//! handling, as well as the potential need for error handling after each
//! execution.  In addition, input and output files may need to be aligned
//! between operations: the output file from `link_myapp` must be the input file
//! for `test_myapp` and the stdout from `test_myapp` is written to a temporary
//! file with the suffix `.test_out`, which the subsequent `check_results`
//! operation must recover by wildcard matching.
//!
//! > ------
//! >
//! > The `chainsop` package does not provide explicit methods of capturing
//! > `stdout` or `stderr`, nor for providing specific `stdin` to
//! > [SubProcOperation] invocations.  Instead, `stdout` or `stderr` should be
//! > redirected to (temporary) files which are then used as input files
//! > (instead of `stdin`) for subsequent operations.
//! >
//! > ------
//!
//! These issues can be more easily be handled by using the [ChainedOps] object,
//! which is supplied with multiple [SubProcOperation] objects that it will
//! perform in a chained sequence.
//!
//! 1. The output file from one [SubProcOperation] is automatically specified as
//!    the input file for the next [SubProcOperation].  The [ChainedOps] object
//!    is provided with the initial input file and the final output file in the
//!    same manner as an individual [SubProcOperation] would have been and it
//!    uses these to configure the first and last [SubProcOperation] objects in
//!    the chain.
//!
//! 2. Error handling is performed after each [SubProcOperation] is executed.
//!    This is generally the same action that the `?` suffix specifies, but it
//!    will also ensure that any temporary files created as part of the chain are
//!    removed.
//!
//! Below is the same example we have been using, re-implemented as a
//! [ChainedOps] sequence:
//!
//!
//! ```
//! # use std::io;
//! # use chainsop::*;
//! # let compile = Executable::new("cc",
//! #                               ExeFileSpec::Append,         // input file(s)
//! #                               ExeFileSpec::option("-o"))   // output file
//! #               .push_arg("-c")
//! #               .push_arg("-O0")
//! #               .push_arg("-g")
//! #               .push_arg("-X").push_arg("c");
//! #
//! # let link = Executable::new("cc",
//! #                            ExeFileSpec::Append,         // input file(s)
//! #                            ExeFileSpec::option("-o"))   // output file
//! #               .push_arg("--print-map");
//! #
//! let mut build_ops = ChainedOps::new("myapp build");
//!
//! // A plain operation can be modified after adding it to the chain
//! let mut compile_foo = build_ops.push_op(&SubProcOperation::new(&compile));
//! compile_foo.set_dir("src/")
//!     .set_input_file(&FileArg::loc("foo.c"))
//!     .set_output_file(&FileArg::loc("../build/foo.o"))
//!     .push_arg("-DDEBUG=1");
//!
//! // Or the operation can be fully-configured and then added to the chain
//! build_ops.push_op(SubProcOperation::new(&compile)
//!                   .set_dir("src/")
//!                   .set_input_file(&FileArg::loc("bar.c"))
//!                   .set_output_file(&FileArg::loc("../build/bar.o")));
//! build_ops.push_op(SubProcOperation::new(&link)
//!                   .set_dir("build/")
//!                   .set_input_file(&FileArg::loc("foo.o"))
//!                   .add_input_file(&FileArg::loc("bar.o"))
//!                   .set_output_file(&FileArg::loc("myapp.exe")));
//! build_ops.push_op(SubProcOperation::new(&Executable::new("bash",
//!                                                          ExeFileSpec::Append,
//!                                                          ExeFileSpec::NoFileUsed))
//!                   .set_dir("build/")
//!                   .set_input_file(&FileArg::loc("myapp.exe"))
//!                   .set_output_file(&FileArg::temp("test_out")));
//! build_ops.push_op(SubProcOperation::new(&Executable::new("grep",
//!                                                          ExeFileSpec::Append,
//!                                                          ExeFileSpec::NoFileUsed))
//!                   .push_arg("Passed")
//!                   .set_input_file(&FileArg::glob_in("build/", "*.test_out")));
//!
//! let mut executor = Executor::DryRun;
//! build_ops.execute(&mut executor, &Some("/home/user/myapp-src"))?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! When using [ChainedOps] to perform a sequence of operations, it is sometimes
//! useful to perform a local computation at some point during the chain.  This
//! can be done by adding a [FunctionOperation] into the chain at the appropriate
//! location.  The [FunctionOperation] supports the same general methods as a
//! [SubProcOperation], but instead of creating a sub-process and running an
//! executable in that sub-process, it calls a specified local function and
//! passes the names of the input and output files.
//!
//! It is additionally sometimes useful to enable or disable individual
//! operations within a chain.  Using our build examples above, perhaps our
//! builder application acts like the `make` tool and does not perform
//! compilations if the file is up-to-date.  The previously-specified chain can
//! be used, but prior to executing the chain, the modification dates of the two
//! `.c` files can be checked against the modification date of the `.exe` file
//! and one or both compilation operations can be disabled if the corresponding
//! `.c` file hasn't changed.  This disabling (or enabling) can be done by
//! calling the [ChainedOpRef::active()] method on the [ChainedOpRef] handle for
//! that operation in the chain.
//!
//!
//! -----
//! ## Structures, Traits, and their relationships:
//!
//! These are described below in more detail, but this is an overview to help
//! visualize the relationships.
//!
//!  * ##### [Executable]
//!
//!    General description of an executable program that can be run: the
//!    executable file, standard arguments, and the manner in which input and
//!    output files should be specified via additional arguments.
//!
//!  * ##### [SubProcOperation]
//!
//!    A specific operation to run an [Executable] is created by referencing the
//!    generic executable information and the specific files to use as input and
//!    output, along with any additional arguments for that specific operation.
//!
//!  * ##### [ChainedOps]
//!
//!    Allows a sequence of multiple specific operations to be run, the typical
//!    mode is that the output file from a previous operation becomes the input
//!    file to a subsequent operation.  Also in this mode, it is convenient to
//!    specify that intermediate files in the chain should be temporary files
//!    that are automatically removed upon completion of the execution.
//!
//!  * ##### [FunctionOperation]
//!
//!    Allows running a local function to convert input file(s) to output files.
//!    This is convenient to use in [ChainedOps] sequences for steps that are
//!    local computations rather than external executables.
//!
//  * The `RunnableOp` serves as a wrapper to contain the other types of
//    operation; higher level functionality is written in terms of a RunnableOp
//    type that will dispatch trait operations to the specific operation type's
//    trait handler.
//!
//!  * ##### [OsRun]
//!
//!    This trait provides the active functionality used to perform the set of
//!    operations defined by the [SubProcOperation] or [ChainedOps].  This also
//!    provides an abstraction layer that can adjust or redirect the active
//!    operations (e.g. for testing).
//!
// ```text
//
//     File Handling                          Generic Description
//    --------------------------              -------------------
//
//     FileArg:                               Executable:
//      ^    Temp("seed")                      ^         exe_file: PathBuf
//      |    Loc(PathBuf)                      |         base_args: Vec<String>
//      |    Glob("glob")                      |   +--<- inp_file
//      |                                      |   +--<- exe_file
//      |  FileTransformation: <------------+  |   |
//      +--<- inp_filename  ^........       |  |  ExeFileSpec:
//      +--<- out_filename          :       |  |     NoFileUsed
//            in_dir                :       |  |     Append
//                                  :       |  |     Option("-opt")
//       [trait] FilesPrep: ........:....   |  |     ViaCall(fn)
//                 set_input_file       :   |  |
//                 set_output_file      :   |  |
//                 set_dir              :   |  |  Specific Operations
//                                      :   |  |  -------------------
//                                      :   |  |
//                                      :...|..|........> SubProcOperation:
//                                      :   |  +------------<- exec   ^  ^
//                                      :   |                  args   |  :
//     ActualFile:                      :   +---------------<- files  |  :
//      ^        NoActualFile           :   |                         |  :
//      |  +-<-- SingleFile             :...|..> FunctionOperation:   |  :
//      |  +-<-* MultiFile              :   |        name       ^^....|..:
//      |  |                            :   |        call: fn   |     |  :
//      |  v                            :   +-----<- files      |     |  :
//      | FileRef:                      :   |                   |     |  :
//      |    StaticFile(PathBuf)        :...|.....v       v.....|.....|..:
//      |    TempFile(*active mgmt*)    :   |     RunnableOp:   |     |  :
//      |                               :   |     ^   Call ->---+     |  :
//      |                               :   |     |   Exec ->---------+  :
//      |                               :   |     |                      :
//      |                               :...|.....|...v       v..........:
//      |                                   |     |   ChainedOps:        :
//      |                                   |     |     <ChainedIntOps>  :
//      |                                   |     +----<-* chain         :
//      |                                   +------------- files         :
//      |                                                                :
//      |               [trait] OpInterface: ............................:
//      +------------------------- execute()
//
//  [solid lines = class references, dotted lines = trait implementations]
// ```
//!
//! ----------------------------------------------------------------------
//!
//! ## Alternatives:
//!
//! * `subprocess` crate (<https://crates.io/crates/subprocess>)
//!
//!     The `subprocess` crate allows creation of pipelines connected via
//!     stdin/stdout, but not sequences using shared input/output files.
//!
//!     In addition, `chainsop` provides automatic creation and management of
//!     temporary files used in the above.
//!
//!     The `chainsop` package provides more direct support for incrementally
//!     building the set of commands with outputs; the subprocess crate would
//!     require more discrete management and building of a `Vec<Exec>`.
//!
//!     The `chainsop` package allows elements of the chain to be local functions
//!     called in the proper sequence of operations and for elements of the chain
//!     to be disabled prior to actual execution (where they are skipped).
//!
//!     The `subprocess` crate provides more features for handling stdout/stderr
//!     redirection, non-blocking and timed sub-process waiting, and interaction
//!     with the sub-process.
//!
//!     Summary: significant overlap in capabilities with slightly different
//!     use-case targets and features.
//!
//! * `duct` (<https://github.com/oconner663/duct.rs>)
//!
//!     Lightweight version of the `subprocess` crate
//!
//! * `cargo-make`, `devrc`, `rhiz`, `run-cli`, `naumann`, `yamis`
//!
//!    Task runners, requiring an external specification of the commands and no
//!    support for chaining inputs/outputs.  These could be written on top of
//!    `chainsop`.
//!
//! * `steward` crate (<https://crates.io/crates/steward>)
//!
//!    Useful for running multiple commands and allows dependency management, but
//!    not input/output chaining or incremental command building.  Does support
//!    other features like environment control and process pools.  Closer to
//!    `chainsop` than the task runners, but again, this could be written on top
//!    of `chainsop`.

mod filehandling;
pub mod errors;
mod executable;
mod operations;
mod execution;

// Exports are setup here such that the user only needs to use the top level
// "chainsop" module to access the public API.

#[doc(inline)]
pub use filehandling::defs::{FilesPrep,FileArg,ActualFile,FileRef};
pub use errors::*;
#[doc(inline)]
pub use executable::{Executable, ExeFileSpec};
#[doc(inline)]
pub use operations::generic::{OpInterface};
#[doc(inline)]
pub use operations::subproc::SubProcOperation;
#[doc(inline)]
pub use operations::function::FunctionOperation;
#[doc(inline)]
pub use operations::chained::{ChainedOps, Activation, ChainedOpRef};
#[doc(inline)]
pub use execution::*;
