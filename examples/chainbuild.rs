use chainsop::*;
use lazy_static::lazy_static;

lazy_static!(
    static ref C_COMPILER: Executable =
        Executable::new("cc",
                        ExeFileSpec::Append,
                        ExeFileSpec::option("-o"))
        .push_arg("-c")
        .push_arg("-O0")
        .push_arg("-g")
        .push_arg("-X").push_arg("c");

    static ref LINKER: Executable =
        Executable::new("cc",
                        ExeFileSpec::Append,
                        ExeFileSpec::option("-o"))
        .push_arg("--print-map");
);


fn build_ops() -> ChainedOps
{
    let build_ops = ChainedOps::new("build myapp");

    // A plain operation can be modified after adding it to the chain
    let mut compile_foo = build_ops.push_op(&SubProcOperation::new(&C_COMPILER));
    compile_foo.set_dir("src/")
        .set_input_file(&FileArg::loc("foo.c"))
        .set_output_file(&FileArg::loc("../build/foo.o"))
        .push_arg("-DDEBUG=1");

    // Or the operation can be fully-configured and then added to the chain
    build_ops.push_op(SubProcOperation::new(&C_COMPILER)
                      .set_dir("src/")
                      .set_input_file(&FileArg::loc("bar.c"))
                      .set_output_file(&FileArg::loc("../build/bar.o")));
    build_ops.push_op(SubProcOperation::new(&LINKER)
                      .set_dir("build/")
                      .set_input_file(&FileArg::loc("foo.o"))
                      .add_input_file(&FileArg::loc("bar.o"))
                      .set_output_file(&FileArg::loc("myapp.exe")));
    build_ops.push_op(SubProcOperation::new(&Executable::new("bash",
                                                             ExeFileSpec::Append,
                                                             ExeFileSpec::NoFileUsed))
                      .set_dir("build/")
                      .set_input_file(&FileArg::loc("myapp.exe"))
                      .set_output_file(&FileArg::temp("test_out")));
    build_ops.push_op(SubProcOperation::new(&Executable::new("grep",
                                                             ExeFileSpec::Append,
                                                              ExeFileSpec::NoFileUsed))
                      .push_arg("Passed")
                      .set_input_file(&FileArg::glob_in("build/", "*.test_out")));
    build_ops
}

fn build(ops: &mut ChainedOps) -> anyhow::Result<()>
{
    let mut executor = Executor::DryRun;
    ops.execute(&mut executor, &Some("/home/user/myapp-src"))?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    build(&mut build_ops())
}
