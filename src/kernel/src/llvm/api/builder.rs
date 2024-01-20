use llvm_sys::{core::LLVMDisposeBuilder, prelude::LLVMBuilderRef};

pub(super) struct LlvmBuilder<'llvm> {
    pub(super) inner: LLVMBuilderRef,
    pub(super) _marker: std::marker::PhantomData<&'llvm ()>,
}

impl<'llvm> LlvmBuilder<'llvm> {}

impl Drop for LlvmBuilder<'_> {
    fn drop(&mut self) {
        unsafe { LLVMDisposeBuilder(self.inner) };
    }
}
