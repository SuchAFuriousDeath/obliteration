use super::{
    builder::LlvmBuilder, error::LlvmRawError, execution_engine::LlvmExecutionEngine,
    module::LlvmModule,
};
use libc::c_char;
use llvm_sys::{
    core::{
        LLVMContextCreate, LLVMContextDispose, LLVMCreateBuilderInContext,
        LLVMModuleCreateWithNameInContext,
    },
    execution_engine::{LLVMCreateExecutionEngineForModule, LLVMExecutionEngineRef},
    prelude::LLVMContextRef,
};
use std::{
    ffi::CStr,
    ptr::null_mut,
    sync::{Mutex, TryLockError},
};
use thiserror::Error;

#[derive(Debug)]
pub struct LlvmContext {
    context: Mutex<LLVMContextRef>,
}

unsafe impl Send for LlvmContext {}
unsafe impl Sync for LlvmContext {}

impl<'llvm> LlvmContext {
    pub fn new() -> Result<Self, CreateContextErrror> {
        let context = unsafe { LLVMContextCreate() };

        if context.is_null() {
            return Err(CreateContextErrror::NullPointerReturned);
        }

        Ok(Self {
            context: Mutex::new(context),
        })
    }

    pub fn create_builder(&self) -> Result<LlvmBuilder<'llvm>, CreateBuilderError> {
        let context = self.context.try_lock()?;

        let inner = unsafe { LLVMCreateBuilderInContext(*context) };

        if inner.is_null() {
            return Err(ContextError::NullPointerReturned.into());
        }

        Ok(LlvmBuilder {
            inner,
            _marker: std::marker::PhantomData,
        })
    }

    pub fn create_module(
        &self,
        name: impl AsRef<CStr>,
    ) -> Result<LlvmModule<'llvm>, CreateModuleError> {
        let context = self.context.try_lock()?;

        let module = unsafe { LLVMModuleCreateWithNameInContext(name.as_ref().as_ptr(), *context) };

        if module.is_null() {
            return Err(ContextError::NullPointerReturned.into());
        }

        Ok(LlvmModule {
            inner: module,
            _marker: std::marker::PhantomData,
        })
    }

    pub(super) fn create_execution_engine_for_module<'module>(
        &self,
        module: &'module LlvmModule<'module>,
    ) -> Result<LlvmExecutionEngine, CreateExececutionEngineError>
    where
        'llvm: 'module,
    {
        let context = self.context.try_lock()?;

        let mut inner: LLVMExecutionEngineRef = null_mut();
        let mut error: *mut c_char = null_mut();

        let retval =
            unsafe { LLVMCreateExecutionEngineForModule(&mut inner, module.inner, &mut error) };

        if retval != 0 {
            let raw_err = unsafe { LlvmRawError::from_ptr(error) };

            return Err(raw_err.into());
        };

        Ok(LlvmExecutionEngine {
            inner,
            _marker: std::marker::PhantomData,
        })
    }
}

impl Drop for LlvmContext {
    fn drop(&mut self) {
        unsafe { LLVMContextDispose(*self.context.get_mut().expect("Couldn't get context")) };
    }
}

#[derive(Debug, Error)]
pub enum CreateContextErrror {
    #[error("LLVM returned null pointer")]
    NullPointerReturned,
}

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("context mutex was poisoned")]
    FailedToLockContext,

    #[error("locking context would block")]
    LockWouldBlock,

    #[error("LLVM returned null pointer")]
    NullPointerReturned,
}

impl<T> From<TryLockError<T>> for ContextError {
    fn from(v: TryLockError<T>) -> Self {
        match v {
            TryLockError::Poisoned(_) => Self::FailedToLockContext,
            TryLockError::WouldBlock => Self::LockWouldBlock,
        }
    }
}

#[derive(Debug, Error)]
pub enum CreateBuilderError {
    #[error(transparent)]
    ContextError(#[from] ContextError),
}

impl<T> From<TryLockError<T>> for CreateBuilderError {
    fn from(v: TryLockError<T>) -> Self {
        Self::ContextError(v.into())
    }
}

#[derive(Debug, Error)]
pub enum CreateModuleError {
    #[error(transparent)]
    ContextError(#[from] ContextError),
}

impl<T> From<TryLockError<T>> for CreateModuleError {
    fn from(v: TryLockError<T>) -> Self {
        Self::ContextError(v.into())
    }
}

#[derive(Debug, Error)]
pub enum CreateExececutionEngineError {
    #[error(transparent)]
    ContextError(#[from] ContextError),

    #[error("LLVM error: {0}")]
    LlvmError(#[from] LlvmRawError),
}

impl<T> From<TryLockError<T>> for CreateExececutionEngineError {
    fn from(v: TryLockError<T>) -> Self {
        Self::ContextError(v.into())
    }
}
