// SPDX-License-Identifier: MIT OR Apache-2.0
pub use self::controller::DebugStates;

use self::controller::CpuController;
use super::hv::{Cpu, CpuExit, CpuIo, CpuRun, Hypervisor};
use super::hw::{DeviceContext, DeviceTree};
use super::ram::RamMap;
use super::screen::Screen;
use super::{VmmEvent, VmmEventHandler};
use crate::error::RustError;
use std::collections::BTreeMap;
use std::num::NonZero;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[cfg_attr(target_arch = "aarch64", path = "aarch64.rs")]
#[cfg_attr(target_arch = "x86_64", path = "x86_64.rs")]
mod arch;
mod controller;

/// Manage all virtual CPUs.
pub struct CpuManager<H: Hypervisor, S: Screen> {
    hv: Arc<H>,
    screen: Arc<S::Buffer>,
    devices: Arc<DeviceTree>,
    event: VmmEventHandler,
    cpus: Vec<CpuController>,
    shutdown: Arc<AtomicBool>,
}

impl<H: Hypervisor, S: Screen> CpuManager<H, S> {
    pub fn new(
        hv: Arc<H>,
        screen: Arc<S::Buffer>,
        devices: Arc<DeviceTree>,
        event: VmmEventHandler,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            hv,
            screen,
            devices,
            event,
            cpus: Vec::new(),
            shutdown,
        }
    }

    pub fn spawn(&mut self, start: usize, map: Option<RamMap>) {
        // Setup arguments.
        let args = Args {
            hv: self.hv.clone(),
            screen: self.screen.clone(),
            devices: self.devices.clone(),
            event: self.event,
            shutdown: self.shutdown.clone(),
        };

        // Spawn thread to drive vCPU.
        let debug = Arc::new((Mutex::default(), Condvar::new()));
        let t = match map {
            Some(map) => std::thread::spawn({
                let debug = debug.clone();

                move || Self::main_cpu(args, debug, start, map)
            }),
            None => todo!(),
        };

        self.cpus.push(CpuController::new(t, debug));
    }

    pub fn debug_lock(&mut self) -> DebugLock<H, S> {
        DebugLock(self)
    }

    fn main_cpu(
        args: Args<H, S>,
        debug: Arc<(Mutex<DebugStates>, Condvar)>,
        entry: usize,
        map: RamMap,
    ) {
        let mut cpu = match args.hv.create_cpu(0) {
            Ok(v) => v,
            Err(e) => {
                let e = RustError::with_source("couldn't create main CPU", e);
                unsafe { args.event.invoke(VmmEvent::Error { reason: &e }) };
                return;
            }
        };

        if let Err(e) = super::arch::setup_main_cpu(&mut cpu, entry, map, args.hv.cpu_features()) {
            let e = RustError::with_source("couldn't setup main CPU", e);
            unsafe { args.event.invoke(VmmEvent::Error { reason: &e }) };
            return;
        }

        Self::run_cpu(&args, &debug, cpu);
    }

    fn run_cpu<'a>(
        args: &'a Args<H, S>,
        debug: &'a (Mutex<DebugStates>, Condvar),
        mut cpu: H::Cpu<'a>,
    ) {
        // Build device contexts for this CPU.
        let mut devices = BTreeMap::<usize, Device<'a, H::Cpu<'a>>>::new();
        let t = &args.devices;

        Device::insert(&mut devices, t.console(), |d| d.create_context(&*args.hv));
        Device::insert(&mut devices, t.debugger(), |d| d.create_context(debug));
        Device::insert(&mut devices, t.vmm(), |d| d.create_context());

        // Dispatch CPU events until shutdown.
        'main: while !args.shutdown.load(Ordering::Relaxed) {
            // Run the vCPU.
            let mut exit = match cpu.run() {
                Ok(v) => v,
                Err(e) => {
                    let e = RustError::with_source("couldn't run CPU", e);
                    unsafe { args.event.invoke(VmmEvent::Error { reason: &e }) };
                    break;
                }
            };

            // Execute VM exited event.
            for d in devices.values_mut() {
                let r = match d.context.exited(exit.cpu()) {
                    Ok(v) => v,
                    Err(e) => {
                        let e = RustError::with_source(
                            format!("couldn't execute a VM exited event on a {}", d.name),
                            e.deref(),
                        );

                        unsafe { args.event.invoke(VmmEvent::Error { reason: &e }) };
                        break 'main;
                    }
                };

                if !r {
                    break 'main;
                }
            }

            // Handle exit.
            let r = match Self::handle_exit(&mut devices, exit) {
                Ok(v) => v,
                Err(e) => {
                    unsafe { args.event.invoke(VmmEvent::Error { reason: &e }) };
                    break;
                }
            };

            if !r {
                break;
            }

            // Execute post exit event.
            for d in devices.values_mut() {
                let r = match d.context.post(&mut cpu) {
                    Ok(v) => v,
                    Err(e) => {
                        let e = RustError::with_source(
                            format!("couldn't execute a post VM exit on a {}", d.name),
                            e.deref(),
                        );

                        unsafe { args.event.invoke(VmmEvent::Error { reason: &e }) };
                        break 'main;
                    }
                };

                if !r {
                    break 'main;
                }
            }
        }

        // Shutdown other CPUs.
        args.shutdown.store(true, Ordering::Relaxed);
    }

    fn handle_exit<'a, C: Cpu>(
        devices: &mut BTreeMap<usize, Device<'a, C>>,
        exit: C::Exit<'_>,
    ) -> Result<bool, RustError> {
        // Check if HLT.
        #[cfg(target_arch = "x86_64")]
        let exit = match exit.into_hlt() {
            Ok(_) => return Ok(true),
            Err(v) => v,
        };

        // Check if I/O.
        match exit.into_io() {
            Ok(io) => return Self::handle_io(devices, io),
            Err(_) => todo!(),
        }
    }

    fn handle_io<'a, C: Cpu>(
        devices: &mut BTreeMap<usize, Device<'a, C>>,
        mut io: <C::Exit<'_> as CpuExit>::Io,
    ) -> Result<bool, RustError> {
        // Get target device.
        let addr = io.addr();
        let dev = match devices
            .range_mut(..=addr)
            .last()
            .map(|v| v.1)
            .filter(move |d| addr < d.end.get())
        {
            Some(v) => v,
            None => {
                let m = format!(
                    "the CPU attempt to execute a memory-mapped I/O on a non-mapped address {:#x}",
                    addr
                );

                return Err(RustError::new(m));
            }
        };

        // Execute.
        dev.context.mmio(&mut io).map_err(|e| {
            RustError::with_source(
                format!("couldn't execute a memory-mapped I/O on a {}", dev.name),
                e.deref(),
            )
        })
    }
}

/// RAII struct to unlock all CPUs when dropped.
pub struct DebugLock<'a, H: Hypervisor, S: Screen>(&'a mut CpuManager<H, S>);

impl<'a, H: Hypervisor, S: Screen> Drop for DebugLock<'a, H, S> {
    fn drop(&mut self) {
        todo!()
    }
}

impl<'a, H: Hypervisor, S: Screen> Deref for DebugLock<'a, H, S> {
    type Target = CpuManager<H, S>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, H: Hypervisor, S: Screen> DerefMut for DebugLock<'a, H, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

/// Encapsulates arguments for a function to run a CPU.
struct Args<H: Hypervisor, S: Screen> {
    hv: Arc<H>,
    screen: Arc<S::Buffer>,
    devices: Arc<DeviceTree>,
    event: VmmEventHandler,
    shutdown: Arc<AtomicBool>,
}

/// Contains instantiated device context for a CPU.
struct Device<'a, C: Cpu> {
    context: Box<dyn DeviceContext<C> + 'a>,
    end: NonZero<usize>,
    name: &'a str,
}

impl<'a, C: Cpu> Device<'a, C> {
    fn insert<T: super::hw::Device>(
        tree: &mut BTreeMap<usize, Self>,
        dev: &'a T,
        f: impl FnOnce(&'a T) -> Box<dyn DeviceContext<C> + 'a>,
    ) {
        let addr = dev.addr();
        let dev = Self {
            context: f(dev),
            end: dev.len().checked_add(addr).unwrap(),
            name: dev.name(),
        };

        assert!(tree.insert(addr, dev).is_none());
    }
}