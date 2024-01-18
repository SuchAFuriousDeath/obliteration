use std::{ffi::CStr, fmt::Display, ops::Deref};

use libc::c_char;

/// A wrapper on LLVM error.
#[derive(Debug)]
pub struct LlvmRawError {
    message: Box<CStr>,
}

impl LlvmRawError {
    /// # Safety
    /// `message` must be pointed to a null-terminated string allocated with `malloc` or a
    /// compatible funtion because this method will free it with `free`.
    pub(super) unsafe fn new(message: *mut c_char) -> Self {
        let owned = CStr::from_ptr(message);

        Self {
            message: owned.into(),
        }
    }
}

impl Drop for LlvmRawError {
    fn drop(&mut self) {
        unsafe { libc::free(self.message.as_ptr() as _) };
    }
}

impl Display for LlvmRawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.deref())
    }
}

impl std::error::Error for LlvmRawError {}
