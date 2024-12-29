// SPDX-License-Identifier: MIT OR Apache-2.0
use self::arch::{GdbRegs, BREAKPOINT_SIZE};
use self::channel::{create_channel, MainStream, VmmStream};
use self::hw::{setup_devices, Device, DeviceTree};
use self::kernel::{
    Kernel, NoteError, PT_DYNAMIC, PT_GNU_EH_FRAME, PT_GNU_RELRO, PT_GNU_STACK, PT_LOAD, PT_NOTE,
    PT_PHDR,
};
use self::ram::{RamBuilder, RamMap};
use crate::gdb::DebugClient;
use crate::hv::{CpuDebug, CpuExit, CpuIo, CpuRun, CpuStates, Hypervisor, Ram};
use crate::profile::Profile;
use futures::FutureExt;
use gdbstub::common::{Signal, Tid};
use gdbstub::stub::MultiThreadStopReason;
use gdbstub::target::ext::base::multithread::{
    MultiThreadBase, MultiThreadResume, MultiThreadResumeOps,
};
use gdbstub::target::ext::thread_extra_info::{ThreadExtraInfo, ThreadExtraInfoOps};
use gdbstub::target::{TargetError, TargetResult};
use kernel::{KernelError, ProgramHeaderError};
use obconf::{BootEnv, ConsoleType, Vm};
use rustc_hash::FxHashMap;
use std::cmp::max;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::future::Future;
use std::num::NonZero;
use std::ops::Deref;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::thread::JoinHandle;
use thiserror::Error;

#[cfg_attr(target_arch = "aarch64", path = "aarch64.rs")]
#[cfg_attr(target_arch = "x86_64", path = "x86_64.rs")]
mod arch;
mod channel;
mod cpu;
mod hw;
mod kernel;
mod ram;

/// Manage a virtual machine that run the kernel.
pub struct Vmm<H> {
    hv: Arc<H>,
    main: Arc<MainStream>,
    devices: Arc<DeviceTree>,
    cpus: FxHashMap<usize, Cpu>,
    next: usize,
    breakpoint: Arc<Mutex<()>>,
    sw_breakpoints: HashMap<u64, [u8; BREAKPOINT_SIZE.get()]>,
    shutdown: Arc<AtomicBool>,
    events: VmmStream,
}

impl Vmm<()> {
    pub fn new(
        profile: &Profile,
        kernel: &Path,
        debugger: Option<DebugClient>,
        shutdown: &Arc<AtomicBool>,
    ) -> Result<Vmm<impl Hypervisor>, VmmError> {
        // Get program header enumerator.
        let mut img = Kernel::open(kernel).map_err(|e| VmmError::OpenKernel(e))?;
        let hdrs = img
            .program_headers()
            .map_err(|e| VmmError::EnumerateProgramHeaders(e))?;

        // Parse program headers.
        let mut segments = Vec::new();
        let mut dynamic = None;
        let mut note = None;

        for (index, item) in hdrs.enumerate() {
            let hdr = item.map_err(|e| VmmError::ReadProgramHeader(index, e))?;

            match hdr.p_type {
                PT_LOAD => {
                    if hdr.p_filesz > hdr.p_memsz {
                        return Err(VmmError::InvalidFilesz(index));
                    }

                    segments.push(hdr);
                }
                PT_DYNAMIC => {
                    if dynamic.is_some() {
                        return Err(VmmError::MultipleDynamic);
                    }

                    dynamic = Some(hdr);
                }
                PT_NOTE => {
                    if note.is_some() {
                        return Err(VmmError::MultipleNote);
                    }

                    note = Some(hdr);
                }
                PT_PHDR | PT_GNU_EH_FRAME | PT_GNU_STACK | PT_GNU_RELRO => (),
                v => return Err(VmmError::UnknownProgramHeaderType(v, index)),
            }
        }

        segments.sort_unstable_by_key(|i| i.p_vaddr);

        // Make sure the first PT_LOAD includes the ELF header.
        let hdr = segments.first().ok_or(VmmError::NoLoadSegment)?;

        if hdr.p_offset != 0 {
            return Err(VmmError::ElfHeaderNotInFirstLoadSegment);
        }

        // Check if PT_DYNAMIC and PT_NOTE exists.
        let dynamic = dynamic.ok_or(VmmError::NoDynamicSegment)?;
        let note = note.ok_or(VmmError::NoNoteSegment)?;

        // Parse PT_NOTE.
        let mut vm_page_size = None;

        if note.p_filesz > 1024 * 1024 {
            return Err(VmmError::NoteSegmentTooLarge);
        }

        for (i, note) in img.notes(&note).map_err(VmmError::SeekToNote)?.enumerate() {
            let note = note.map_err(move |e| VmmError::ReadKernelNote(i, e))?;

            if note.name.as_ref() != b"obkrnl" {
                continue;
            }

            match note.ty {
                0 => {
                    if vm_page_size.is_some() {
                        return Err(VmmError::DuplicateKernelNote(i));
                    }

                    vm_page_size = note
                        .desc
                        .as_ref()
                        .try_into()
                        .map(usize::from_ne_bytes)
                        .ok()
                        .and_then(NonZero::new)
                        .filter(|v| v.is_power_of_two());

                    if vm_page_size.is_none() {
                        return Err(VmmError::InvalidNoteDescription(i));
                    }
                }
                v => return Err(VmmError::UnknownKernelNoteType(v, i)),
            }
        }

        // Check if required notes exists.
        let vm_page_size = vm_page_size.ok_or(VmmError::NoPageSizeInKernelNote)?;

        // Get kernel memory size.
        let mut len = 0;

        for hdr in &segments {
            if hdr.p_vaddr < len {
                return Err(VmmError::OverlappedLoadSegment(hdr.p_vaddr));
            }

            len = hdr
                .p_vaddr
                .checked_add(hdr.p_memsz)
                .ok_or(VmmError::InvalidPmemsz(hdr.p_vaddr))?;
        }

        // Round kernel memory size.
        let host_page_size = Self::get_page_size().map_err(VmmError::GetHostPageSize)?;
        let block_size = max(vm_page_size, host_page_size);
        let len = NonZero::new(len)
            .ok_or(VmmError::ZeroLengthLoadSegment)?
            .get()
            .checked_next_multiple_of(block_size.get())
            .ok_or(VmmError::TotalSizeTooLarge)?;

        // Setup RAM.
        let ram_size = NonZero::new(1024 * 1024 * 1024 * 8).unwrap();

        // Setup virtual devices.
        let devices = Arc::new(setup_devices(ram_size.get(), block_size));

        // Setup hypervisor.
        let mut hv = unsafe { crate::hv::new(8, ram_size, block_size, debugger.is_some()) }
            .map_err(VmmError::SetupHypervisor)?;

        // Map the kernel.
        let feats = hv.cpu_features().clone();
        let mut ram = RamBuilder::new(hv.ram_mut());

        let kern = ram
            .alloc_kernel(NonZero::new(len).unwrap())
            .map_err(VmmError::AllocateRamForKernel)?;

        for hdr in &segments {
            let mut src = img
                .segment_data(hdr)
                .map_err(|e| VmmError::SeekToOffset(hdr.p_offset, e))?;
            let mut dst = &mut kern[hdr.p_vaddr..(hdr.p_vaddr + hdr.p_memsz)];

            match std::io::copy(&mut src, &mut dst) {
                Ok(v) => {
                    if v != u64::try_from(hdr.p_filesz).unwrap() {
                        return Err(VmmError::IncompleteKernel);
                    }
                }
                Err(e) => return Err(VmmError::ReadKernel(e, hdr.p_offset)),
            }
        }

        ram.alloc_stack(NonZero::new(1024 * 1024 * 2).unwrap())
            .map_err(VmmError::AllocateRamForStack)?;

        // Allocate arguments.
        let env = BootEnv::Vm(Vm {
            vmm: devices.vmm().addr(),
            console: devices.console().addr(),
            host_page_size,
        });

        ram.alloc_args(env, profile.kernel_config().clone())
            .map_err(VmmError::AllocateRamForArgs)?;

        // Build RAM.
        let map = ram
            .build(&feats, vm_page_size, &devices, dynamic)
            .map_err(VmmError::BuildRam)?;

        // Spawn main CPU.
        let (events, main) = create_channel();
        let mut vmm = Vmm {
            hv: Arc::new(hv),
            main: Arc::new(main),
            devices,
            cpus: FxHashMap::default(),
            next: 0,
            breakpoint: Arc::default(),
            sw_breakpoints: HashMap::new(),
            shutdown: shutdown.clone(),
            events,
        };

        vmm.spawn(map.kern_vaddr + img.entry(), Some(map), debugger.is_some());

        Ok(vmm)
    }
}

impl<H> Vmm<H> {
    pub fn recv(&mut self) -> impl Future<Output = VmmEvent> + '_ {
        std::future::poll_fn(|cx| {
            for (&id, cpu) in &mut self.cpus {
                // The sender side will never close without sending the value.
                if cpu.exiting.poll_unpin(cx).is_ready() {
                    let c = self.cpus.remove(&id).unwrap();
                    let r = c.thread.join().unwrap();

                    return Poll::Ready(VmmEvent::Exit(id, r));
                }
            }

            Poll::Pending
        })
    }

    pub fn lock(&mut self) {
        for cpu in self.cpus.values_mut() {
            cpu.debug.as_mut().unwrap().lock();
        }
    }

    pub fn release(&mut self) {
        for cpu in self.cpus.values_mut() {
            cpu.debug.as_mut().unwrap().release();
        }
    }

    #[cfg(unix)]
    fn get_page_size() -> Result<NonZero<usize>, std::io::Error> {
        let v = unsafe { libc::sysconf(libc::_SC_PAGE_SIZE) };

        if v < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(v.try_into().ok().and_then(NonZero::new).unwrap())
        }
    }

    #[cfg(windows)]
    fn get_page_size() -> Result<NonZero<usize>, std::io::Error> {
        use std::mem::zeroed;
        use windows_sys::Win32::System::SystemInformation::GetSystemInfo;
        let mut i = unsafe { zeroed() };

        unsafe { GetSystemInfo(&mut i) };

        Ok(i.dwPageSize.try_into().ok().and_then(NonZero::new).unwrap())
    }
}

impl<H: Hypervisor> Vmm<H> {
    const GDB_ENOENT: u8 = 2;
    const GDB_EFAULT: u8 = 14;

    pub fn spawn(&mut self, start: usize, map: Option<RamMap>, debug: bool) {
        // Setup arguments.
        let args = CpuArgs {
            hv: self.hv.clone(),
            main: self.main.clone(),
            devices: self.devices.clone(),
            breakpoint: self.breakpoint.clone(),
            shutdown: self.shutdown.clone(),
        };

        // Setup debug channel.
        let (debug, debugger) = if debug {
            Some(self::cpu::debug::channel()).unzip()
        } else {
            None.unzip()
        };

        // Spawn thread to drive vCPU.
        let id = self.next;
        let (tx, exiting) = futures::channel::oneshot::channel();
        let thread = match map {
            Some(map) => std::thread::spawn(move || {
                let r = Self::main_cpu(args, debugger, start, map);
                tx.send(()).unwrap();
                r
            }),
            None => todo!(),
        };

        self.next += 1;

        assert!(self
            .cpus
            .insert(
                id,
                Cpu {
                    thread,
                    exiting,
                    debug,
                },
            )
            .is_none());
    }

    fn main_cpu(
        args: CpuArgs<H>,
        debug: Option<self::cpu::debug::Debugger>,
        entry: usize,
        map: RamMap,
    ) -> Result<bool, CpuError> {
        // Create CPU.
        let mut cpu = match args.hv.create_cpu(0) {
            Ok(v) => v,
            Err(e) => return Err(CpuError::Create(Box::new(e))),
        };

        if let Err(e) = self::arch::setup_main_cpu(&mut cpu, entry, map, args.hv.cpu_features()) {
            return Err(CpuError::Setup(Box::new(e)));
        }

        // Wait for debugger.
        if let Some(debug) = &debug {
            if let Some(v) = Self::handle_breakpoint(&args, debug, &mut cpu, None)? {
                return Ok(v);
            }
        }

        // Run.
        Self::run_cpu(&args, debug, cpu)
    }

    fn run_cpu<'c>(
        args: &'c CpuArgs<H>,
        debug: Option<self::cpu::debug::Debugger>,
        mut cpu: H::Cpu<'c>,
    ) -> Result<bool, CpuError> {
        // Build device contexts for this CPU.
        let hv = args.hv.deref();
        let main = args.main.as_ref();
        let mut devices = BTreeMap::<usize, self::cpu::Device<'c, H::Cpu<'c>>>::new();
        let t = &args.devices;

        self::cpu::Device::insert(&mut devices, t.console(), |d| d.create_context(hv, main));
        self::cpu::Device::insert(&mut devices, t.vmm(), |d| d.create_context());

        // Dispatch CPU events until shutdown.
        loop {
            // Check for shutdown signal.
            if args.shutdown.load(Ordering::Relaxed) {
                return Ok(true);
            }

            // Run the vCPU.
            let mut exit = match cpu.run() {
                Ok(v) => v,
                Err(e) => return Err(CpuError::Run(Box::new(e))),
            };

            // Execute VM exited event.
            for d in devices.values_mut() {
                match d.context.exited(exit.cpu()) {
                    Ok(Some(v)) => return Ok(v),
                    Ok(None) => (),
                    Err(e) => return Err(CpuError::DeviceExitHandler(d.name.to_owned(), e)),
                }
            }

            // Handle exit.
            if let Some(v) = Self::handle_exit(args, debug.as_ref(), &mut devices, exit)? {
                return Ok(v);
            }

            // Execute post exit event.
            for d in devices.values_mut() {
                match d.context.post(&mut cpu) {
                    Ok(Some(v)) => return Ok(v),
                    Ok(None) => (),
                    Err(e) => return Err(CpuError::DevicePostExitHandler(d.name.to_owned(), e)),
                }
            }
        }
    }

    fn handle_exit<'c, C: crate::hv::Cpu>(
        args: &'c CpuArgs<H>,
        debugger: Option<&self::cpu::debug::Debugger>,
        devices: &mut BTreeMap<usize, self::cpu::Device<'c, C>>,
        exit: C::Exit<'_>,
    ) -> Result<Option<bool>, CpuError> {
        // Check if HLT.
        #[cfg(target_arch = "x86_64")]
        let exit = match exit.into_hlt() {
            Ok(_) => return Ok(None),
            Err(v) => v,
        };

        // Check if I/O.
        let exit = match exit.into_io() {
            Ok(io) => return Self::handle_io(devices, io),
            Err(v) => v,
        };

        // Check if debug.
        match exit.into_debug() {
            Ok(mut debug) => {
                let reason = debug.reason();

                if let Some(debugger) = debugger {
                    Self::handle_breakpoint(args, debugger, debug.cpu(), Some(reason))
                } else {
                    todo!()
                }
            }
            Err(_) => todo!(),
        }
    }

    fn handle_io<C: crate::hv::Cpu>(
        devices: &mut BTreeMap<usize, self::cpu::Device<'_, C>>,
        mut io: <C::Exit<'_> as CpuExit>::Io,
    ) -> Result<Option<bool>, CpuError> {
        // Get target device.
        let addr = io.addr();
        let dev = match devices
            .range_mut(..=addr)
            .last()
            .map(|v| v.1)
            .filter(move |d| addr < d.end.get())
        {
            Some(v) => v,
            None => return Err(CpuError::MmioAddr(addr)),
        };

        // Execute.
        dev.context
            .mmio(&mut io)
            .map_err(|e| CpuError::Mmio(dev.name.to_owned(), e))
    }

    fn handle_breakpoint(
        args: &CpuArgs<H>,
        debug: &self::cpu::debug::Debugger,
        cpu: &mut impl crate::hv::Cpu,
        stop: Option<MultiThreadStopReason<u64>>,
    ) -> Result<Option<bool>, CpuError> {
        // Notify GUI. We need to allow only one CPU to enter the debugger dispatch loop.
        let lock = args.breakpoint.lock().unwrap();

        args.main.breakpoint(stop);

        // Wait for command from debugger thread.
        loop {
            let req = match debug.recv() {
                Some(v) => v,
                None => return Ok(Some(true)),
            };

            match req {
                self::cpu::debug::DebugReq::GetRegs => {
                    // Get states.
                    let mut states = match cpu.states() {
                        Ok(v) => v,
                        Err(e) => return Err(CpuError::GetStates(Box::new(e))),
                    };

                    debug.send(self::cpu::debug::DebugRes::Regs(Self::get_debug_regs(
                        &mut states,
                    )?));
                }
                self::cpu::debug::DebugReq::TranslateAddress(addr) => match cpu.translate(addr) {
                    Ok(v) => debug.send(self::cpu::debug::DebugRes::TranslatedAddress(v)),
                    Err(e) => return Err(CpuError::TranslateAddr(addr, Box::new(e))),
                },
                self::cpu::debug::DebugReq::Lock => {} // We already in a locked loop.
                self::cpu::debug::DebugReq::Release => break,
            }
        }

        drop(lock);

        Ok(None)
    }

    #[cfg(target_arch = "aarch64")]
    fn get_debug_regs(_: &mut impl CpuStates) -> Result<GdbRegs, CpuError> {
        todo!()
    }

    #[cfg(target_arch = "x86_64")]
    fn get_debug_regs<C: CpuStates>(states: &mut C) -> Result<GdbRegs, CpuError> {
        use gdbstub_arch::x86::reg::{X86SegmentRegs, X87FpuInternalRegs};

        let error = |n: &'static str, e: C::Err| CpuError::ReadReg(n, Box::new(e));
        let mut load_greg = |name: &'static str, func: fn(&mut C) -> Result<usize, C::Err>| {
            func(states)
                .map(|v| TryInto::<u64>::try_into(v).unwrap())
                .map_err(|e| error(name, e))
        };

        Ok(GdbRegs {
            regs: [
                load_greg("rax", |s| s.get_rax())?,
                load_greg("rbx", |s| s.get_rbx())?,
                load_greg("rcx", |s| s.get_rcx())?,
                load_greg("rdx", |s| s.get_rdx())?,
                load_greg("rsi", |s| s.get_rsi())?,
                load_greg("rdi", |s| s.get_rdi())?,
                load_greg("rbp", |s| s.get_rbp())?,
                load_greg("rsp", |s| s.get_rsp())?,
                load_greg("r8", |s| s.get_r8())?,
                load_greg("r9", |s| s.get_r9())?,
                load_greg("r10", |s| s.get_r10())?,
                load_greg("r11", |s| s.get_r11())?,
                load_greg("r12", |s| s.get_r12())?,
                load_greg("r13", |s| s.get_r13())?,
                load_greg("r14", |s| s.get_r14())?,
                load_greg("r15", |s| s.get_r15())?,
            ],
            rip: load_greg("rip", |s| s.get_rip())?,
            eflags: states
                .get_rflags()
                .map(|v| v.into_bits().try_into().unwrap())
                .map_err(|e| error("rflags", e))?,
            segments: X86SegmentRegs {
                cs: states.get_cs().map_err(|e| error("cs", e))?.into(),
                ss: states.get_ss().map_err(|e| error("ss", e))?.into(),
                ds: states.get_ds().map_err(|e| error("ds", e))?.into(),
                es: states.get_es().map_err(|e| error("es", e))?.into(),
                fs: states.get_fs().map_err(|e| error("fs", e))?.into(),
                gs: states.get_gs().map_err(|e| error("gs", e))?.into(),
            },
            st: [
                states.get_st0().map_err(|e| error("st0", e))?,
                states.get_st1().map_err(|e| error("st1", e))?,
                states.get_st2().map_err(|e| error("st2", e))?,
                states.get_st3().map_err(|e| error("st3", e))?,
                states.get_st4().map_err(|e| error("st4", e))?,
                states.get_st5().map_err(|e| error("st5", e))?,
                states.get_st6().map_err(|e| error("st6", e))?,
                states.get_st7().map_err(|e| error("st7", e))?,
            ],
            fpu: X87FpuInternalRegs {
                fctrl: states.get_fcw().map_err(|e| error("fcw", e))?,
                fstat: states.get_fsw().map_err(|e| error("fsw", e))?,
                ftag: states.get_ftwx().map_err(|e| error("ftwx", e))?,
                fiseg: states.get_fiseg().map_err(|e| error("fiseg", e))?,
                fioff: states.get_fioff().map_err(|e| error("fioff", e))?,
                foseg: states.get_foseg().map_err(|e| error("foseg", e))?,
                fooff: states.get_fooff().map_err(|e| error("fooff", e))?,
                fop: states.get_fop().map_err(|e| error("fop", e))?,
            },
            xmm: [
                states.get_xmm0().map_err(|e| error("xmm0", e))?,
                states.get_xmm1().map_err(|e| error("xmm1", e))?,
                states.get_xmm2().map_err(|e| error("xmm2", e))?,
                states.get_xmm3().map_err(|e| error("xmm3", e))?,
                states.get_xmm4().map_err(|e| error("xmm4", e))?,
                states.get_xmm5().map_err(|e| error("xmm5", e))?,
                states.get_xmm6().map_err(|e| error("xmm6", e))?,
                states.get_xmm7().map_err(|e| error("xmm7", e))?,
                states.get_xmm8().map_err(|e| error("xmm8", e))?,
                states.get_xmm9().map_err(|e| error("xmm9", e))?,
                states.get_xmm10().map_err(|e| error("xmm10", e))?,
                states.get_xmm11().map_err(|e| error("xmm11", e))?,
                states.get_xmm12().map_err(|e| error("xmm12", e))?,
                states.get_xmm13().map_err(|e| error("xmm13", e))?,
                states.get_xmm14().map_err(|e| error("xmm14", e))?,
                states.get_xmm15().map_err(|e| error("xmm15", e))?,
            ],
            mxcsr: states.get_mxcsr().map_err(|e| error("mxcsr", e))?,
        })
    }

    #[cfg(target_arch = "aarch64")]
    fn set_debug_regs(_: &mut impl CpuStates, _: GdbRegs) -> Result<(), CpuError> {
        todo!()
    }

    #[cfg(target_arch = "x86_64")]
    fn set_debug_regs(_: &mut impl CpuStates, _: GdbRegs) -> Result<(), CpuError> {
        todo!()
    }
}

impl<H> Drop for Vmm<H> {
    fn drop(&mut self) {
        // Set shutdown flag before dropping the other fields so their background thread can stop
        // before they try to join with it.
        self.shutdown.store(true, Ordering::Relaxed);

        // Wait for all CPU to stop.
        for (_, cpu) in self.cpus.drain() {
            // We need to drop the debug channel first so it will unblock the CPU thread if it is
            // waiting for a request.
            drop(cpu.debug);
            drop(cpu.thread.join().unwrap());
        }
    }
}

impl<H: Hypervisor> MultiThreadBase for Vmm<H> {
    fn read_registers(&mut self, regs: &mut GdbRegs, tid: Tid) -> TargetResult<(), Self> {
        let cpu = self
            .cpus
            .get_mut(&(tid.get() - 1))
            .ok_or(TargetError::Errno(Self::GDB_ENOENT))?;

        *regs = cpu
            .debug
            .as_mut()
            .unwrap()
            .get_regs()
            .ok_or(TargetError::Errno(Self::GDB_ENOENT))?; // The CPU thread just stopped.

        Ok(())
    }

    fn write_registers(&mut self, regs: &GdbRegs, tid: Tid) -> TargetResult<(), Self> {
        todo!()
    }

    fn read_addrs(
        &mut self,
        start_addr: u64,
        data: &mut [u8],
        tid: Tid,
    ) -> TargetResult<usize, Self> {
        let Some(len) = NonZero::new(data.len()) else {
            return Ok(0);
        };

        // Translate virtual address to physical address.
        let cpu = self
            .cpus
            .get_mut(&(tid.get() - 1))
            .ok_or(TargetError::Errno(Self::GDB_ENOENT))?;

        let addr = cpu
            .debug
            .as_mut()
            .unwrap()
            .translate_address(start_addr.try_into().unwrap())
            .ok_or(TargetError::Errno(Self::GDB_ENOENT))?;

        // Get data.
        let src = self
            .hv
            .ram()
            .lock(addr, len)
            .ok_or(TargetError::Errno(Self::GDB_EFAULT))?;

        data.copy_from_slice(unsafe { std::slice::from_raw_parts(src.as_ptr(), src.len().get()) });

        Ok(len.get())
    }

    fn write_addrs(&mut self, start_addr: u64, data: &[u8], tid: Tid) -> TargetResult<(), Self> {
        todo!()
    }

    fn is_thread_alive(&mut self, tid: Tid) -> Result<bool, Self::Error> {
        todo!()
    }

    fn list_active_threads(
        &mut self,
        thread_is_active: &mut dyn FnMut(Tid),
    ) -> Result<(), Self::Error> {
        for id in (0..self.cpus.len()).map(|v| NonZero::new(v + 1).unwrap()) {
            thread_is_active(id);
        }

        Ok(())
    }

    #[inline(always)]
    fn support_resume(&mut self) -> Option<MultiThreadResumeOps<'_, Self>> {
        Some(self)
    }

    #[inline(always)]
    fn support_thread_extra_info(&mut self) -> Option<ThreadExtraInfoOps<'_, Self>> {
        Some(self)
    }
}

impl<H: Hypervisor> ThreadExtraInfo for Vmm<H> {
    fn thread_extra_info(&self, tid: Tid, buf: &mut [u8]) -> Result<usize, Self::Error> {
        todo!()
    }
}

impl<H: Hypervisor> MultiThreadResume for Vmm<H> {
    fn resume(&mut self) -> Result<(), Self::Error> {
        self.release();

        Ok(())
    }

    fn clear_resume_actions(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_resume_action_continue(
        &mut self,
        tid: Tid,
        signal: Option<Signal>,
    ) -> Result<(), Self::Error> {
        if let Some(signal) = signal {
            todo!("set_resume_action_continue with signal {signal:?}");
        }

        Ok(())
    }
}

/// Contains objects to control a CPU from outside.
struct Cpu {
    thread: JoinHandle<Result<bool, CpuError>>,
    exiting: futures::channel::oneshot::Receiver<()>,
    debug: Option<self::cpu::debug::Debuggee>,
}

/// Encapsulates arguments for a function to run a CPU.
struct CpuArgs<H> {
    hv: Arc<H>,
    main: Arc<MainStream>,
    devices: Arc<DeviceTree>,
    breakpoint: Arc<Mutex<()>>,
    shutdown: Arc<AtomicBool>,
}

/// Event from VMM.
pub enum VmmEvent {
    Exit(usize, Result<bool, CpuError>),
    Log(ConsoleType, String),
    Breakpoint(Option<MultiThreadStopReason<u64>>),
}

/// Represents an error when [`Vmm::new()`] fails.
#[derive(Debug, Error)]
pub enum VmmError {
    #[error("couldn't open the kernel")]
    OpenKernel(#[source] KernelError),

    #[error("couldn't start enumerating program headers")]
    EnumerateProgramHeaders(#[source] std::io::Error),

    #[error("couldn't read program header #{0}")]
    ReadProgramHeader(usize, #[source] ProgramHeaderError),

    #[error("invalid p_filesz on on PT_LOAD {0}")]
    InvalidFilesz(usize),

    #[error("multiple PT_DYNAMIC is not supported")]
    MultipleDynamic,

    #[error("multiple PT_NOTE is not supported")]
    MultipleNote,

    #[error("unknown p_type {0} on program header {1}")]
    UnknownProgramHeaderType(u32, usize),

    #[error("the first PT_LOAD does not include ELF header")]
    ElfHeaderNotInFirstLoadSegment,

    #[error("no PT_LOAD on the kernel")]
    NoLoadSegment,

    #[error("no PT_DYNAMIC on the kernel")]
    NoDynamicSegment,

    #[error("no PT_NOTE on the kernel")]
    NoNoteSegment,

    #[error("PT_NOTE is too large")]
    NoteSegmentTooLarge,

    #[error("couldn't seek to PT_NOTE")]
    SeekToNote(#[source] std::io::Error),

    #[error("couldn't read kernel note #{0}")]
    ReadKernelNote(usize, #[source] NoteError),

    #[error("invalid description on kernel note #{0}")]
    InvalidNoteDescription(usize),

    #[error("kernel note #{0} is duplicated")]
    DuplicateKernelNote(usize),

    #[error("unknown type {0} on kernel note #{1}")]
    UnknownKernelNoteType(u32, usize),

    #[error("no page size in kernel note")]
    NoPageSizeInKernelNote,

    #[error("couldn't get host page size")]
    GetHostPageSize(#[source] std::io::Error),

    #[error("PT_LOAD at {0:#} is overlapped with the previous PT_LOAD")]
    OverlappedLoadSegment(usize),

    #[error("invalid p_memsz on PT_LOAD at {0:#}")]
    InvalidPmemsz(usize),

    #[error("the kernel has PT_LOAD with zero length")]
    ZeroLengthLoadSegment,

    #[error("total size of PT_LOAD is too large")]
    TotalSizeTooLarge,

    #[error("couldn't setup a hypervisor")]
    SetupHypervisor(#[source] crate::hv::HypervisorError),

    #[error("couldn't allocate RAM for the kernel")]
    AllocateRamForKernel(#[source] crate::hv::RamError),

    #[error("couldn't seek to offset {0:#x}")]
    SeekToOffset(u64, #[source] std::io::Error),

    #[error("the kernel is incomplete")]
    IncompleteKernel,

    #[error("couldn't read kernel at offset {1}")]
    ReadKernel(#[source] std::io::Error, u64),

    #[error("couldn't allocate RAM for stack")]
    AllocateRamForStack(#[source] crate::hv::RamError),

    #[error("couldn't allocate RAM for arguments")]
    AllocateRamForArgs(#[source] crate::hv::RamError),

    #[error("couldn't build RAM")]
    BuildRam(#[source] ram::RamBuilderError),
}

/// Represents an error when a vCPU fails.
#[derive(Debug, Error)]
pub enum CpuError {
    #[error("couldn't create vCPU")]
    Create(#[source] Box<dyn Error + Send + Sync>),

    #[error("couldn't setup vCPU")]
    Setup(#[source] Box<dyn Error + Send + Sync>),

    #[error("couldn't run vCPU")]
    Run(#[source] Box<dyn Error + Send + Sync>),

    #[error("couldn't execute a VM exited event on a {0}")]
    DeviceExitHandler(String, #[source] Box<dyn Error + Send + Sync>),

    #[error("the vCPU attempt to execute a memory-mapped I/O on a non-mapped address {0:#x}")]
    MmioAddr(usize),

    #[error("couldn't execute a memory-mapped I/O on a {0}")]
    Mmio(String, #[source] Box<dyn Error + Send + Sync>),

    #[error("couldn't get vCPU states")]
    GetStates(#[source] Box<dyn Error + Send + Sync>),

    #[error("couldn't read {0} register")]
    ReadReg(&'static str, #[source] Box<dyn Error + Send + Sync>),

    #[error("couldn't translate address {0:#x}")]
    TranslateAddr(usize, #[source] Box<dyn Error + Send + Sync>),

    #[error("couldn't execute a post VM exit on a {0}")]
    DevicePostExitHandler(String, #[source] Box<dyn Error + Send + Sync>),
}

/// Represents an error when [`main_cpu()`] fails to reach event loop.
#[derive(Debug, Error)]
enum MainCpuError {
    #[error("couldn't get vCPU states")]
    GetCpuStatesFailed(#[source] Box<dyn Error + Send + Sync>),

    #[cfg(target_arch = "aarch64")]
    #[error("vCPU does not support {0:#x} page size")]
    PageSizeNotSupported(NonZero<usize>),

    #[cfg(target_arch = "aarch64")]
    #[error("physical address supported by vCPU too small")]
    PhysicalAddressTooSmall,

    #[error("couldn't commit vCPU states")]
    CommitCpuStatesFailed(#[source] Box<dyn Error + Send + Sync>),
}
