// SPDX-License-Identifier: MIT OR Apache-2.0
use super::arch::{KvmStates, StatesError};
use super::ffi::{KVM_EXIT_DEBUG, KVM_EXIT_HLT, KVM_EXIT_IO, KVM_RUN};
use super::run::KvmRun;
use crate::{Cpu, CpuDebug, CpuExit, CpuIo, CpuRun, DebugEvent, IoBuf};
use libc::{ioctl, munmap};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::MutexGuard;

/// Implementation of [`Cpu`] for KVM.
pub struct KvmCpu<'a> {
    id: usize,
    fd: MutexGuard<'a, OwnedFd>,
    cx: (*mut KvmRun, usize),
}

impl<'a> KvmCpu<'a> {
    /// # Safety
    /// - `cx` cannot be null and must be obtained from `mmap` on `fd`.
    /// - `len` must be the same value that used on `mmap`.
    pub unsafe fn new(id: usize, fd: MutexGuard<'a, OwnedFd>, cx: *mut KvmRun, len: usize) -> Self {
        assert!(len >= size_of::<KvmRun>());

        Self {
            id,
            fd,
            cx: (cx, len),
        }
    }
}

impl Drop for KvmCpu<'_> {
    fn drop(&mut self) {
        use std::io::Error;

        if unsafe { munmap(self.cx.0.cast(), self.cx.1) } < 0 {
            panic!("failed to munmap kvm_run: {}", Error::last_os_error());
        };
    }
}

impl<'a> Cpu for KvmCpu<'a> {
    type States<'b>
        = KvmStates<'b>
    where
        Self: 'b;
    type GetStatesErr = StatesError;
    type Exit<'b>
        = KvmExit<'b, 'a>
    where
        Self: 'b;
    type TranslateErr = std::io::Error;

    fn id(&self) -> usize {
        self.id
    }

    fn states(&mut self) -> Result<Self::States<'_>, Self::GetStatesErr> {
        KvmStates::from_cpu(&mut self.fd)
    }

    #[cfg(target_arch = "aarch64")]
    fn translate(&self, vaddr: usize) -> Result<usize, std::io::Error> {
        todo!()
    }

    #[cfg(target_arch = "x86_64")]
    fn translate(&self, vaddr: usize) -> Result<usize, std::io::Error> {
        use super::ffi::{KVM_TRANSLATE, KvmTranslation};

        let mut data = KvmTranslation {
            linear_address: vaddr,
            physical_address: 0,
            valid: 0,
            writeable: 0,
            usermode: 0,
            pad: [0; 5],
        };

        match unsafe { ioctl(self.fd.as_raw_fd(), KVM_TRANSLATE, &mut data) } {
            0 => Ok(data.physical_address),
            _ => Err(std::io::Error::last_os_error()),
        }
    }
}

impl CpuRun for KvmCpu<'_> {
    type RunErr = std::io::Error;

    fn run(&mut self) -> Result<Self::Exit<'_>, Self::RunErr> {
        if unsafe { ioctl(self.fd.as_raw_fd(), KVM_RUN, 0) } < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(KvmExit(self))
        }
    }
}

/// Implementation of [`Cpu::Exit`] for KVM.
pub struct KvmExit<'a, 'b>(&'a mut KvmCpu<'b>);

impl<'a, 'b> CpuExit for KvmExit<'a, 'b> {
    type Cpu = KvmCpu<'b>;
    type Io = KvmIo<'a, 'b>;
    type Debug = KvmDebug<'a, 'b>;

    fn cpu(&mut self) -> &mut Self::Cpu {
        self.0
    }

    #[cfg(target_arch = "x86_64")]
    fn into_hlt(self) -> Result<(), Self> {
        if unsafe { (*self.0.cx.0).exit_reason == KVM_EXIT_HLT } {
            Ok(())
        } else {
            Err(self)
        }
    }

    fn into_io(self) -> Result<Self::Io, Self> {
        if unsafe { (*self.0.cx.0).exit_reason } == KVM_EXIT_IO {
            Ok(KvmIo(self.0))
        } else {
            Err(self)
        }
    }

    fn into_debug(self) -> Result<Self::Debug, Self> {
        if unsafe { (*self.0.cx.0).exit_reason } == KVM_EXIT_DEBUG {
            Ok(KvmDebug(self.0))
        } else {
            Err(self)
        }
    }
}

/// Implementation of [`CpuIo`] for KVM.
pub struct KvmIo<'a, 'b>(&'a mut KvmCpu<'b>);

impl<'b> CpuIo for KvmIo<'_, 'b> {
    type Cpu = KvmCpu<'b>;

    fn addr(&self) -> usize {
        unsafe { (*self.0.cx.0).exit.mmio.phys_addr }
    }

    fn buffer(&mut self) -> IoBuf {
        let io = unsafe { &mut (*self.0.cx.0).exit.mmio };
        let len: usize = io.len.try_into().unwrap();
        let buf = &mut io.data[..len];

        match io.is_write {
            0 => IoBuf::Read(buf),
            _ => IoBuf::Write(buf),
        }
    }

    fn cpu(&mut self) -> &mut Self::Cpu {
        self.0
    }
}

/// Implementation of [`CpuDebug`] for KVM.
pub struct KvmDebug<'a, 'b>(&'a mut KvmCpu<'b>);

impl<'b> CpuDebug for KvmDebug<'_, 'b> {
    type Cpu = KvmCpu<'b>;

    fn reason(&mut self) -> DebugEvent {
        let debug = unsafe { (*self.0.cx.0).exit.debug.arch };

        match debug.exception {
            3 => DebugEvent::SwBreak,
            exception => todo!("unhandled exception {exception}"),
        }
    }

    fn cpu(&mut self) -> &mut Self::Cpu {
        self.0
    }
}
