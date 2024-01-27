use super::stat::Stat;
use super::{IoCmd, Vnode};
use crate::errno::Errno;
use crate::process::VThread;
use crate::ucred::Ucred;
use bitflags::bitflags;
use std::any::Any;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

/// An implementation of `file` structure.
#[derive(Debug)]
pub struct VFile {
    ty: VFileType,                    // f_type
    data: Arc<dyn Any + Send + Sync>, // f_data
    ops: &'static VFileOps,           // f_ops
    flags: VFileFlags,                // f_flag
}

impl VFile {
    pub(super) fn new(
        ty: VFileType,
        data: Arc<dyn Any + Send + Sync>,
        ops: &'static VFileOps,
    ) -> Self {
        Self {
            ty,
            data,
            ops,
            flags: VFileFlags::empty(),
        }
    }

    pub fn flags(&self) -> VFileFlags {
        self.flags
    }

    pub fn flags_mut(&mut self) -> &mut VFileFlags {
        &mut self.flags
    }

    /// An implementation of `fo_write`.
    pub fn write(&self, data: &[u8], td: Option<&VThread>) -> Result<usize, Box<dyn Errno>> {
        (self.ops.write)(self, data, td)
    }

    /// An implementation of `fo_ioctl`.
    pub fn ioctl(
        &self,
        cmd: IoCmd,
        data: &mut [u8],
        td: Option<&VThread>,
    ) -> Result<(), Box<dyn Errno>> {
        (self.ops.ioctl)(self, cmd, data, td)
    }

    /// An implementation of `fo_stat`.
    pub fn stat(
        &self,
        stat: &mut Stat,
        cred: &Ucred,
        td: Option<&VThread>,
    ) -> Result<(), Box<dyn Errno>> {
        (self.ops.stat)(self, stat, cred, td)
    }

    /// An implementation of `fo_close`.
    pub fn close(&self, td: Option<&VThread>) -> Result<(), Box<dyn Errno>> {
        (self.ops.close)(self, td)
    }
}

impl Seek for VFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        todo!()
    }
}

impl Read for VFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        todo!()
    }
}

/// Type of [`VFile`].
#[derive(Debug)]
pub enum VFileType {
    Vnode(Arc<Vnode>), // DTYPE_VNODE
}

/// An implementation of `fileops` structure.
#[derive(Debug)]
pub struct VFileOps {
    pub write: fn(&VFile, &[u8], Option<&VThread>) -> Result<usize, Box<dyn Errno>>,
    pub ioctl: fn(&VFile, IoCmd, &mut [u8], Option<&VThread>) -> Result<(), Box<dyn Errno>>,
    pub stat: VFileStat,
    pub close: VFileclose,
}

type VFileStat = fn(&VFile, &mut Stat, &Ucred, Option<&VThread>) -> Result<(), Box<dyn Errno>>;
type VFileclose = fn(&VFile, Option<&VThread>) -> Result<(), Box<dyn Errno>>;

bitflags! {
    /// Flags for [`VFile`].
    #[derive(Debug, Clone, Copy)]
    pub struct VFileFlags: u32 {
        const FREAD = 0x00000001;
        const FWRITE = 0x00000002;
    }
}
