use llvm_sys::prelude::LLVMValueRef;

pub(super) struct LLVMValue<'builder> {
    inner: LLVMValueRef,
    _marker: std::marker::PhantomData<&'builder ()>,
}
