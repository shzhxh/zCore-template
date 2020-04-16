#![no_std]
#![feature(asm)]
#![feature(ptr_offset_from)]
#![feature(global_asm)]
#![deny(warnings, unused_must_use)]

#[macro_use]
extern crate alloc;
#[macro_use]
extern crate log;

use {
    alloc::{boxed::Box, sync::Arc, vec::Vec},
    core::mem::size_of,
    kernel_hal::GeneralRegs,
    xmas_elf::{
        program::{Flags, ProgramHeader, SegmentData, Type},
        sections::SectionData,
        symbol_table::Entry,
        ElfFile,
    },
    zircon_object::{
        ipc::*,
        util::kcounter::*,
        object::*,
        resource::{Resource, ResourceFlags, ResourceKind},
        task::*,
        vm::*,
        ZxError, ZxResult,
    },
    zircon_syscall::Syscall,
    kernel_hal::PhysFrame,
};

#[allow(dead_code)]
// These describe userboot itself
const K_PROC_SELF: usize = 0;
const K_VMARROOT_SELF: usize = 1;
// Essential job and resource handles
const K_ROOTJOB: usize = 2;
const K_ROOTRESOURCE: usize = 3;
// Essential VMO handles
const K_ZBI: usize = 4;
const K_FIRSTVDSO: usize = 5;
const K_USERBOOT_DECOMPRESSOR: usize = 8;
#[allow(dead_code)]
const K_FIRSTKERNELFILE: usize = K_USERBOOT_DECOMPRESSOR;
#[allow(dead_code)]
const K_CRASHLOG: usize = 9;
#[allow(dead_code)]
const K_COUNTERNAMES: usize = 10;
#[allow(dead_code)]
const K_COUNTERS: usize = 11;
#[allow(dead_code)]
const K_FISTINSTRUMENTATIONDATA: usize = 12;
#[allow(dead_code)]
const K_HANDLECOUNT: usize = K_FISTINSTRUMENTATIONDATA + 3;

/// Program images to run.
pub struct Images<T: AsRef<[u8]>> {
    pub userboot: T,
    pub vdso: T,
    pub decompressor: T,
    pub zbi: T,
}

pub fn run_userboot(images: &Images<impl AsRef<[u8]>>, cmdline: &str) -> Arc<Process> {
    let job = Job::root();
    let proc = Process::create(&job, "proc", 0).unwrap();
    let thread = Thread::create(&proc, "thread", 0).unwrap();
    let resource = Resource::create("root", ResourceKind::ROOT, 0, 0, ResourceFlags::empty());
    let vmar = proc.vmar();

    // userboot
    let (entry, userboot_size) = {
        let elf = ElfFile::new(images.userboot.as_ref()).unwrap();
        let size = elf.load_segment_size();
        let vmar = vmar
            .allocate(None, size, VmarFlags::CAN_MAP_RXW, PAGE_SIZE)
            .unwrap();
        vmar.load_from_elf(&elf).unwrap();
        (vmar.addr() + elf.header.pt2.entry_point() as usize, size)
    };

    // vdso
    let vdso_vmo = {
        let elf = ElfFile::new(images.vdso.as_ref()).unwrap();
        let vdso_vmo = VmObject::new_paged(images.vdso.as_ref().len() / PAGE_SIZE + 1);
        vdso_vmo.write(0, images.vdso.as_ref());
        let size = elf.load_segment_size();
        let vmar = vmar
            .allocate_at(
                userboot_size,
                size,
                VmarFlags::CAN_MAP_RXW | VmarFlags::SPECIFIC,
                PAGE_SIZE,
            )
            .unwrap();
        vmar.map_from_elf(&elf, vdso_vmo.clone()).unwrap();
        #[cfg(feature = "std")]
        {
            let syscall_entry_offset =
                elf.get_symbol_address("zcore_syscall_entry")
                    .expect("failed to locate syscall entry") as usize;
            // fill syscall entry
            vdso_vmo.write(
                syscall_entry_offset,
                &(kernel_hal_unix::syscall_entry as usize).to_ne_bytes(),
            );
        }
        vdso_vmo
    };

    // zbi
    let zbi_vmo = {
        let vmo = VmObject::new_paged(images.zbi.as_ref().len() / PAGE_SIZE + 1);
        vmo.write(0, images.zbi.as_ref());
        vmo.set_name("zbi");
        vmo
    };

    // decompressor
    let decompressor_vmo = {
        let elf = ElfFile::new(images.decompressor.as_ref()).unwrap();
        let size = elf.load_segment_size();
        let vmo = VmObject::new_paged(size / PAGE_SIZE);
        vmo.write(0, images.decompressor.as_ref());
        vmo.set_name("lib/hermetic/decompress-zbi.so");
        vmo
    };

    // stack
    const STACK_PAGES: usize = 8;
    let stack_vmo = VmObject::new_paged(STACK_PAGES);
    let flags = MMUFlags::READ | MMUFlags::WRITE | MMUFlags::USER;
    let stack_bottom = vmar
        .map(None, stack_vmo.clone(), 0, stack_vmo.len(), flags)
        .unwrap();
    // WARN: align stack to 16B, then emulate a 'call' (push rip)
    let sp = stack_bottom + stack_vmo.len() - 8;

    // channel
    let (user_channel, kernel_channel) = Channel::create();
    let handle = Handle::new(user_channel, Rights::DEFAULT_CHANNEL);

    // FIXME: pass correct handles
    let mut handles = vec![Handle::new(proc.clone(), Rights::DUPLICATE); 15];
    handles[K_VMARROOT_SELF] = Handle::new(proc.vmar(), Rights::DEFAULT_VMAR | Rights::IO);
    handles[K_ROOTJOB] = Handle::new(job, Rights::DEFAULT_JOB);
    handles[K_ROOTRESOURCE] = Handle::new(resource, Rights::DEFAULT_RESOURCE);
    handles[K_ZBI] = Handle::new(zbi_vmo, Rights::DEFAULT_VMO);
    // set up handles[K_FIRSTVDSO..K_LASTVDSO + 1]
    const VDSO_CONSTANT_BASE: usize = 0x4940;
    let constants: [u8; 112] = unsafe { core::mem::transmute(kernel_hal::vdso_constants()) };
    vdso_vmo.write(VDSO_CONSTANT_BASE, &constants);
    vdso_vmo.set_name("vdso/full");
    let vdso_test1 = vdso_vmo.create_clone(0, vdso_vmo.len());
    vdso_test1.set_name("vdso/test1");
    let vdso_test2 = vdso_vmo.create_clone(0, vdso_vmo.len());
    vdso_test2.set_name("vdso/test2");
    handles[K_FIRSTVDSO] = Handle::new(vdso_vmo, Rights::DEFAULT_VMO | Rights::EXECUTE);
    handles[K_FIRSTVDSO + 1] = Handle::new(vdso_test1, Rights::DEFAULT_VMO | Rights::EXECUTE);
    handles[K_FIRSTVDSO + 2] = Handle::new(vdso_test2, Rights::DEFAULT_VMO | Rights::EXECUTE);
    // FIXME correct rights for decompressor engine
    handles[K_USERBOOT_DECOMPRESSOR] =
        Handle::new(decompressor_vmo, Rights::DEFAULT_VMO | Rights::EXECUTE);
    // TODO to use correct CrashLogVmo handle
    let crash_log_vmo = VmObject::new_paged(1);
    crash_log_vmo.set_name("crashlog");
    handles[K_CRASHLOG] = Handle::new(crash_log_vmo, Rights::DEFAULT_VMO);

    let (counter_name_vmo, kcounters_vmo) = kcounter_vmo_init();
    handles[K_COUNTERNAMES] = Handle::new(counter_name_vmo, Rights::DEFAULT_VMO);
    handles[K_COUNTERS] = Handle::new(kcounters_vmo, Rights::DEFAULT_VMO);

    // TODO to use correct Instrumentation data handle
    let instrumentation_data_vmo = VmObject::new_paged(0);
    instrumentation_data_vmo.set_name("UNIMPLEMENTED_VMO");
    handles[K_FISTINSTRUMENTATIONDATA] =
        Handle::new(instrumentation_data_vmo.clone(), Rights::DEFAULT_VMO);
    handles[K_FISTINSTRUMENTATIONDATA + 1] =
        Handle::new(instrumentation_data_vmo.clone(), Rights::DEFAULT_VMO);
    handles[K_FISTINSTRUMENTATIONDATA + 2] =
        Handle::new(instrumentation_data_vmo, Rights::DEFAULT_VMO);

    // check: handle to root proc should be only

    let data = Vec::from(cmdline.replace(':', "\0") + "\0console.shell=true\0");
    let msg = MessagePacket { data, handles };
    kernel_channel.write(msg).unwrap();

    proc.start(&thread, entry, sp, handle, 0)
        .expect("failed to start main thread");
    proc
}

const KCOUNTER_MAGIC: u64 = 1547273975u64;
#[repr(C)]
struct kcounter_vmo_header {
    magic: u64,
    max_cpu: u64,
    counter_table_size: usize,
}

fn kcounter_vmo_init() -> (Arc<VmObject>, Arc<VmObject>) {
    extern "C" {
        fn kcounter_descriptor_begin();
        fn kcounter_descriptor_end();
        fn kcounters_arena_start();
        fn kcounters_arena_page_end();
    }
    const DESC_SIZE: usize = size_of::<KCounterDesc>();
    let counter_name_vmo = {
        let counter_table_size = kcounter_descriptor_end as usize - kcounter_descriptor_begin as usize;
        let counter_page_size = pages(counter_table_size / size_of::<KCounterDescriptor>() * DESC_SIZE + 24);
        let counter_name_vmo = VmObject::new_paged(counter_page_size);
        let header = kcounter_vmo_header {
            magic: KCOUNTER_MAGIC,
            max_cpu: 1u64,
            counter_table_size,
        };
        let serde_header: [u8; size_of::<kcounter_vmo_header>()] = unsafe{ core::mem::transmute(header) };
        counter_name_vmo.write(0, &serde_header);
        counter_name_vmo
    };
    let start = kcounter_descriptor_begin as usize as *const KCounterDescriptor;
    let end = kcounter_descriptor_end as usize as *const KCounterDescriptor;
    let descs = unsafe { core::slice::from_raw_parts(start, end.offset_from(start) as usize) };
    for (i, descriptor) in descs.iter().enumerate() {
        let serde_counter: [u8; DESC_SIZE] =
            unsafe { core::mem::transmute(descriptor.gen_desc()) };
        counter_name_vmo.write(24 + i * DESC_SIZE, &serde_counter);
    }
    counter_name_vmo.set_name("counters/desc");

    let counter_frames = (kcounters_arena_start as usize/PAGE_SIZE..kcounters_arena_page_end as usize/PAGE_SIZE)
        .map(|vaddr| {
            Some(PhysFrame::new_with_paddr(kernel_hal::virt_to_phys(vaddr*PAGE_SIZE)))
        }).collect();
    let kcounters_vmo = VmObject::create_paged_with_frames(counter_frames);
    kcounters_vmo.set_name("counters/arena");
    (counter_name_vmo, kcounters_vmo)

}

kcounter!(EXCEPTIONS_USER, "exceptions.user");
kcounter!(EXCEPTIONS_TIMER, "exceptions.timer");

#[export_name = "run_task"]
pub fn run_task(thread: Arc<Thread>) {
    let vmtoken = thread.proc().vmar().table_phys();
    let future = async move {
        kernel_hal::Thread::set_tid(thread.id(), thread.proc().id());
        loop {
            let mut cx = thread.wait_for_run().await;
            trace!("go to user: {:#x?}", cx);
            debug!("switch to {}|{}", thread.proc().name(), thread.name());
            kernel_hal::context_run(&mut cx);
            trace!("back from user: {:#x?}", cx);
            EXCEPTIONS_USER.add(1);
            let mut exit = false;
            match cx.trap_num {
                0x100 => exit = handle_syscall(&thread, &mut cx.general).await,
                0x20..=0x3f => {
                    kernel_hal::irq_handle(cx.trap_num as u8 - 0x20);
                    if cx.trap_num == 0x20 {
                        EXCEPTIONS_TIMER.add(1);
                        kernel_hal::yield_now().await;
                    }
                }
                _ => panic!("not supported interrupt from user mode. {:#x?}", cx),
            }
            thread.end_running(cx);
            if exit {
                break;
            }
        }
    };
    kernel_hal::Thread::spawn(Box::pin(future), vmtoken);
}

async fn handle_syscall(thread: &Arc<Thread>, regs: &mut GeneralRegs) -> bool {
    let num = regs.rax as u32;
    // LibOS: Function call ABI
    #[cfg(feature = "std")]
    let args = unsafe {
        let a6 = (regs.rsp as *const usize).read();
        let a7 = (regs.rsp as *const usize).add(1).read();
        [
            regs.rdi, regs.rsi, regs.rdx, regs.rcx, regs.r8, regs.r9, a6, a7,
        ]
    };
    // RealOS: Zircon syscall ABI
    #[cfg(not(feature = "std"))]
    let args = [
        regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9, regs.r12, regs.r13,
    ];
    let mut syscall = Syscall {
        regs,
        thread: thread.clone(),
        exit: false,
    };
    syscall.regs.rax = syscall.syscall(num, args).await as usize;
    syscall.exit
}

pub trait ElfExt {
    fn load_segment_size(&self) -> usize;
    fn get_symbol_address(&self, symbol: &str) -> Option<u64>;
}

impl ElfExt for ElfFile<'_> {
    /// Get total size of all LOAD segments.
    fn load_segment_size(&self) -> usize {
        let pages = self
            .program_iter()
            .filter(|ph| ph.get_type().unwrap() == Type::Load)
            .map(|ph| pages(ph.mem_size() as usize))
            .sum::<usize>();
        pages * PAGE_SIZE
    }

    /// Get address of the given `symbol`.
    fn get_symbol_address(&self, symbol: &str) -> Option<u64> {
        for section in self.section_iter() {
            if let SectionData::SymbolTable64(entries) = section.get_data(self).unwrap() {
                for e in entries {
                    if e.get_name(self).unwrap() == symbol {
                        return Some(e.value());
                    }
                }
            }
        }
        None
    }
}

pub trait VmarExt {
    fn load_from_elf(&self, elf: &ElfFile) -> ZxResult;
    fn map_from_elf(&self, elf: &ElfFile, vmo: Arc<VmObject>) -> ZxResult;
}

impl VmarExt for VmAddressRegion {
    /// Create `VMObject` from all LOAD segments of `elf` and map them to this VMAR.
    /// Return the first `VMObject`.
    fn load_from_elf(&self, elf: &ElfFile) -> ZxResult {
        for ph in elf.program_iter() {
            if ph.get_type().unwrap() != Type::Load {
                continue;
            }
            let vmo = make_vmo(&elf, ph)?;
            let len = vmo.len();
            let flags = ph.flags().to_mmu_flags();
            self.map_at(ph.virtual_addr() as usize, vmo.clone(), 0, len, flags)?;
        }
        Ok(())
    }

    fn map_from_elf(&self, elf: &ElfFile, vmo: Arc<VmObject>) -> ZxResult {
        for ph in elf.program_iter() {
            if ph.get_type().unwrap() != Type::Load {
                continue;
            }
            let flags = ph.flags().to_mmu_flags();
            let vmo_offset = pages(ph.physical_addr() as usize) * PAGE_SIZE;
            let len = pages(ph.mem_size() as usize) * PAGE_SIZE;
            self.map_at(
                ph.virtual_addr() as usize,
                vmo.clone(),
                vmo_offset,
                len,
                flags,
            )?;
        }
        Ok(())
    }
}

trait FlagsExt {
    fn to_mmu_flags(&self) -> MMUFlags;
}

impl FlagsExt for Flags {
    fn to_mmu_flags(&self) -> MMUFlags {
        let mut flags = MMUFlags::USER;
        if self.is_read() {
            flags.insert(MMUFlags::READ);
        }
        if self.is_write() {
            flags.insert(MMUFlags::WRITE);
        }
        if self.is_execute() {
            flags.insert(MMUFlags::EXECUTE);
        }
        flags
    }
}

fn make_vmo(elf: &ElfFile, ph: ProgramHeader) -> ZxResult<Arc<VmObject>> {
    assert_eq!(ph.get_type().unwrap(), Type::Load);
    let pages = pages(ph.mem_size() as usize);
    let vmo = VmObject::new_paged(pages);
    let data = match ph.get_data(&elf).unwrap() {
        SegmentData::Undefined(data) => data,
        _ => return Err(ZxError::INVALID_ARGS),
    };
    vmo.write(0, data);
    Ok(vmo)
}
