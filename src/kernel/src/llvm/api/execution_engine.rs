use llvm_sys::execution_engine::LLVMExecutionEngineRef;

pub(super) struct LlvmExecutionEngine<'module> {
    pub(super) inner: LLVMExecutionEngineRef,
    pub(super) _marker: std::marker::PhantomData<&'module ()>,
}

impl<'module> LlvmExecutionEngine<'module> {}
