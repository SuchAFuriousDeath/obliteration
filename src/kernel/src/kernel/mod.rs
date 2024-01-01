use crate::budget::ProcType;
use crate::ee::{EntryArg, ExecutionEngine, RawFn};
use crate::fs::{FsError, VPath};
use crate::process::{VProc, VProcError, VThread};
use crate::rtld::{LoadError, LoadFlags, ModuleFlags, RuntimeLinker, RuntimeLinkerError};
use crate::tty::{TtyError, TtyManager};
use crate::ucred::{AuthAttrs, AuthCaps, AuthInfo, AuthPaid, Ucred};
use crate::{
    arch::MachDep,
    arnd::Arnd,
    budget::{Budget, BudgetManager},
    dmem::DmemManager,
    fs::Fs,
    memory::{MemoryManager, MemoryManagerError},
    regmgr::RegMgr,
    syscalls::Syscalls,
    sysctl::Sysctl,
};
use crate::{info, warn, Args};
use llt::{SpawnError, Thread};
use macros::vpath;
use param::Param;
use std::io::Error as IoError;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

#[allow(unused)]
pub struct Kernel<E: ExecutionEngine> {
    param: Arc<Param>,
    arnd: Arc<Arnd>,
    auth: AuthInfo,
    budgetmgr: Arc<BudgetManager>,
    dmemmgr: Arc<DmemManager>,
    ee: Arc<E>,
    fs: Arc<Fs>,
    ld: Arc<RuntimeLinker<E>>,
    mm: Arc<MemoryManager>,
    proc: Arc<VProc>,
    regmgr: Arc<RegMgr>,
    sysctl: Arc<Sysctl>,
    ttymgr: Arc<TtyManager>,
}

impl<E: ExecutionEngine> Kernel<E> {
    pub const LIBKERNEL_PATH: &'static VPath = vpath!("/system/common/lib/libkernel.sprx");
    pub const LIBC_INTERNAL_PATH: &'static VPath =
        vpath!("/system/common/lib/libSceLibcInternal.sprx");

    pub fn new(
        args: Args,
        param: Arc<Param>,
        auth: AuthInfo,
        ee: Arc<E>,
    ) -> Result<Arc<Self>, KernelError<E>> {
        let mut syscalls = Syscalls::new();

        let cred = Arc::new(Ucred::new(
            0,
            0,
            vec![0],
            AuthInfo {
                paid: AuthPaid::KERNEL,
                caps: AuthCaps::new([0x4000000000000000, 0, 0, 0]),
                attrs: AuthAttrs::new([0, 0, 0, 0]),
                unk: [0; 64],
            },
        ));

        // Initializes filesystem.
        let fs = Fs::new(args.system, args.game, &param, &cred, &mut syscalls)?;

        let arnd = Arnd::new();
        let budgetmgr = BudgetManager::new(&mut syscalls);
        let budget_id = budgetmgr.create(Budget::new("big app", ProcType::BigApp));

        let dmemmgr = DmemManager::new(&fs, &mut syscalls);
        let machdep = MachDep::new(&mut syscalls);

        // Initialize memory management.
        let mm = MemoryManager::new(&mut syscalls)?;
        let mut log = info!();

        writeln!(log, "Page size             : {:#x}", mm.page_size()).unwrap();
        writeln!(
            log,
            "Allocation granularity: {:#x}",
            mm.allocation_granularity()
        )
        .unwrap();
        writeln!(
            log,
            "Main stack            : {:p}:{:p}",
            mm.stack().start(),
            mm.stack().end()
        )
        .unwrap();

        let regmgr = RegMgr::new(&mut syscalls);

        info!("Initializing runtime linker.");

        let sysctl = Sysctl::new(&arnd, &mm, &machdep, &mut syscalls);

        // Initialize TTY system.
        let ttymgr = TtyManager::new(&fs)?;

        // TODO: Get correct budget name from the PS4.
        let proc = VProc::new(
            auth,
            budget_id,
            ProcType::BigApp,
            1,         // See sys_budget_set on the PS4.
            fs.root(), // TODO: Change to a proper value once FS rework is done.
            "QXuNNl0Zhn",
            &mut syscalls,
        )?;

        let ld = RuntimeLinker::new(&fs, &mm, &ee, &mut syscalls, args.debug_dump.as_deref())?;

        ee.set_syscalls(syscalls);

        let kernel = Kernel {
            param,
            arnd,
            auth,
            budgetmgr,
            dmemmgr,
            ee,
            fs,
            ld,
            mm,
            proc,
            regmgr,
            sysctl,
            ttymgr,
        };

        Ok(Arc::new(kernel))
    }

    pub fn run(&self, path: PathBuf) -> Result<(), RunError<E>> {
        // Print application module.
        let app = self.ld.app();
        let mut log = info!();

        writeln!(log, "Application   : {}", app.path()).unwrap();
        app.print(log);

        // Preload libkernel.
        let mut flags = LoadFlags::UNK1;

        if self.proc.budget_ptype() == ProcType::BigApp {
            flags |= LoadFlags::BIG_APP;
        }

        info!("Preloading libkernel");

        let module = self
            .ld
            .load(&self.proc, Self::LIBKERNEL_PATH, flags, false, true)?;

        module.flags_mut().remove(ModuleFlags::UNK2);
        module.print(info!());

        self.ld.set_kern_module(module);

        info!("Preloading libSceLibcInternal.");

        let module = self
            .ld
            .load(&self.proc, Self::LIBC_INTERNAL_PATH, flags, false, true)?;

        module.flags_mut().remove(ModuleFlags::UNK2);
        module.print(info!());

        drop(module);

        // Get eboot.bin.
        if app.file_info().is_none() {
            todo!("statically linked eboot.bin");
        }

        // Get entry point.
        let boot = self.ld.kern_module().unwrap();
        let mut arg = Box::pin(EntryArg::<E>::new(
            &self.arnd,
            &self.proc,
            &self.mm,
            app.clone(),
        ));
        let entry = unsafe { boot.get_function(boot.entry().unwrap()) };
        let entry = move || unsafe { entry.exec1(arg.as_mut().as_vec().as_ptr()) };

        // Spawn main thread.
        info!("Starting application.");

        // TODO: Check how this constructed.
        let cred = Arc::new(Ucred::new(0, 0, vec![0], AuthInfo::SYS_CORE));
        let main = VThread::new(&self.proc, &cred);
        let stack = self.mm.stack();
        let main = unsafe { main.start(stack.start(), stack.len(), entry) }?;

        // Begin Discord Rich Presence before blocking current thread.
        discord_presence(&self.param);

        // Wait for main thread to exit. This should never return.
        join_thread(main).map_err(|e| e.into())
    }
}

#[derive(Error, Debug)]
pub enum KernelError<E: ExecutionEngine> {
    #[error("failed to construct filesystem: {0}")]
    FsError(#[from] FsError),

    #[error("failed to construct memory manager: {0}")]
    MemoryManagerError(#[from] MemoryManagerError),

    #[error("failed to construct tty manager: {0}")]
    TtyError(#[from] TtyError),

    #[error("failed to construct vproc: {0}")]
    VProcError(#[from] VProcError),

    #[error("failed to construct runtime linker: {0}")]
    RuntimeLinkerError(#[from] RuntimeLinkerError<E>),
}

#[derive(Error, Debug)]
pub enum RunError<E: ExecutionEngine> {
    #[error("failed to spawn main thread: {0}")]
    SpawnError(#[from] SpawnError),

    #[error("failed to join main thread: {0}")]
    JoinThreadError(#[from] IoError),

    #[error("failed to load module: {0}")]
    LoadError(#[from] LoadError<E>),
}

#[cfg(unix)]
fn join_thread(thr: Thread) -> Result<(), IoError> {
    let err = unsafe { libc::pthread_join(thr, std::ptr::null_mut()) };

    if err != 0 {
        Err(IoError::from_raw_os_error(err))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn join_thread(thr: Thread) -> Result<(), IoError> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};

    if unsafe { WaitForSingleObject(thr, INFINITE) } != WAIT_OBJECT_0 {
        return Err(IoError::last_os_error());
    }

    assert_ne!(unsafe { CloseHandle(thr) }, 0);

    Ok(())
}

fn discord_presence(param: &Param) {
    use discord_rich_presence::activity::{Activity, Assets, Timestamps};
    use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

    // Initialize new Discord IPC with our ID.
    info!("Initializing Discord rich presence.");

    let mut client = match DiscordIpcClient::new("1168617561244565584") {
        Ok(v) => v,
        Err(e) => {
            warn!(e, "Failed to create Discord IPC");
            return;
        }
    };

    // Attempt to have IPC connect to user's Discord, will fail if user doesn't have Discord running.
    if client.connect().is_err() {
        // No Discord running should not be a warning.
        return;
    }

    // Create details about game.
    let details = format!(
        "Playing {} - {}",
        param.title().as_ref().unwrap(),
        param.title_id()
    );
    let start = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Send activity to Discord.
    let payload = Activity::new()
        .details(&details)
        .assets(
            Assets::new()
                .large_image("obliteration-icon")
                .large_text("Obliteration"),
        )
        .timestamps(Timestamps::new().start(start.try_into().unwrap()));

    if let Err(e) = client.set_activity(payload) {
        // If failing here, user's Discord most likely crashed or is offline.
        warn!(e, "Failed to update Discord presence");
        return;
    }

    // Keep client alive forever.
    Box::leak(client.into());
}
