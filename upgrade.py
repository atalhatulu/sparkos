import os

# 1. Update gui.rs
with open('src/gui.rs', 'r') as f:
    gui = f.read()

gui = gui.replace('width: 1280', 'width: 1920')
gui = gui.replace('height: 720', 'height: 1080')
gui = gui.replace('data_port.write(1280)', 'data_port.write(1920)')
gui = gui.replace('data_port.write(720)', 'data_port.write(1080)')

# Taskbar and Start Menu constants
gui = gui.replace('720 - 34', '1080 - 34')
gui = gui.replace('720 - 30', '1080 - 30')
gui = gui.replace('720 - 20', '1080 - 20')
gui = gui.replace('1280 - 100', '1920 - 100')
gui = gui.replace('1280 - 85', '1920 - 85')
gui = gui.replace('y: 720 - 30 - 80', 'y: 1080 - 30 - 80')
gui = gui.replace('610', '970') # 720-110 = 610, 1080-110 = 970
gui = gui.replace('625', '985')
gui = gui.replace('665', '1025')

# Add BACKBUFFER and swap_buffers
gui = gui.replace('pub static mut PHYS_OFFSET: u64 = 0;', 'pub static mut PHYS_OFFSET: u64 = 0;\npub static mut BACKBUFFER: *mut u32 = core::ptr::null_mut();')

init_orig = """        VESA.framebuffer = (PHYS_OFFSET + 0xFD000000) as *mut u32;
    }
}"""
init_new = """        VESA.framebuffer = (PHYS_OFFSET + 0xFD000000) as *mut u32;
        let mut buf = alloc::vec::Vec::<u32>::with_capacity(1920 * 1080);
        buf.resize(1920 * 1080, 0);
        BACKBUFFER = buf.as_mut_ptr();
        core::mem::forget(buf);
    }
}

pub fn swap_buffers() {
    unsafe {
        if VESA.framebuffer.is_null() || BACKBUFFER.is_null() { return; }
        core::ptr::copy_nonoverlapping(BACKBUFFER, VESA.framebuffer, 1920 * 1080);
    }
}
"""
gui = gui.replace(init_orig, init_new)

# Replace drawing to VESA.framebuffer with BACKBUFFER
gui = gui.replace('VESA.framebuffer.add(offset)', 'BACKBUFFER.add(offset)')
gui = gui.replace('VESA.framebuffer.add(src_offset)', 'BACKBUFFER.add(src_offset)')
gui = gui.replace('VESA.framebuffer.add(dst_offset)', 'BACKBUFFER.add(dst_offset)')
gui = gui.replace('VESA.framebuffer.add(i)', 'BACKBUFFER.add(i)')
gui = gui.replace('VESA.framebuffer.is_null()', 'BACKBUFFER.is_null()')

with open('src/gui.rs', 'w') as f:
    f.write(gui)


# 2. Update mouse.rs
with open('src/mouse.rs', 'r') as f:
    mouse = f.read()

mouse = mouse.replace('1280', '1920')
mouse = mouse.replace('1279', '1919')
mouse = mouse.replace('720', '1080')
mouse = mouse.replace('719', '1079')
mouse = mouse.replace('690', '1050')

# Menu coords
mouse = mouse.replace('610', '970')
mouse = mouse.replace('650', '1010')

# Add swap_buffers calls after draw_desktop_and_window
mouse = mouse.replace('w.visible);', 'w.visible);\n                    crate::gui::swap_buffers();')
mouse = mouse.replace('crate::gui::draw_desktop();', 'crate::gui::draw_desktop();\n                crate::gui::swap_buffers();')

with open('src/mouse.rs', 'w') as f:
    f.write(mouse)

# 3. Update main.rs
with open('src/main.rs', 'r') as f:
    main = f.read()

main = main.replace('gui::draw_desktop_and_window(100, 100, 800, 500, false);', 'gui::draw_desktop_and_window(100, 100, 800, 500, false);\n    gui::swap_buffers();')
with open('src/main.rs', 'w') as f:
    f.write(main)

print("Upgrade complete!")
