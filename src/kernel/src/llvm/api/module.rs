use llvm_sys::{core::LLVMDisposeModule, prelude::LLVMModuleRef};

pub struct LlvmModule<'llvm> {
    pub(super) inner: LLVMModuleRef,
    pub(super) _marker: std::marker::PhantomData<&'llvm ()>,
}

impl<'llvm> LlvmModule<'llvm> {}

impl Drop for LlvmModule<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeModule(self.inner) };
    }
}
