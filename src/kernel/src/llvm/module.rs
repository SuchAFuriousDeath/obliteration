use super::{Error, Llvm};
use llvm_sys::core::*;
use llvm_sys::execution_engine::*;
use llvm_sys::prelude::LLVMBool;
use llvm_sys::prelude::LLVMBuilderRef;
use llvm_sys::prelude::LLVMContextRef;
use llvm_sys::prelude::LLVMModuleRef;
use llvm_sys::prelude::LLVMTypeRef;
use llvm_sys::prelude::LLVMValueRef;

use std::ffi::c_char;
use std::ffi::CStr;
use std::ptr::null_mut;
use std::sync::Arc;

/// A wrapper on LLVM module for thread-safe.
pub struct LlvmModule {
    llvm: Arc<Llvm>,
    module: LLVMModuleRef,
    builder: LLVMBuilderRef,
    types: Types,
}

pub struct LLVMBuilderHandle<'a> {
    builder: LLVMBuilderRef,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl LLVMBuilderHandle<'_> {
    pub fn new(builder: LLVMBuilderRef) -> Self {
        Self {
            builder,
            _marker: std::marker::PhantomData,
        }
    }
}

impl LlvmModule {
    pub(super) fn new(
        llvm: &Arc<Llvm>,
        module: LLVMModuleRef,
        builder: LLVMBuilderRef,
        types: Types,
    ) -> Self {
        Self {
            llvm: llvm.clone(),
            module,
            builder,
            types,
        }
    }

    pub fn create_execution_engine(mut self) -> Result<ExecutionEngine, Error> {
        let mut ee: LLVMExecutionEngineRef = null_mut();
        let module = self.module;
        let mut error: *mut c_char = null_mut();

        self.module = null_mut();

        if self.llvm.with_context(|_| unsafe {
            LLVMCreateExecutionEngineForModule(&mut ee, module, &mut error)
        }) != 0
        {
            return Err(unsafe { Error::new(error) });
        }

        Ok(ExecutionEngine {
            llvm: self.llvm.clone(),
            ee,
        })
    }

    pub fn create_entry_function(&mut self, offset: usize) -> LLVMValueRef {
        self.create_function(
            CStr::from_bytes_with_nul(b"entry\0").unwrap(),
            offset,
            self.types.void,
            &mut [self.types.void_ptr],
            false,
        )
    }

    pub fn create_function(
        &mut self,
        name: impl AsRef<CStr>,
        offset: usize,
        ret: LLVMTypeRef,
        params: &mut [LLVMTypeRef],
        is_var_arg: bool,
    ) -> LLVMValueRef {
        let func_type = unsafe {
            LLVMFunctionType(
                ret,
                params.as_mut_ptr(),
                params.len().try_into().unwrap(),
                is_var_arg as LLVMBool,
            )
        };

        unsafe { LLVMAddFunction(self.module, name.as_ref().as_ptr(), func_type) }
    }
}

impl Drop for LlvmModule {
    fn drop(&mut self) {
        let m = self.module;

        if !m.is_null() {
            self.llvm.with_context(|_| unsafe { LLVMDisposeModule(m) });
        }
    }
}

/// A wrapper on LLVM Execution Engine for thread-safe.
///
/// # Safety
/// All JITed functions from this EE must not invoked once this EE has been droped.
pub struct ExecutionEngine {
    llvm: Arc<Llvm>,
    ee: LLVMExecutionEngineRef,
}

impl Drop for ExecutionEngine {
    fn drop(&mut self) {
        self.llvm
            .with_context(|_| unsafe { LLVMDisposeExecutionEngine(self.ee) });
    }
}

pub(super) struct Types {
    void: LLVMTypeRef,
    void_ptr: LLVMTypeRef,
    i8: LLVMTypeRef,
    i16: LLVMTypeRef,
    i32: LLVMTypeRef,
    i64: LLVMTypeRef,
    u8: LLVMTypeRef,
    u16: LLVMTypeRef,
    u32: LLVMTypeRef,
    u64: LLVMTypeRef,
}

impl Types {
    pub fn from_context(ctx: LLVMContextRef) -> Self {
        unsafe {
            Self {
                void: LLVMVoidTypeInContext(ctx),
                void_ptr: LLVMPointerTypeInContext(ctx, 0),
                i8: LLVMInt8TypeInContext(ctx),
                i16: LLVMInt16TypeInContext(ctx),
                i32: LLVMInt32TypeInContext(ctx),
                i64: LLVMInt64TypeInContext(ctx),
                u8: LLVMInt8TypeInContext(ctx),
                u16: LLVMInt16TypeInContext(ctx),
                u32: LLVMInt32TypeInContext(ctx),
                u64: LLVMInt64TypeInContext(ctx),
            }
        }
    }
}
