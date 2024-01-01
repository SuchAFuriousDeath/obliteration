use crate::kernel::Kernel;
use crate::llvm::Llvm;
use crate::log::{print, LOGGER};
use crate::ucred::AuthInfo;
use clap::{Parser, ValueEnum};
use kernel::RunError;
use param::Param;
use serde::Deserialize;
use std::fs::{create_dir_all, remove_dir_all, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use sysinfo::System;

mod arch;
mod arnd;
mod budget;
mod dmem;
mod ee;
mod errno;
mod fs;
mod idt;
mod kernel;
mod llvm;
mod log;
mod memory;
mod process;
mod regmgr;
mod rtld;
mod signal;
mod syscalls;
mod sysctl;
mod tty;
mod ucred;

fn main() -> ExitCode {
    // Begin logger.
    log::init();

    // Load arguments.
    let args = if std::env::args().any(|a| a == "--debug") {
        let file = match File::open(".kernel-debug") {
            Ok(v) => v,
            Err(e) => {
                error!(e, "Failed to open .kernel-debug");
                return ExitCode::FAILURE;
            }
        };

        match serde_yaml::from_reader(file) {
            Ok(v) => v,
            Err(e) => {
                error!(e, "Failed to read .kernel-debug");
                return ExitCode::FAILURE;
            }
        }
    } else {
        Args::parse()
    };

    // Initialize debug dump.
    if let Some(path) = &args.debug_dump {
        // Remove previous dump.
        if args.clear_debug_dump {
            if let Err(e) = remove_dir_all(path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!(e, "Failed to remove {}", path.display());
                }
            }
        }

        // Create a directory.
        if let Err(e) = create_dir_all(path) {
            warn!(e, "Failed to create {}", path.display());
        }

        // Create log file for us.
        let log = path.join("obliteration.log");

        match File::create(&log) {
            Ok(v) => LOGGER.get().unwrap().set_file(v),
            Err(e) => warn!(e, "Failed to create {}", log.display()),
        }
    }

    // Get path to param.sfo.
    let mut path = args.game.join("sce_sys");

    path.push("param.sfo");

    // Open param.sfo.
    let param = match File::open(&path) {
        Ok(v) => v,
        Err(e) => {
            error!(e, "Cannot open {}", path.display());
            return ExitCode::FAILURE;
        }
    };

    // Load param.sfo.
    let param = match Param::read(param) {
        Ok(v) => Arc::new(v),
        Err(e) => {
            error!(e, "Cannot read {}", path.display());
            return ExitCode::FAILURE;
        }
    };

    // Select execution engine.
    match args.execution_engine.unwrap_or_default() {
        #[cfg(target_arch = "x86_64")]
        ExecutionEngine::Native => run(args, param, crate::ee::native::NativeEngine::new()),
        #[cfg(not(target_arch = "x86_64"))]
        ExecutionEngine::Native => {
            error!("Native execution engine cannot be used on your machine.");
            return ExitCode::FAILURE;
        }
        ExecutionEngine::Llvm => run(args, param, crate::ee::llvm::LlvmEngine::new(&Llvm::new())),
    }
}

fn run<E: crate::ee::ExecutionEngine>(args: Args, param: Arc<Param>, ee: Arc<E>) -> ExitCode {
    match run_inner(args, param, ee) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("Run failed: {e:?}");

            ExitCode::FAILURE
        }
    }
}

fn run_inner<E: crate::ee::ExecutionEngine>(
    args: Args,
    param: Arc<Param>,
    ee: Arc<E>,
) -> Result<(), RunError<E>> {
    // Get auth info for the process.
    let auth = AuthInfo::from_title_id(param.title_id()).ok_or(RunError::TitleIdInvalid)?;

    print_hwinfo(&args, &param);

    let kernel = Kernel::new(args, param, auth, ee)?;

    kernel.run()
}

#[derive(Parser, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Args {
    #[arg(long)]
    system: PathBuf,

    #[arg(long)]
    game: PathBuf,

    #[arg(long)]
    debug_dump: Option<PathBuf>,

    #[arg(long)]
    #[serde(default)]
    clear_debug_dump: bool,

    #[arg(long, short)]
    execution_engine: Option<ExecutionEngine>,
}

#[derive(Clone, Copy, ValueEnum, Deserialize)]
enum ExecutionEngine {
    Native,
    Llvm,
}

impl Default for ExecutionEngine {
    #[cfg(target_arch = "x86_64")]
    fn default() -> Self {
        ExecutionEngine::Native
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn default() -> Self {
        ExecutionEngine::Llvm
    }
}

fn print_hwinfo(args: &Args, param: &Param) {
    // Show basic infomation.
    let mut log = info!();
    let hwinfo = System::new_with_specifics(
        sysinfo::RefreshKind::new()
            .with_memory(sysinfo::MemoryRefreshKind::new())
            .with_cpu(sysinfo::CpuRefreshKind::new()),
    );

    // Init information
    writeln!(log, "Starting Obliteration Kernel.").unwrap();
    writeln!(log, "System directory    : {}", args.system.display()).unwrap();
    writeln!(log, "Game directory      : {}", args.game.display()).unwrap();

    if let Some(ref v) = args.debug_dump {
        writeln!(log, "Debug dump directory: {}", v.display()).unwrap();
    }

    // Param information
    writeln!(
        log,
        "Application Title   : {}",
        param.title().as_ref().unwrap()
    )
    .unwrap();
    writeln!(log, "Application ID      : {}", param.title_id()).unwrap();
    writeln!(log, "Application Category: {}", param.category()).unwrap();
    writeln!(
        log,
        "Application Version : {}",
        param.app_ver().as_ref().unwrap()
    )
    .unwrap();

    writeln!(log, "CPU Information     : {}", hwinfo.cpus()[0].brand()).unwrap();
    writeln!(
        log,
        "Memory Available    : {}/{} MB",
        hwinfo.available_memory() / 1048576,
        hwinfo.total_memory() / 1048576
    )
    .unwrap(); // Convert Bytes to MB

    print(log);
}
