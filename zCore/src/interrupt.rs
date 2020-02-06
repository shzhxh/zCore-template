#![allow(dead_code)]
#![allow(non_upper_case_globals)]
use {
    super::process::processor,
    kernel_hal_bare::arch::{ack, timer_init},
    trapframe::{init as init_interrupt, TrapFrame},
    x86_64::registers::control::*,
};

#[export_name = "hal_lapic_addr"]
pub static LAPIC_ADDR: usize = 0xfee0_0000;

pub fn init() {
    check_and_set_cpu_features();
    unsafe {
        init_interrupt();
    }
    timer_init();
    x86_64::instructions::interrupts::enable();
}

fn check_and_set_cpu_features() {
    unsafe {
        // Enable NX bit.
        Efer::update(|f| f.insert(EferFlags::NO_EXECUTE_ENABLE));

        // By default the page of CR3 have write protect.
        // We have to remove that before editing page table.
        Cr0::update(|f| f.remove(Cr0Flags::WRITE_PROTECT));
    }
}

// Reference: https://wiki.osdev.org/Exceptions
const DivideError: u8 = 0;
const Debug: u8 = 1;
const NonMaskableInterrupt: u8 = 2;
const Breakpoint: u8 = 3;
const Overflow: u8 = 4;
const BoundRangeExceeded: u8 = 5;
const InvalidOpcode: u8 = 6;
const DeviceNotAvailable: u8 = 7;
const DoubleFault: u8 = 8;
const CoprocessorSegmentOverrun: u8 = 9;
const InvalidTSS: u8 = 10;
const SegmentNotPresent: u8 = 11;
const StackSegmentFault: u8 = 12;
const GeneralProtectionFault: u8 = 13;
const PageFault: u8 = 14;
const FloatingPointException: u8 = 16;
const AlignmentCheck: u8 = 17;
const MachineCheck: u8 = 18;
const SIMDFloatingPointException: u8 = 19;
const VirtualizationException: u8 = 20;
const SecurityException: u8 = 30;

const IRQ0: u8 = 32;

// IRQ
const Timer: u8 = 0;

#[no_mangle]
pub extern "C" fn rust_trap(tf: &mut TrapFrame) {
    trace!("Interrupt: {:#x} @ CPU{}", tf.trap_num, 0); // TODO 0 should replace in multi-core case
    match tf.trap_num as u8 {
        Breakpoint => breakpoint(),
        DoubleFault => double_fault(tf),
        PageFault => page_fault(tf),
        IRQ0..=63 => {
            let irq = tf.trap_num as u8 - IRQ0;
            ack(irq); // must ack before switching
            match irq {
                Timer => processor().tick(),
                _ => {
                    warn!("unhandled external IRQ number: {}", irq);
                }
            }
        }
        _ => panic!("Unhandled interrupt {:x} {:#x?}", tf.trap_num, tf),
    }
}

fn breakpoint() {
    error!("\nEXCEPTION: Breakpoint");
}

fn double_fault(tf: &TrapFrame) {
    panic!("\nEXCEPTION: Double Fault\n{:#x?}", tf);
}

fn page_fault(tf: &mut TrapFrame) {
    panic!("\nEXCEPTION: Page Fault\n{:#x?}", tf);
}
