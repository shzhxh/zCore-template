use super::Scheme;
use crate::DeviceResult;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor(u32);

impl RgbColor {
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    #[inline]
    pub const fn r(self) -> u8 {
        (self.0 >> 16) as u8
    }

    #[inline]
    pub const fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }

    #[inline]
    pub const fn b(self) -> u8 {
        self.0 as u8
    }

    #[inline]
    pub const fn raw_value(self) -> u32 {
        self.0
    }
}

/// Color format for one pixel. `RGB888` means R in bits 16-23, G in bits 8-15 and B in bits 0-7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFormat {
    RGB332,
    RGB565,
    RGB888,
    ARGB8888,
}

impl ColorFormat {
    /// Number of bits per pixel.
    #[inline]
    pub const fn depth(self) -> u8 {
        match self {
            Self::RGB332 => 8,
            Self::RGB565 => 16,
            Self::RGB888 => 24,
            Self::ARGB8888 => 32,
        }
    }

    /// Number of bytes per pixel.
    #[inline]
    pub const fn bytes(self) -> u8 {
        self.depth() / 8
    }
}

#[derive(Debug)]
pub struct Rectangle {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DisplayInfo {
    /// visible width
    pub width: u32,
    /// visible height
    pub height: u32,
    /// color encoding format of RGBA
    pub format: ColorFormat,
    /// frame buffer base virtual address
    pub fb_base_vaddr: usize,
    /// frame buffer size
    pub fb_size: usize,
}

impl DisplayInfo {
    /// Number of bytes between each row of the frame buffer.
    #[inline]
    pub const fn pitch(self) -> u32 {
        self.width * self.format.bytes() as u32
    }
}

pub trait DisplayScheme: Scheme {
    fn info(&self) -> DisplayInfo;

    /// Returns the raw framebuffer.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it returns the raw pointer of the framebuffer.
    #[allow(clippy::mut_from_ref)]
    unsafe fn raw_fb(&self) -> &mut [u8];

    /// Write pixel color.
    #[inline]
    fn draw_pixel(&self, x: u32, y: u32, color: RgbColor) {
        let info = self.info();
        let fb = unsafe { self.raw_fb() };
        let offset = (x + y * info.width) as usize * info.format.bytes() as usize;
        if offset < info.fb_size {
            unsafe { fb_write_color(&mut fb[offset as usize] as _, color, info.format) };
        }
    }

    /// Fill a given rectangle with `color`.
    fn fill_rect(&self, rect: &Rectangle, color: RgbColor) {
        let info = self.info();
        let left = rect.x.min(info.width);
        let right = (left + rect.width).min(info.width);
        let top = rect.y.min(info.height);
        let bottom = (top + rect.height).min(info.height);
        for j in top..bottom {
            for i in left..right {
                self.draw_pixel(i, j, color);
            }
        }
    }

    /// Clear the screen with `color`.
    fn clear(&self, color: RgbColor) {
        let info = self.info();
        self.fill_rect(
            &Rectangle {
                x: 0,
                y: 0,
                width: info.width,
                height: info.height,
            },
            color,
        )
    }

    /// Whether need to flush the frambuffer to screen.
    #[inline]
    fn need_flush(&self) -> bool {
        false
    }

    /// Flush framebuffer to screen.
    #[inline]
    fn flush(&self) -> DeviceResult {
        Ok(())
    }
}

const fn pack_channel(r_val: u8, _r_bits: u8, g_val: u8, g_bits: u8, b_val: u8, b_bits: u8) -> u32 {
    ((r_val as u32) << (g_bits + b_bits)) | ((g_val as u32) << b_bits) | b_val as u32
}

unsafe fn fb_write_color(ptr: *mut u8, color: RgbColor, format: ColorFormat) {
    let (r, g, b) = (color.r(), color.g(), color.b());
    let dst = core::slice::from_raw_parts_mut(ptr, 4);
    match format {
        ColorFormat::RGB332 => {
            *ptr = pack_channel(r >> (8 - 3), 3, g >> (8 - 3), 3, b >> (8 - 2), 2) as u8
        }
        ColorFormat::RGB565 => {
            *(ptr as *mut u16) =
                pack_channel(r >> (8 - 5), 5, g >> (8 - 6), 6, b >> (8 - 5), 5) as u16
        }
        ColorFormat::RGB888 => {
            dst[2] = r;
            dst[1] = g;
            dst[0] = b;
        }
        ColorFormat::ARGB8888 => *(ptr as *mut u32) = color.raw_value(),
    }
}
