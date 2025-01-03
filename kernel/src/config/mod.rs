use core::ptr::null;
use macros::elf_note;
use obconf::{BootEnv, Config};

pub use self::arch::*;

#[cfg_attr(target_arch = "aarch64", path = "aarch64.rs")]
#[cfg_attr(target_arch = "x86_64", path = "x86_64.rs")]
mod arch;

/// # Interupt safety
/// This function is interupt safe.
pub fn boot_env() -> &'static BootEnv {
    // This function is not allowed to access the CPU context due to it can be called before the
    // context has been activated.
    // SAFETY: This is safe because the setup() requirements.
    unsafe { &*BOOT_ENV }
}

/// # Context safety
/// This function does not require a CPU context.
///
/// # Interrupt safety
/// This function can be called from interrupt handler.
pub fn config() -> &'static Config {
    // SAFETY: This is safe because the setup() requirements.
    unsafe { &*CONFIG }
}

/// # Safety
/// This function must be called immediately in the kernel entry point. After that it must never
/// be called again.
pub unsafe fn setup(env: &'static BootEnv, conf: &'static Config) {
    // The requirement of this function imply that it is not allowed to access the CPU context.
    BOOT_ENV = env;
    CONFIG = conf;
}

static mut BOOT_ENV: *const BootEnv = null();
static mut CONFIG: *const Config = null();

#[elf_note(section = ".note.obkrnl.page-size", name = "obkrnl", ty = 0)]
static NOTE_PAGE_SIZE: [u8; size_of::<usize>()] = PAGE_SIZE.get().to_ne_bytes();
