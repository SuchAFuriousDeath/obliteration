use libc::c_char;
use std::ffi::CStr;
use std::fmt::Display;

/// A wrapper for an LLVM error.
#[derive(Debug)]
pub(super) struct LlvmRawError {
    inner: Box<str>,
}

impl LlvmRawError {
    /// # Safety
    /// `message` must be pointed to a null-terminated string allocated with `malloc` or a
    /// compatible funtion because this method will free it with `free`.
    pub(super) unsafe fn from_ptr(message: *mut c_char) -> Self {
        let inner = CStr::from_ptr(message)
            .to_string_lossy()
            .into_owned()
            .into_boxed_str();

        libc::free(message as _);

        Self { inner }
    }
}

impl Display for LlvmRawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl std::error::Error for LlvmRawError {}
