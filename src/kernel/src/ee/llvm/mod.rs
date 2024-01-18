use self::codegen::Codegen;
use super::ExecutionEngine;
use crate::fs::VPathBuf;
use crate::llvm::Llvm;
use crate::rtld::Module;
use crate::syscalls::Syscalls;
use std::sync::Arc;
use thiserror::Error;

mod codegen;

/// An implementation of [`ExecutionEngine`] using JIT powered by LLVM IR.
#[derive(Debug)]
pub struct LlvmEngine {
    llvm: Arc<Llvm>,
}

impl LlvmEngine {
    pub fn new(llvm: &Arc<Llvm>) -> Arc<Self> {
        Arc::new(Self { llvm: llvm.clone() })
    }

    fn lift(
        &self,
        module: &Module<Self>,
    ) -> Result<crate::llvm::module::ExecutionEngine, LiftError> {
        // Get a list of public functions.
        let targets = match module.entry() {
            Some(v) => vec![v].into_boxed_slice(),
            None => Box::new([]),
        };

        // Lift the public functions.
        let mut lifting = self.llvm.create_module(module.path());
        let mut codegen = Codegen::new(&mut lifting, module);

        for addr in targets.into_iter() {
            codegen
                .lift(*addr)
                .map_err(|e| LiftError::LiftingFailed(*addr, e))?;
        }

        drop(codegen);

        // Create LLVM execution engine.
        let lifted = lifting.create_execution_engine()?;

        Ok(lifted)
    }
}

impl ExecutionEngine for LlvmEngine {
    type RawFn = RawFn;
    type SetupModuleErr = SetupModuleError;
    type GetFunctionErr = GetFunctionError;

    fn set_syscalls(&self, v: Syscalls) {
        todo!()
    }

    fn setup_module(self: &Arc<Self>, md: &mut Module<Self>) -> Result<(), Self::SetupModuleErr> {
        self.lift(md)
            .map(|_| ())
            .map_err(|e| SetupModuleError::LiftFailed(md.path().to_owned(), e))
    }

    unsafe fn get_function(
        self: &Arc<Self>,
        md: &Arc<Module<Self>>,
        addr: usize,
    ) -> Result<Arc<Self::RawFn>, Self::GetFunctionErr> {
        todo!()
    }
}

/// An implementation of [`ExecutionEngine::RawFn`].
#[derive(Debug)]
pub struct RawFn {}

impl super::RawFn for RawFn {
    fn addr(&self) -> usize {
        todo!()
    }

    unsafe fn exec1<R, A>(&self, a: A) -> R {
        todo!()
    }
}

/// An implementation of [`ExecutionEngine::SetupModuleErr`].
#[derive(Debug, Error)]
pub enum SetupModuleError {
    #[error("failed to lift module {0}")]
    LiftFailed(VPathBuf, LiftError),
}

/// An implementation of [`ExecutionEngine::GetFunctionErr`].
#[derive(Debug, Error)]
pub enum GetFunctionError {}

/// Represents an error when module lifting is failed.
#[derive(Debug, Error)]
pub enum LiftError {
    #[error("cannot lift function {0:#018x}")]
    LiftingFailed(usize, #[source] self::codegen::LiftError),

    #[error("cannot create LLVM execution engine for {0}")]
    CreateExecutionEngineFailed(#[from] crate::llvm::error::Error),
}
