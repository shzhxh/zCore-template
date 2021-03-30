use riscv::register::{
	scause::{
		self,
        Scause,
		Trap,
		Exception,
		Interrupt,
	},
    sie,
	sepc,
    stval,
	stvec,
	sscratch,
    sstatus,
	sstatus::Sstatus,
};
use trapframe::{TrapFrame, UserContext};

/*
use crate::timer::{
	TICKS,
	clock_set_next_event,
    clock_close,
};
*/

//use crate::context::TrapFrame;
use super::sbi;
use super::plic;
use super::uart;

use crate::{putfmt, KERNEL_OFFSET};
use super::timer_set_next;

//global_asm!(include_str!("trap.asm"));

/*
#[repr(C)]
pub struct TrapFrame{
	pub x: [usize; 32], //General registers
	pub sstatus: Sstatus,
	pub sepc: usize,
	pub stval: usize,
	pub scause: Scause,
}
*/

pub fn init(){
	unsafe{

		sstatus::set_sie();

        init_uart();

        sie::set_sext();
        init_ext();
	}
	bare_println!("+++ setup interrupte! +++");
}

#[no_mangle]
pub extern "C" fn trap_handler(tf: &mut TrapFrame) {
    let sepc = tf.sepc;
    let scause = scause::read();
    let stval = stval::read();
    let is_int = scause.bits() >> 63;
    let code = scause.bits() & !(1 << 63);

	match scause.cause() {
		Trap::Exception(Exception::Breakpoint) => breakpoint(&mut tf.sepc),
		Trap::Exception(Exception::IllegalInstruction) => panic!("IllegalInstruction: {:#x}->{:#x}", sepc, stval),
        Trap::Exception(Exception::LoadFault) => panic!("Load access fault: {:#x}->{:#x}", sepc, stval),
        Trap::Exception(Exception::StoreFault) => panic!("Store access fault: {:#x}->{:#x}", sepc, stval),
        Trap::Exception(Exception::LoadPageFault) => page_fault(stval, tf),
        Trap::Exception(Exception::StorePageFault) => page_fault(stval, tf),
        Trap::Exception(Exception::InstructionPageFault) => page_fault(stval, tf),
		Trap::Interrupt(Interrupt::SupervisorTimer) => super_timer(),
		Trap::Interrupt(Interrupt::SupervisorSoft) => super_soft(),
		Trap::Interrupt(Interrupt::SupervisorExternal) => plic::handle_interrupt(),
		_ => panic!("Undefined Trap: {:#x} {:#x}", is_int, code)
	}
}

#[export_name = "hal_irq_handle"]
pub fn irq_handle(irq: u8) {

    error!("unhandled U-mode IRQ number: {}", irq);
}

fn breakpoint(sepc: &mut usize){
	bare_println!("A breakpoint set @0x{:x} ", sepc);

	//sepc为触发中断指令ebreak的地址
	//防止无限循环中断，让sret返回时跳转到sepc的下一条指令地址
	*sepc +=2
}

fn page_fault(stval: usize, tf: &mut TrapFrame){
    panic!("EXCEPTION Page Fault: {:?} @ {:#x}->{:#x}", scause::read().cause(), tf.sepc, stval);
}

fn super_timer(){
    timer_set_next();

    bare_print!(".");

	//发生外界中断时，epc的指令还没有执行，故无需修改epc到下一条
}

fn init_uart(){
    uart::Uart::new(0x1000_0000 + KERNEL_OFFSET).simple_init();

    bare_println!("+++ Setting up UART interrupts +++");
}

pub fn init_ext(){
    // Qemu virt
    // UART0 = 10
    plic::set_priority(10, 7);
    plic::set_threshold(0);
    plic::enable(10);

    bare_println!("+++ Setting up PLIC +++");
}

fn super_soft(){
    sbi::clear_ipi();
    bare_println!("Interrupt::SupervisorSoft!");
}

pub fn init_soft(){
    unsafe {
        sie::set_ssoft();
    }
	bare_println!("+++ setup soft int! +++");
}

#[export_name = "fetch_trap_num"]
pub fn fetch_trap_num(_context: &UserContext) -> usize {
    scause::read().bits()
}


pub fn wait_for_interrupt() {
    unsafe {
        // enable interrupt and disable
        let sie = riscv::register::sstatus::read().sie();
        riscv::register::sstatus::set_sie();
        riscv::asm::wfi();
        if !sie {
            riscv::register::sstatus::clear_sie();
        }
    }
}
