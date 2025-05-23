#![no_std]
#![cfg_attr(not(test), no_main)]

use self::config::Config;
use self::context::{ContextSetup, current_procmgr};
use self::imgact::Ps4Abi;
use self::malloc::KernelHeap;
use self::proc::{Fork, Proc, ProcAbi, ProcMgr, Thread};
use self::sched::sleep;
use self::uma::Uma;
use self::vm::Vm;
use alloc::sync::Arc;
use core::mem::zeroed;
use krt::info;

#[cfg_attr(target_arch = "aarch64", path = "aarch64.rs")]
#[cfg_attr(target_arch = "x86_64", path = "x86_64.rs")]
mod arch;
mod config;
mod context;
mod event;
mod imgact;
mod imgfmt;
mod lock;
mod malloc;
mod proc;
mod sched;
mod signal;
mod subsystem;
mod trap;
mod uma;
mod vm;

extern crate alloc;

/// This will be called by [`krt`] crate.
///
/// See Orbis kernel entry point for a reference.
#[cfg_attr(target_os = "none", unsafe(no_mangle))]
fn main(config: &'static ::config::Config) -> ! {
    // SAFETY: This function has a lot of restrictions. See Context documentation for more details.
    info!("Starting Obliteration Kernel.");

    // Setup the CPU after the first print to let the bootloader developer know (some of) their code
    // are working.
    let config = Config::new(config);
    let arch = unsafe { self::arch::setup_main_cpu() };

    // Setup proc0 to represent the kernel.
    let proc0 = Proc::new_bare(Arc::new(Proc0Abi));

    // Setup thread0 to represent this thread.
    let proc0 = Arc::new(proc0);
    let thread0 = Thread::new_bare(proc0);

    // Activate CPU context.
    let thread0 = Arc::new(thread0);

    unsafe { self::context::run_with_context(config, arch, 0, thread0, setup, run) };
}

fn setup() -> ContextSetup {
    // Run sysinit vector for subsystem. The Orbis use linker to put all sysinit functions in a list
    // then loop the list to execute all of it. We manually execute those functions instead for
    // readability. This also allow us to pass data from one function to another function. See
    // mi_startup function on the Orbis for a reference.
    let procs = ProcMgr::new();
    let uma = init_vm(); // 161 on PS4 11.00.

    ContextSetup { uma, pmgr: procs }
}

fn run() -> ! {
    // Activate stage 2 heap.
    info!("Activating stage 2 heap.");

    unsafe { KERNEL_HEAP.activate_stage2() };

    // Run remaining sysinit vector.
    create_init(); // 659 on PS4 11.00.
    swapper(); // 1119 on PS4 11.00.
}

/// See `vm_mem_init` function on the Orbis for a reference.
///
/// # Reference offsets
/// | Version | Offset |
/// |---------|--------|
/// |PS4 11.00|0x39A390|
fn init_vm() -> Arc<Uma> {
    // Initialize VM.
    let vm = Vm::new().unwrap();

    info!("Initial memory size: {}", vm.initial_memory_size());
    info!("Boot area          : {:#x}", vm.boot_area());

    // Initialize UMA.
    Uma::new(vm)
}

/// See `create_init` function on the Orbis for a reference.
///
/// # Reference offsets
/// | Version | Offset |
/// |---------|--------|
/// |PS4 11.00|0x2BEF30|
fn create_init() {
    let pmgr = current_procmgr().unwrap();
    let abi = Arc::new(Ps4Abi);
    let flags = Fork::CopyFd | Fork::CreateProcess;

    pmgr.fork(abi, flags).unwrap();

    todo!()
}

/// See `scheduler` function on the PS4 for a reference.
fn swapper() -> ! {
    // TODO: Subscribe to "system_suspend_phase2_pre_sync" and "system_resume_phase2" event.
    let procs = current_procmgr().unwrap();

    loop {
        // TODO: Implement a call to vm_page_count_min().
        let procs = procs.list();

        if procs.len() == 0 {
            // TODO: The PS4 check for some value for non-zero but it seems like that value always
            // zero.
            sleep();
            continue;
        }

        todo!();
    }
}

/// Implementation of [`ProcAbi`] for kernel process.
///
/// See `null_sysvec` on the PS4 for a reference.
struct Proc0Abi;

impl ProcAbi for Proc0Abi {
    /// See `null_fetch_syscall_args` on the PS4 for a reference.
    fn syscall_handler(&self) {
        unimplemented!()
    }
}

// SAFETY: PRIMITIVE_HEAP is a mutable static so it valid for reads and writes. This will be safe as
// long as no one access PRIMITIVE_HEAP.
#[allow(dead_code)]
#[cfg_attr(target_os = "none", global_allocator)]
static KERNEL_HEAP: KernelHeap = unsafe { KernelHeap::new(&raw mut PRIMITIVE_HEAP) };
static mut PRIMITIVE_HEAP: [u8; 1024 * 1024] = unsafe { zeroed() };
