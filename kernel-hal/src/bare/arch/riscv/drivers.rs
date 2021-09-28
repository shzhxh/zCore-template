use alloc::{boxed::Box, sync::Arc};

use zcore_drivers::builder::{DeviceTreeDriverBuilder, IoMapper};
use zcore_drivers::irq::riscv::ScauseIntCode;
use zcore_drivers::scheme::IrqScheme;
use zcore_drivers::{Device, DeviceResult};

use crate::drivers::{IRQ, UART};
use crate::utils::init_once::InitOnce;
use crate::{mem::phys_to_virt, PhysAddr, VirtAddr};

static PLIC: InitOnce<Arc<dyn IrqScheme>> = InitOnce::new();

struct IoMapperImpl;

impl IoMapper for IoMapperImpl {
    fn query_or_map(&self, paddr: PhysAddr, _size: usize) -> Option<VirtAddr> {
        Some(phys_to_virt(paddr)) // FIXME
    }
}

pub(super) fn init() -> DeviceResult {
    let dev_list =
        DeviceTreeDriverBuilder::new(phys_to_virt(crate::config::KCONFIG.dtb_paddr), IoMapperImpl)?
            .build()?;
    for dev in dev_list.into_iter() {
        match dev {
            Device::Irq(irq) => {
                if !IRQ.is_completed() {
                    IRQ.init_once_by(irq);
                } else {
                    PLIC.init_once_by(irq);
                }
            }
            Device::Uart(uart) => UART.init_once_by(uart),
            _ => {}
        }
    }

    IRQ.register_handler(
        ScauseIntCode::SupervisorSoft as _,
        Box::new(|| super::trap::super_soft()),
    )?;
    IRQ.register_handler(
        ScauseIntCode::SupervisorTimer as _,
        Box::new(|| super::trap::super_timer()),
    )?;
    IRQ.unmask(ScauseIntCode::SupervisorSoft as _)?;
    IRQ.unmask(ScauseIntCode::SupervisorTimer as _)?;

    Ok(())
}
