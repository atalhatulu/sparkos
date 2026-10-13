//! CPU RAM surface fills. These routines are not for MMIO/framebuffer registers.
//! Caller must own writable backing of at least width * height u32 pixels.

pub unsafe fn clear(ptr: *mut u32, width: u32, height: u32, color: u32) {
    if ptr.is_null() { return; }
    if let Some(len) = (width as usize).checked_mul(height as usize) {
        core::slice::from_raw_parts_mut(ptr, len).fill(color);
    }
}

pub unsafe fn fill_rect(ptr: *mut u32, width: u32, height: u32,
    x: u32, y: u32, rect_w: u32, rect_h: u32, color: u32) {
    if ptr.is_null() || x >= width || y >= height { return; }
    let row_len = rect_w.min(width - x) as usize;
    let rows = rect_h.min(height - y);
    if row_len == 0 || rows == 0 { return; }
    for row in y..y + rows {
        let offset = row as usize * width as usize + x as usize;
        core::slice::from_raw_parts_mut(ptr.add(offset), row_len).fill(color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clear_preserves_guard_pixels() {
        let mut pixels = [99u32; 14];
        unsafe { clear(pixels.as_mut_ptr().add(1), 4, 3, 7); }
        assert_eq!(&pixels[1..13], &[7; 12]);
        assert_eq!(pixels[0], 99); assert_eq!(pixels[13], 99);
    }
    #[test]
    fn rect_clips_both_axes_and_rejects_outside_without_overflow() {
        let mut pixels = [0u32; 14];
        unsafe {
            let ptr = pixels.as_mut_ptr().add(1);
            fill_rect(ptr, 4, 3, 2, 1, u32::MAX, u32::MAX, 7);
            fill_rect(ptr, 4, 3, u32::MAX, u32::MAX, 2, 2, 9);
            fill_rect(ptr, 4, 3, 0, 0, 0, 3, 9);
        }
        assert_eq!(pixels, [0, 0,0,0,0, 0,0,7,7, 0,0,7,7, 0]);
    }
}
