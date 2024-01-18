use self::module::{LlvmModule, Types};
use llvm_sys::core::*;
use llvm_sys::prelude::LLVMContextRef;
use std::ffi::CString;
use std::sync::{Arc, Mutex};

pub(self) mod error;
pub(self) mod module;
pub(self) mod types;

/// A LLVM wrapper for thread-safe.
#[derive(Debug)]
pub struct Llvm {
    context: Mutex<LLVMContextRef>,
}

impl Llvm {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            context: Mutex::new(unsafe { LLVMContextCreate() }),
        })
    }

    pub fn create_module(self: &Arc<Self>, name: &str) -> LlvmModule {
        let (module, builder, types) = self.with_context(|ctx| unsafe {
            let name = CString::new(name).unwrap();
            let module = LLVMModuleCreateWithNameInContext(name.as_ptr(), ctx);
            let builder = LLVMCreateBuilderInContext(ctx);
            let types = Types::from_context(ctx);

            (module, builder, types)
        });

        LlvmModule::new(self, module, builder, types)
    }

    fn with_context<F, R>(&self, f: F) -> R
    where
        F: FnOnce(LLVMContextRef) -> R,
    {
        f(*self.context.lock().unwrap())
    }
}

impl Drop for Llvm {
    fn drop(&mut self) {
        unsafe { LLVMContextDispose(*self.context.get_mut().unwrap()) };
    }
}

unsafe impl Send for Llvm {}
unsafe impl Sync for Llvm {}
