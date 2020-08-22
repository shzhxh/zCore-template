//!
use super::*;
use crate::time::TimeSpec;
use alloc::{collections::BTreeMap, sync::Arc, sync::Weak};
use lazy_static::lazy_static;
use spin::Mutex;
use spin::RwLock;
use zircon_object::vm::*;

lazy_static! {
    static ref KEY2SHM: RwLock<BTreeMap<usize, Weak<spin::Mutex<ShmGuard>>>> =
        RwLock::new(BTreeMap::new());
}

/// shmid data structure
///
/// struct shmid_ds
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ShmidDs {
    /// Ownership and permissions
    pub perm: IpcPerm,
    /// Size of segment (bytes)
    pub segsz: usize,
    /// Last attach time
    pub atime: usize,
    /// Last detach time
    pub dtime: usize,
    /// Last change time
    pub ctime: usize,
    /// number of current attaches
    pub nattch: usize,
}

/// shared memory Identifier for process
#[derive(Clone)]
pub struct ShmIdentifier {
    /// Shared memory address
    pub addr: usize,
    /// Shared memory buffer and data
    pub guard: Arc<spin::Mutex<ShmGuard>>,
}

/// shared memory buffer and data
pub struct ShmGuard {
    /// shared memory buffer
    pub shared_guard: Arc<VmObject>,
    /// shared memory data
    pub shmid_ds: Mutex<ShmidDs>,
}

impl ShmIdentifier {
    /// set the shared memory address on attach
    pub fn set_addr(&mut self, addr: usize) {
        self.addr = addr;
    }

    /// Get or create a ShmGuard
    pub fn new_shared_guard(
        key: usize,
        memsize: usize,
        flags: usize,
    ) -> Arc<spin::Mutex<ShmGuard>> {
        let mut key2shm = KEY2SHM.write();

        // found in the map
        if let Some(weak_guard) = key2shm.get(&key) {
            if let Some(guard) = weak_guard.upgrade() {
                return guard;
            }
        }
        let shared_guard = Arc::new(spin::Mutex::new(ShmGuard {
            shared_guard: VmObject::new_paged(pages(memsize)),
            shmid_ds: Mutex::new(ShmidDs {
                perm: IpcPerm {
                    key: key as u32,
                    uid: 0,
                    gid: 0,
                    cuid: 0,
                    cgid: 0,
                    // least significant 9 bits
                    mode: (flags as u32) & 0x1ff,
                    __seq: 0,
                    __pad1: 0,
                    __pad2: 0,
                },
                segsz: memsize,
                atime: 0,
                dtime: 0,
                ctime: TimeSpec::now().sec,
                nattch: 0,
            }),
        }));
        // insert to global map
        key2shm.insert(key, Arc::downgrade(&shared_guard));
        shared_guard
    }
}

impl ShmGuard {
    /// set last attach time
    pub fn atime(&self) {
        self.shmid_ds.lock().atime = TimeSpec::now().sec;
    }

    /// set last dettach time
    pub fn dtime(&self) {
        self.shmid_ds.lock().dtime = TimeSpec::now().sec;
    }

    /// set last change time
    pub fn ctime(&self) {
        self.shmid_ds.lock().ctime = TimeSpec::now().sec;
    }
}
