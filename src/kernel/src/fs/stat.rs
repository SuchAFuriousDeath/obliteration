/// An implementation of the `stat` structure.
#[repr(C)]
pub struct Stat {
    dev: u32,            // st_dev
    ino: u32,            // st_ino
    mode: u16,           // st_mode
    nlink: u16,          // st_nlink
    uid: i32,            // st_uid
    gid: i32,            // st_gid
    dev_type: u32,       // st_rdev
    atime: TimeSpec,     // st_atim
    mtime: TimeSpec,     // st_mtim
    ctime: TimeSpec,     // st_ctim
    size: i64,           // st_size
    blocks: i64,         // st_blocks
    blksize: i32,        // st_blksize
    flags: u32,          // st_flags
    gen: u32,            // st_gen
    _spare: i32,         // st_spare
    birthtime: TimeSpec, // st_birthtim
}

/// An implementation of the `timespec` structure.
#[repr(C)]
pub struct TimeSpec {
    sec: i64,
    nsec: i64,
}
