use super::error::LlvmRawError;
use llvm_sys::{
    core::{
        LLVMContextDispose, LLVMCreateBuilderInContext, LLVMDisposeBuilder, LLVMDisposeModule,
        LLVMModuleCreateWithNameInContext,
    },
    execution_engine::{LLVMCreateExecutionEngineForModule, LLVMExecutionEngineRef},
    prelude::{LLVMBuilderRef, LLVMContextRef, LLVMModuleRef, LLVMValueRef},
};
use std::{
    ffi::CStr,
    num::NonZeroI32,
    sync::{Mutex, TryLockError},
};
use thiserror::Error;

/// This module provider safe wrappers for the LLVM-C api. Do not change things in there without good reason.

pub struct LlvmContext {
    context: Mutex<LLVMContextRef>,
}

impl<'llvm> LlvmContext {
    pub(super) fn create_builder(&self) -> Result<LLVMBuilderHandle<'llvm>, CreateError> {
        let context = self.context.try_lock()?;

        let builder = unsafe { LLVMCreateBuilderInContext(*context) };

        if builder.is_null() {
            return Err(CreateError::NullPointerReturned);
        }

        Ok(LLVMBuilderHandle {
            builder,
            _marker: std::marker::PhantomData,
        })
    }

    pub(super) fn create_module(
        &self,
        name: impl AsRef<CStr>,
    ) -> Result<LlvmModuleHandle<'llvm>, CreateError> {
        let context = self.context.try_lock()?;

        let module = unsafe { LLVMModuleCreateWithNameInContext(name.as_ref().as_ptr(), *context) };

        if module.is_null() {
            return Err(CreateError::NullPointerReturned);
        }

        Ok(LlvmModuleHandle {
            module,
            _marker: std::marker::PhantomData,
        })
    }

    pub(super) fn create_execution_engine_for_module<'module>(
        &self,
        module: &LlvmModuleHandle<'module>,
    ) -> Result<LlvmExecutionEngineHandle, CreateError> {
        use libc::c_char;
        use std::ptr::null_mut;

        let context = self.context.try_lock()?;

        let mut ee: LLVMExecutionEngineRef = null_mut();
        let mut error: *mut c_char = null_mut();

        let module = module.module;

        let retval = unsafe { LLVMCreateExecutionEngineForModule(&mut ee, module, &mut error) };

        if let Some(retval) = NonZeroI32::new(retval) {
            let raw = unsafe { LlvmRawError::new(error) };

            return Err(CreateError::LlvmError(retval, raw));
        };

        Ok(LlvmExecutionEngineHandle {
            ee,
            _marker: std::marker::PhantomData,
            _marker2: std::marker::PhantomData,
        })
    }
}

impl Drop for LlvmContext {
    fn drop(&mut self) {
        unsafe { LLVMContextDispose(*self.context.get_mut().expect("Couldn't get context")) };
    }
}

#[derive(Debug, Error)]
pub enum CreateError {
    #[error("context mutex was poisoned")]
    FailedToLockContext,

    #[error("locking context would block")]
    LockWouldBlock,

    #[error("LLVM return null pointer")]
    NullPointerReturned,

    #[error("LLVM error: {0}")]
    LlvmError(NonZeroI32, #[source] LlvmRawError),
}

impl<T> From<TryLockError<T>> for CreateError {
    fn from(v: TryLockError<T>) -> Self {
        match v {
            TryLockError::Poisoned(_) => CreateError::FailedToLockContext,
            TryLockError::WouldBlock => CreateError::LockWouldBlock,
        }
    }
}

pub(super) struct LLVMValueHandle<'llvm> {
    value: LLVMValueRef,
    _marker: std::marker::PhantomData<&'llvm ()>,
}

impl<'llvm> LLVMValueHandle<'llvm> {}

pub(super) struct LLVMBuilderHandle<'llvm> {
    builder: LLVMBuilderRef,
    _marker: std::marker::PhantomData<&'llvm ()>,
}

impl<'llvm> LLVMBuilderHandle<'llvm> {}

impl Drop for LLVMBuilderHandle<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeBuilder(self.builder) };
    }
}

pub(super) struct LlvmModuleHandle<'llvm> {
    module: LLVMModuleRef,
    _marker: std::marker::PhantomData<&'llvm ()>,
}

impl<'llvm> LlvmModuleHandle<'llvm> {}

impl Drop for LlvmModuleHandle<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeModule(self.module) };
    }
}

pub(super) struct LlvmExecutionEngineHandle<'llvm, 'module> {
    ee: LLVMExecutionEngineRef,
    _marker: std::marker::PhantomData<&'llvm ()>,
    _marker2: std::marker::PhantomData<&'module ()>,
}
