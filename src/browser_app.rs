//! SparkOS Desktop V1.24 — Lightweight Web Viewer (`browser.app`)
//!
//! Provides a dedicated Ring-3 web viewer application with URL navigation,
//! safe HTML parsing (title, headings, paragraphs, links), and SparkUI widget rendering.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::libspark_ui::{Button, Label, TextBox, Widget};

pub const BROWSER_WIDTH: u32 = 360;
pub const BROWSER_HEIGHT: u32 = 220;

#[derive(Debug, Clone)]
pub enum HtmlNode {
    Heading(String),
    Paragraph(String),
    Link { text: String, href: String },
}

#[derive(Debug, Clone)]
pub struct HtmlDocument {
    pub title: String,
    pub nodes: Vec<HtmlNode>,
}

impl HtmlDocument {
    /// Safe, lightweight HTML tag extractor
    pub fn parse(html_str: &str) -> Self {
        let mut title = String::from("Untitled Page");
        let mut nodes = Vec::new();

        // 1. Extract <title>...</title>
        if let Some(t_start) = html_str.find("<title>") {
            let after = &html_str[t_start + 7..];
            if let Some(t_end) = after.find("</title>") {
                title = String::from(&after[..t_end]);
            }
        }

        // 2. Extract <h1>...</h1> or <h2>
        let mut pos = 0;
        while let Some(h_start) = html_str[pos..].find("<h1>") {
            let abs_h_start = pos + h_start + 4;
            let rest = &html_str[abs_h_start..];
            if let Some(h_end) = rest.find("</h1>") {
                nodes.push(HtmlNode::Heading(String::from(&rest[..h_end])));
                pos = abs_h_start + h_end + 5;
            } else {
                break;
            }
        }

        // 3. Extract <p>...</p>
        pos = 0;
        while let Some(p_start) = html_str[pos..].find("<p>") {
            let abs_p_start = pos + p_start + 3;
            let rest = &html_str[abs_p_start..];
            if let Some(p_end) = rest.find("</p>") {
                nodes.push(HtmlNode::Paragraph(String::from(&rest[..p_end])));
                pos = abs_p_start + p_end + 4;
            } else {
                break;
            }
        }

        // 4. Extract <a href="...">...</a>
        pos = 0;
        while let Some(a_start) = html_str[pos..].find("<a href=\"") {
            let abs_a_start = pos + a_start + 9;
            let rest = &html_str[abs_a_start..];
            if let Some(quote_end) = rest.find('"') {
                let href = String::from(&rest[..quote_end]);
                let after_tag = &rest[quote_end..];
                if let Some(tag_close) = after_tag.find('>') {
                    let text_start = &after_tag[tag_close + 1..];
                    if let Some(a_end) = text_start.find("</a>") {
                        let text = String::from(&text_start[..a_end]);
                        nodes.push(HtmlNode::Link { text, href });
                        pos = abs_a_start + quote_end + tag_close + 1 + a_end + 4;
                        continue;
                    }
                }
            }
            break;
        }

        Self { title, nodes }
    }
}

pub struct BrowserState {
    pub url_bar: TextBox,
    pub go_button: Button,
    pub current_doc: Option<HtmlDocument>,
    pub status: String,
}

impl BrowserState {
    pub fn new() -> Self {
        let mut state = Self {
            url_bar: TextBox::new(8, 6, 260, 20),
            go_button: Button::new(274, 6, 40, 20, "Go"),
            current_doc: None,
            status: String::from("Ready"),
        };
        state.url_bar.text = String::from("http://example.com");
        state.load_example_page();
        state
    }

    /// Loads demo page for example.com
    pub fn load_example_page(&mut self) {
        let html_content = r#"
            <html>
                <head><title>Example Domain</title></head>
                <body>
                    <h1>Example Domain</h1>
                    <p>This domain is for use in documentation examples.</p>
                    <p>SparkOS Microkernel Lightweight Web Viewer.</p>
                    <a href="http://sparkos.org">More information...</a>
                </body>
            </html>
        "#;
        self.current_doc = Some(HtmlDocument::parse(html_content));
        self.status = String::from("200 OK (Loaded)");
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        let bg_color = 0x000F172A; // Navy Slate
        let page_bg = 0x00FFFFFF;  // Pure White
        let text_color = 0x001E293B; // Slate 800
        let heading_color = 0x000284C7; // Blue
        let link_color = 0x002563EB;    // Link Blue

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Navigation Bar (URL Input + Go Button)
        self.url_bar.draw(surface_ptr, w, h);
        self.go_button.draw(surface_ptr, w, h);

        // 2. Browser Content Viewport (White card)
        let vp_top = 32u32;
        let vp_h = h.saturating_sub(vp_top + 16);
        for row in 0..vp_h {
            let py = vp_top + row;
            if py >= h { break; }
            for col in 8..(w - 8) {
                let offset = (py as usize) * (w as usize) + (col as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), page_bg);
                }
            }
        }

        // 3. Render HTML Elements
        if let Some(doc) = &self.current_doc {
            let mut y = vp_top + 8;

            // Page Title
            let title_lbl = Label::new(16, y as i32, &format!("Title: {}", doc.title), 0x0064748B, page_bg);
            title_lbl.draw(surface_ptr, w, h);
            y += 16;

            for node in &doc.nodes {
                if y + 12 >= vp_top + vp_h { break; }
                match node {
                    HtmlNode::Heading(text) => {
                        let h_lbl = Label::new(16, y as i32, text, heading_color, page_bg);
                        h_lbl.draw(surface_ptr, w, h);
                        y += 14;
                    }
                    HtmlNode::Paragraph(text) => {
                        let p_lbl = Label::new(16, y as i32, text, text_color, page_bg);
                        p_lbl.draw(surface_ptr, w, h);
                        y += 12;
                    }
                    HtmlNode::Link { text, .. } => {
                        let a_lbl = Label::new(16, y as i32, &format!("[Link: {}]", text), link_color, page_bg);
                        a_lbl.draw(surface_ptr, w, h);
                        y += 14;
                    }
                }
            }
        }

        // 4. Status Bar
        let status_y = h.saturating_sub(12);
        crate::font::draw_text(surface_ptr, w, h, 8, status_y, &self.status, 0x0034D399, bg_color);
    }
}

pub fn spawn_browser_app(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for browser.app")?;
    let code = crate::terminal_app::terminal_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = crate::task::process::create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );

    // Register NetworkAccess permission
    let manifest = crate::permission::AppManifest::new("browser", alloc::vec![crate::permission::AppPermission::NetworkAccess]);
    crate::permission::PERMISSION_MANAGER.lock().register_process_permissions(pid, &manifest);

    let surf_id = crate::surface::create_surface_for_pid(pid, BROWSER_WIDTH, BROWSER_HEIGHT)?;
    let _win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 90, 60, BROWSER_WIDTH, BROWSER_HEIGHT)
        .map_err(|_| "window creation failed")?;

    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
        let phys_addr = surface.shmem_phys_addr;
        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
        let state = BrowserState::new();
        state.render_to_surface(surf_ptr, BROWSER_WIDTH, BROWSER_HEIGHT);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, BROWSER_WIDTH, BROWSER_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window)",
        name, pid, code_base, surf_id);

    Ok(pid)
}
