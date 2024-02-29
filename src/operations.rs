pub mod generic;
pub mod subproc;
pub mod function;
pub mod chained;
pub use crate::operations::generic::*;
pub use crate::operations::subproc::SubProcOperation;
pub use crate::operations::function::FunctionOperation;
pub use crate::operations::chained::ChainedOps;
