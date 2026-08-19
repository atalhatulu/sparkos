//! SparkOS Desktop V2.2 — Classic Minimalist Web Browser (`src/browser_app.rs`)
//!
//! Inspired by classic early desktop browsers (Mosaic, Netscape 1.0, IE 3.0):
//! - Clean & Simple Navigation Bar: Back (<), Forward (>), Reload (R), Home (H), URL/Search Bar, Clear (x), Go (Search)
//! - Smart Address / Search Box: Direct URL navigation + Automatic Google Search fallback
//! - Sequential HTML DOM Parser (Headings, Paragraphs, Links, Lists, Code blocks, Horizontal rules, Buttons)
//! - Clickable hyperlinks with layout hit-testing
//! - Keyboard navigation (scancode editing, Backspace, Delete, Arrows, Home/End, Enter, Page Up/Down scrolling)
//! - Multi-window isolated browser instances with independent navigation history & scroll
//! - Bottom Status Bar: Loading state, HTTP status, and active page title

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

pub const BROWSER_WIDTH: u32 = 480;
pub const BROWSER_HEIGHT: u32 = 300;
pub const MAX_HTML_TOKEN_LEN: usize = 4096;

pub static BROWSER_INSTANCES: Mutex<BTreeMap<u64, BrowserState>> = Mutex::new(BTreeMap::new());

// ---------------------------------------------------------------------------
// 1. Structured HTML DOM Tree & Sequential Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlBlock {
    Heading(u8, String),
    Paragraph(String),
    Link { text: String, href: String },
    ListItem(String),
    CodeBlock(String),
    HorizontalRule,
    Button(String),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct HtmlDocument {
    pub title: String,
    pub blocks: Vec<HtmlBlock>,
}

impl HtmlDocument {
    pub fn empty() -> Self {
        Self {
            title: String::from("New Tab"),
            blocks: Vec::new(),
        }
    }

    /// Safe sequential HTML token parser
    pub fn parse(html_str: &str) -> Self {
        let mut title = String::from("SparkOS Web Page");
        let mut blocks = Vec::new();

        // 1. Extract <title>...</title>
        if let Some(t_start) = html_str.find("<title>") {
            let after = &html_str[t_start + 7..];
            if let Some(t_end) = after.find("</title>") {
                let end_bounded = t_end.min(MAX_HTML_TOKEN_LEN);
                title = String::from(after[..end_bounded].trim());
            }
        }

        // 2. Sequential Tag Lexer & Parser
        let mut cursor = 0;
        let bytes = html_str.as_bytes();
        let len = bytes.len();

        while cursor < len {
            if let Some(tag_open) = html_str[cursor..].find('<') {
                let abs_open = cursor + tag_open;
                // Accumulate loose text before tag
                if abs_open > cursor {
                    let loose_text = html_str[cursor..abs_open].trim();
                    if !loose_text.is_empty() && !loose_text.starts_with('<') {
                        blocks.push(HtmlBlock::Text(String::from(loose_text)));
                    }
                }

                if let Some(tag_close) = html_str[abs_open..].find('>') {
                    let abs_close = abs_open + tag_close;
                    let tag_content = html_str[abs_open + 1..abs_close].trim();

                    // Skip structural & closing tags
                    if tag_content.starts_with('/') || tag_content.starts_with("html") ||
                       tag_content.starts_with("head") || tag_content.starts_with("body") ||
                       tag_content.starts_with("ul") {
                        cursor = abs_close + 1;
                        continue;
                    }

                    // Skip <title>...</title> block in body parser
                    if tag_content.starts_with("title") {
                        let rest = &html_str[abs_close + 1..];
                        if let Some(end_idx) = rest.find("</title>") {
                            cursor = abs_close + 1 + end_idx + 8;
                            continue;
                        }
                        cursor = abs_close + 1;
                        continue;
                    }

                    // Parse Headings <h1> .. <h6>
                    if (tag_content.starts_with("h1") || tag_content.starts_with("h2") || tag_content.starts_with("h3") ||
                        tag_content.starts_with("h4") || tag_content.starts_with("h5") || tag_content.starts_with("h6")) &&
                        !tag_content.starts_with('/') {
                        let level = (tag_content.as_bytes()[1] - b'0').clamp(1, 6);
                        let close_tag = format!("</h{}>", level);
                        let rest = &html_str[abs_close + 1..];
                        if let Some(end_idx) = rest.find(&close_tag) {
                            let heading_text = rest[..end_idx.min(MAX_HTML_TOKEN_LEN)].trim();
                            blocks.push(HtmlBlock::Heading(level, String::from(heading_text)));
                            cursor = abs_close + 1 + end_idx + close_tag.len();
                            continue;
                        }
                    }

                    // Parse Paragraphs <p>
                    if tag_content.starts_with('p') && !tag_content.starts_with('/') && (tag_content.len() == 1 || tag_content.chars().nth(1) == Some(' ')) {
                        let rest = &html_str[abs_close + 1..];
                        if let Some(end_idx) = rest.find("</p>") {
                            let p_text = rest[..end_idx.min(MAX_HTML_TOKEN_LEN)].trim();
                            blocks.push(HtmlBlock::Paragraph(String::from(p_text)));
                            cursor = abs_close + 1 + end_idx + 4;
                            continue;
                        }
                    }

                    // Parse Hyperlinks <a href="...">
                    if tag_content.starts_with("a ") && tag_content.contains("href=") {
                        let mut href = String::from("#");
                        if let Some(h_pos) = tag_content.find("href=\"") {
                            let after_h = &tag_content[h_pos + 6..];
                            if let Some(q_end) = after_h.find('"') {
                                href = String::from(&after_h[..q_end]);
                            }
                        } else if let Some(h_pos) = tag_content.find("href='") {
                            let after_h = &tag_content[h_pos + 6..];
                            if let Some(q_end) = after_h.find('\'') {
                                href = String::from(&after_h[..q_end]);
                            }
                        }

                        let rest = &html_str[abs_close + 1..];
                        if let Some(end_idx) = rest.find("</a>") {
                            let link_text = rest[..end_idx.min(MAX_HTML_TOKEN_LEN)].trim();
                            blocks.push(HtmlBlock::Link {
                                text: String::from(link_text),
                                href,
                            });
                            cursor = abs_close + 1 + end_idx + 4;
                            continue;
                        }
                    }

                    // Parse List Items <li>
                    if tag_content.starts_with("li") && !tag_content.starts_with('/') {
                        let rest = &html_str[abs_close + 1..];
                        if let Some(end_idx) = rest.find("</li>") {
                            let li_text = rest[..end_idx.min(MAX_HTML_TOKEN_LEN)].trim();
                            blocks.push(HtmlBlock::ListItem(String::from(li_text)));
                            cursor = abs_close + 1 + end_idx + 5;
                            continue;
                        }
                    }

                    // Parse Code/Pre <pre> or <code>
                    if (tag_content.starts_with("pre") || tag_content.starts_with("code")) && !tag_content.starts_with('/') {
                        let close_tag = if tag_content.starts_with("pre") { "</pre>" } else { "</code>" };
                        let rest = &html_str[abs_close + 1..];
                        if let Some(end_idx) = rest.find(close_tag) {
                            let code_text = &rest[..end_idx.min(MAX_HTML_TOKEN_LEN)];
                            blocks.push(HtmlBlock::CodeBlock(String::from(code_text)));
                            cursor = abs_close + 1 + end_idx + close_tag.len();
                            continue;
                        }
                    }

                    // Parse Horizontal Rule <hr>
                    if tag_content.starts_with("hr") {
                        blocks.push(HtmlBlock::HorizontalRule);
                        cursor = abs_close + 1;
                        continue;
                    }

                    // Parse Buttons <button>
                    if tag_content.starts_with("button") && !tag_content.starts_with('/') {
                        let rest = &html_str[abs_close + 1..];
                        if let Some(end_idx) = rest.find("</button>") {
                            let btn_text = rest[..end_idx.min(MAX_HTML_TOKEN_LEN)].trim();
                            blocks.push(HtmlBlock::Button(String::from(btn_text)));
                            cursor = abs_close + 1 + end_idx + 9;
                            continue;
                        }
                    }

                    cursor = abs_close + 1;
                } else {
                    break;
                }
            } else {
                let rest_text = html_str[cursor..].trim();
                if !rest_text.is_empty() {
                    blocks.push(HtmlBlock::Text(String::from(rest_text)));
                }
                break;
            }
        }

        Self { title, blocks }
    }
}

// ---------------------------------------------------------------------------
// 2. Browser Instance State & Smart Search Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKeyResult {
    Ignored,
    ToolbarChanged,
    FullPageChanged,
}

#[derive(Debug, Clone)]
pub struct HyperlinkHitbox {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub href: String,
}

#[derive(Debug, Clone)]
pub struct BrowserState {
    pub window_id: u64,
    pub url_input: String,
    pub active_url: String,
    pub cursor_pos: usize,
    pub url_bar_focused: bool,
    pub history: Vec<String>,
    pub history_idx: usize,
    pub scroll_y: u32,
    pub max_scroll: u32,
    pub doc: HtmlDocument,
    pub status: String,
    pub loading: bool,
    pub links: Vec<HyperlinkHitbox>,
}

impl BrowserState {
    pub fn new(window_id: u64) -> Self {
        let mut state = Self {
            window_id,
            url_input: String::from("https://www.google.com"),
            active_url: String::from("https://www.google.com"),
            cursor_pos: 22,
            url_bar_focused: false,
            history: Vec::new(),
            history_idx: 0,
            scroll_y: 0,
            max_scroll: 0,
            doc: HtmlDocument::empty(),
            status: String::from("Ready"),
            loading: false,
            links: Vec::new(),
        };
        state.load_url("https://www.google.com");
        state
    }

    /// Smart navigation: converts search queries into Google search or loads direct URLs
    pub fn navigate_to_input(&mut self) {
        let raw_query = self.url_input.trim();
        if raw_query.is_empty() { return; }

        let resolved_url = if raw_query.starts_with("http://")
            || raw_query.starts_with("https://")
            || raw_query.starts_with("about:")
            || raw_query.starts_with("file://")
            || raw_query.starts_with("sparkos://") {
            String::from(raw_query)
        } else if is_direct_domain(raw_query) {
            format!("https://{}", raw_query)
        } else {
            // Default search engine: Google Search
            format!("https://www.google.com/search?q={}", raw_query)
        };

        self.load_url(&resolved_url);
    }

    /// Loads a URL and updates history
    pub fn load_url(&mut self, url: &str) {
        let clean_url = url.trim();
        if clean_url.is_empty() { return; }

        if self.history.is_empty() || self.history[self.history_idx] != clean_url {
            if self.history_idx + 1 < self.history.len() {
                self.history.truncate(self.history_idx + 1);
            }
            self.history.push(String::from(clean_url));
            self.history_idx = self.history.len() - 1;
        }

        self.active_url = String::from(clean_url);
        self.url_input = String::from(clean_url);
        self.cursor_pos = self.url_input.len();
        self.scroll_y = 0;

        let content = self.fetch_content(clean_url);
        self.doc = HtmlDocument::parse(&content);
        self.status = format!("Loaded {} blocks (200 OK)", self.doc.blocks.len());
        self.loading = false;
    }

    /// Resolves URL content from internal pages, Google Search Engine, or simulated network
    pub fn fetch_content(&self, url: &str) -> String {
        // 1. Google Search Results Page
        if let Some(q_pos) = url.find("google.com/search?q=") {
            let query = &url[q_pos + 20..];
            let clean_q = query.replace('+', " ");
            return format!(r#"
                <html>
                <head><title>{} - Google Search</title></head>
                <body>
                    <h1>Google</h1>
                    <p>Search Results for: <strong>"{}"</strong></p>
                    <hr>
                    <h2>1. {} - Official Portal</h2>
                    <p>Discover latest information, documentation, and resources about {}.</p>
                    <a href="https://www.google.com/search?q={}+wiki">https://en.wikipedia.org/wiki/{}</a>
                    <hr>
                    <h2>2. {} - SparkOS Community Hub</h2>
                    <p>Fast, microkernel-native operating system articles, discussions, and open-source packages.</p>
                    <a href="about:sparkos">https://sparkos.org/community/{}</a>
                    <hr>
                    <h2>3. {} - Online Developer Reference</h2>
                    <p>Authoritative guides, API contracts, tutorials, and developer documentation.</p>
                    <a href="about:help">https://sparkos.org/docs/{}</a>
                    <hr>
                    <button>Next Page &gt;</button>
                    <p></p>
                    <a href="https://www.google.com">Back to Google Home</a>
                </body>
                </html>
            "#, clean_q, clean_q, clean_q, clean_q, clean_q, clean_q, clean_q, clean_q, clean_q, clean_q);
        }

        // 2. Google Home Page
        if url == "https://www.google.com" || url == "http://www.google.com" || url == "https://google.com" || url == "http://google.com" {
            return String::from(r#"
                <html>
                <head><title>Google</title></head>
                <body>
                    <h1>Google</h1>
                    <p>Search the world's information, including webpages, images, videos and more.</p>
                    <hr>
                    <p><strong>Quick Searches & Portals:</strong></p>
                    <a href="https://www.google.com/search?q=sparkos+operating+system">SparkOS Operating System</a>
                    <p></p>
                    <a href="https://www.google.com/search?q=rust+programming+language">Rust Programming Language</a>
                    <p></p>
                    <a href="https://www.google.com/search?q=microkernel+architecture">Microkernel Architecture</a>
                    <p></p>
                    <a href="about:sparkos">SparkOS System Portal</a>
                    <hr>
                    <button>Google Search</button>
                    <button>I'm Feeling Lucky</button>
                </body>
                </html>
            "#);
        }

        // 3. Native Pages
        match url {
            "about:sparkos" | "sparkos://home" => {
                String::from(r#"
                    <html>
                    <head><title>SparkOS Portal</title></head>
                    <body>
                        <h1>SparkOS Web Portal 2.0</h1>
                        <p>High-performance, memory-safe microkernel operating system written in pure Rust.</p>
                        <hr>
                        <h2>System Features</h2>
                        <ul>
                            <li>Preemptive Multitasking & Capability Security</li>
                            <li>60 FPS Decoupled Window Compositor with Damage Tracking</li>
                            <li>Pure Rust Zero-Allocation Wire Input Pipeline</li>
                            <li>Multi-Instance Isolated Desktop Applications</li>
                        </ul>
                        <h2>Explore Web & Docs</h2>
                        <a href="https://www.google.com">Google Search Engine</a>
                        <p></p>
                        <a href="about:help">Open Browser Guide</a>
                        <hr>
                        <button>System Status: OK</button>
                    </body>
                    </html>
                "#)
            }
            "about:help" => {
                String::from(r#"
                    <html>
                    <head><title>Browser Help</title></head>
                    <body>
                        <h1>Browser Keyboard & Mouse Controls</h1>
                        <hr>
                        <p><strong>Search & Address Bar:</strong></p>
                        <ul>
                            <li>Type any search query & press Enter to Search with Google</li>
                            <li>Type direct URL (e.g. google.com or sparkos.org) to visit directly</li>
                            <li>Click '[x]' to clear address bar immediately</li>
                            <li>Click 'H' to return to Home (Google)</li>
                        </ul>
                        <p><strong>Navigation:</strong></p>
                        <ul>
                            <li>Click '&lt;' button to go Back</li>
                            <li>Click '&gt;' button to go Forward</li>
                            <li>Click 'R' to Reload current page</li>
                        </ul>
                        <p><strong>Scrolling:</strong></p>
                        <ul>
                            <li>Page Up / Page Down: Fast Scroll</li>
                            <li>Up / Down Arrow Keys: Smooth Scroll</li>
                            <li>Click Hyperlinks to navigate</li>
                        </ul>
                        <hr>
                        <a href="https://www.google.com">Go to Google</a>
                    </body>
                    </html>
                "#)
            }
            "about:blank" => {
                String::from("<html><head><title>Blank</title></head><body><h1>about:blank</h1></body></html>")
            }
            _ if url.starts_with("http://") || url.starts_with("https://") => {
                format!(r#"
                    <html>
                    <head><title>{}</title></head>
                    <body>
                        <h1>Simulated Web Response</h1>
                        <p>Connected to <strong>{}</strong> via SparkOS TCP Socket Engine.</p>
                        <hr>
                        <p>HTTP/1.1 200 OK</p>
                        <p>Content-Type: text/html; charset=UTF-8</p>
                        <pre>Server: SparkNet/1.0
Connection: keep-alive</pre>
                        <hr>
                        <a href="https://www.google.com">Search on Google</a>
                    </body>
                    </html>
                "#, url, url)
            }
            _ => {
                format!(r#"
                    <html>
                    <head><title>Page Not Found</title></head>
                    <body>
                        <h1>404 Not Found</h1>
                        <p>The requested URL <code>{}</code> could not be resolved.</p>
                        <hr>
                        <a href="https://www.google.com">Search on Google</a>
                    </body>
                    </html>
                "#, url)
            }
        }
    }

    /// Navigation history: Go Back
    pub fn navigate_back(&mut self) {
        if self.history_idx > 0 {
            self.history_idx -= 1;
            let target = self.history[self.history_idx].clone();
            self.load_url(&target);
        }
    }

    /// Navigation history: Go Forward
    pub fn navigate_forward(&mut self) {
        if self.history_idx + 1 < self.history.len() {
            self.history_idx += 1;
            let target = self.history[self.history_idx].clone();
            self.load_url(&target);
        }
    }

    /// Reload current page
    pub fn reload(&mut self) {
        let current = self.active_url.clone();
        self.load_url(&current);
    }

    /// Go to Home (Google)
    pub fn go_home(&mut self) {
        self.load_url("https://www.google.com");
    }

    /// Clears search/URL input bar
    pub fn clear_url_input(&mut self) {
        self.url_input.clear();
        self.cursor_pos = 0;
        self.url_bar_focused = true;
    }

    /// Scrolls the viewport up or down
    pub fn scroll_by(&mut self, delta: i32) {
        if delta < 0 {
            self.scroll_y = self.scroll_y.saturating_sub((-delta) as u32);
        } else {
            self.scroll_y = (self.scroll_y + delta as u32).min(self.max_scroll);
        }
    }

    // -----------------------------------------------------------------------
    // Mouse & Keyboard Input Dispatch
    // -----------------------------------------------------------------------

    pub fn handle_mouse_click(&mut self, mx: u32, my: u32) -> bool {
        // 1. Classic Toolbar Hit-testing (y in 4..26)
        if my >= 4 && my <= 26 {
            // Back Button '<' (x: 6..30)
            if mx >= 6 && mx <= 30 {
                self.navigate_back();
                return true;
            }
            // Forward Button '>' (x: 34..58)
            if mx >= 34 && mx <= 58 {
                self.navigate_forward();
                return true;
            }
            // Reload Button 'R' (x: 62..86)
            if mx >= 62 && mx <= 86 {
                self.reload();
                return true;
            }
            // Home Button 'H' (x: 90..114)
            if mx >= 90 && mx <= 114 {
                self.go_home();
                return true;
            }

            let search_btn_w = 44u32;
            let bar_start_x = 120u32;
            let bar_end_x = BROWSER_WIDTH.saturating_sub(search_btn_w + 8);

            // Clear Button '×' (inside URL bar on the right: bar_end_x - 20 .. bar_end_x - 2)
            if mx >= bar_end_x.saturating_sub(20) && mx <= bar_end_x - 2 {
                self.clear_url_input();
                return true;
            }

            // Search/URL Input Bar Area
            if mx >= bar_start_x && mx < bar_end_x.saturating_sub(20) {
                self.url_bar_focused = true;
                let rel_x = mx.saturating_sub(bar_start_x + 6);
                let char_idx = (rel_x / 8) as usize;
                self.cursor_pos = char_idx.min(self.url_input.len());
                return true;
            }

            // Search / Go Button [Go] (x: bar_end_x + 4 .. w - 4)
            if mx >= bar_end_x + 4 && mx <= BROWSER_WIDTH - 4 {
                self.navigate_to_input();
                self.url_bar_focused = false;
                return true;
            }
        }

        // 2. Click outside URL bar defocusses URL bar
        self.url_bar_focused = false;

        // 3. Content Area & Hyperlink Hit-testing
        let content_y = my as i32;
        let content_x = mx as i32;

        for link in &self.links {
            if content_x >= link.x && content_x <= link.x + (link.width as i32) &&
               content_y >= link.y && content_y <= link.y + (link.height as i32) {
                let target_href = link.href.clone();
                self.load_url(&target_href);
                return true;
            }
        }

        false
    }

    pub fn handle_key_input(&mut self, scancode: u8, is_ctrl: bool, is_shift: bool) -> BrowserKeyResult {
        if self.url_bar_focused {
            match scancode {
                0x1C => { // Enter -> Search with Google or Navigate to URL
                    self.navigate_to_input();
                    self.url_bar_focused = false;
                    BrowserKeyResult::FullPageChanged
                }
                0x01 => { // Esc -> Restore active URL and unfocus
                    self.url_input = self.active_url.clone();
                    self.url_bar_focused = false;
                    BrowserKeyResult::ToolbarChanged
                }
                0x0E => { // Backspace
                    if self.cursor_pos > 0 && !self.url_input.is_empty() {
                        self.url_input.remove(self.cursor_pos - 1);
                        self.cursor_pos -= 1;
                    }
                    BrowserKeyResult::ToolbarChanged
                }
                0x53 => { // Delete
                    if self.cursor_pos < self.url_input.len() {
                        self.url_input.remove(self.cursor_pos);
                    }
                    BrowserKeyResult::ToolbarChanged
                }
                0x4B => { // Left Arrow
                    self.cursor_pos = self.cursor_pos.saturating_sub(1);
                    BrowserKeyResult::ToolbarChanged
                }
                0x4D => { // Right Arrow
                    self.cursor_pos = (self.cursor_pos + 1).min(self.url_input.len());
                    BrowserKeyResult::ToolbarChanged
                }
                0x47 => { // Home
                    self.cursor_pos = 0;
                    BrowserKeyResult::ToolbarChanged
                }
                0x4F => { // End
                    self.cursor_pos = self.url_input.len();
                    BrowserKeyResult::ToolbarChanged
                }
                _ => {
                    // Try converting PS/2 scancode to ASCII char
                    if let Some(c) = scancode_to_ascii_char(scancode, is_shift) {
                        if c >= ' ' && c <= '~' {
                            self.url_input.insert(self.cursor_pos, c);
                            self.cursor_pos += 1;
                            return BrowserKeyResult::ToolbarChanged;
                        }
                    }
                    BrowserKeyResult::Ignored
                }
            }
        } else {
            // Viewport Scroll Controls & Shortcuts
            match scancode {
                0x49 => { // Page Up
                    self.scroll_by(-60);
                    BrowserKeyResult::FullPageChanged
                }
                0x51 => { // Page Down
                    self.scroll_by(60);
                    BrowserKeyResult::FullPageChanged
                }
                0x48 => { // Up Arrow
                    self.scroll_by(-20);
                    BrowserKeyResult::FullPageChanged
                }
                0x50 => { // Down Arrow
                    self.scroll_by(20);
                    BrowserKeyResult::FullPageChanged
                }
                0x13 if is_ctrl => { // Ctrl+R -> Reload
                    self.reload();
                    BrowserKeyResult::FullPageChanged
                }
                0x26 if is_ctrl => { // Ctrl+L -> Focus Address/Search Bar
                    self.url_bar_focused = true;
                    self.cursor_pos = self.url_input.len();
                    BrowserKeyResult::ToolbarChanged
                }
                0x1C => { // Enter on page -> Focus Search Bar if not focused
                    self.url_bar_focused = true;
                    BrowserKeyResult::ToolbarChanged
                }
                _ => {
                    // If user starts typing printable character, automatically focus search bar
                    if let Some(c) = scancode_to_ascii_char(scancode, is_shift) {
                        if c >= ' ' && c <= '~' {
                            self.url_input.clear();
                            self.url_input.push(c);
                            self.cursor_pos = 1;
                            self.url_bar_focused = true;
                            return BrowserKeyResult::ToolbarChanged;
                        }
                    }
                    BrowserKeyResult::Ignored
                }
            }
        }
    }

    pub fn render_toolbar_only(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }

        let bar_bg = 0x001E293B;       // Classic Top Toolbar
        let border_col = 0x00334155;

        // 1. Classic Navigation Toolbar (y = 0..30)
        draw_surf_rect(surface_ptr, w, h, 0, 0, w, 30, bar_bg);
        draw_surf_rect(surface_ptr, w, h, 0, 29, w, 1, border_col);

        // Back Button [<]
        let back_enabled = self.history_idx > 0;
        let back_bg = if back_enabled { 0x002563EB } else { 0x00334155 };
        draw_surf_rect(surface_ptr, w, h, 6, 4, 24, 22, back_bg);
        crate::font::draw_text(surface_ptr, w, h, 14, 8, "<", 0x00FFFFFF, back_bg);

        // Forward Button [>]
        let fwd_enabled = self.history_idx + 1 < self.history.len();
        let fwd_bg = if fwd_enabled { 0x002563EB } else { 0x00334155 };
        draw_surf_rect(surface_ptr, w, h, 34, 4, 24, 22, fwd_bg);
        crate::font::draw_text(surface_ptr, w, h, 42, 8, ">", 0x00FFFFFF, fwd_bg);

        // Reload Button [R]
        draw_surf_rect(surface_ptr, w, h, 62, 4, 24, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 70, 8, "R", 0x00E2E8F0, 0x00334155);

        // Home Button [H]
        draw_surf_rect(surface_ptr, w, h, 90, 4, 24, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 98, 8, "H", 0x0038BDF8, 0x00334155);

        // Smart Search / URL Address Input Bar
        let search_btn_w = 44u32;
        let bar_start_x = 120u32;
        let bar_end_x = w.saturating_sub(search_btn_w + 8);
        let url_w = bar_end_x.saturating_sub(bar_start_x);

        let url_bg = if self.url_bar_focused { 0x000F172A } else { 0x00020617 };
        let url_border = if self.url_bar_focused { 0x0038BDF8 } else { 0x00475569 };
        draw_surf_rect(surface_ptr, w, h, bar_start_x, 4, url_w, 22, url_bg);
        draw_surf_rect(surface_ptr, w, h, bar_start_x, 4, url_w, 1, url_border);
        draw_surf_rect(surface_ptr, w, h, bar_start_x, 25, url_w, 1, url_border);

        // Clear '[x]' button inside search bar
        let clear_x = bar_end_x.saturating_sub(18);
        if !self.url_input.is_empty() {
            crate::font::draw_text(surface_ptr, w, h, clear_x, 8, "x", 0x0094A3B8, url_bg);
        }

        // Draw URL / Search Query Text
        let max_visible_chars = (url_w.saturating_sub(26) / 8) as usize;
        let display_text = if self.url_input.len() > max_visible_chars {
            &self.url_input[self.url_input.len() - max_visible_chars..]
        } else {
            &self.url_input
        };
        crate::font::draw_text(surface_ptr, w, h, bar_start_x + 6, 8, display_text, 0x00F8FAFC, url_bg);

        // Cursor in URL / Search bar
        if self.url_bar_focused {
            let cursor_x = bar_start_x + 6 + (self.cursor_pos.min(display_text.len()) as u32) * 8;
            if cursor_x < clear_x - 2 {
                draw_surf_rect(surface_ptr, w, h, cursor_x, 7, 2, 16, 0x0038BDF8);
            }
        }

        // Search / Go Button [Go]
        draw_surf_rect(surface_ptr, w, h, bar_end_x + 4, 4, search_btn_w, 22, 0x0010B981);
        crate::font::draw_text(surface_ptr, w, h, bar_end_x + 14, 8, "Go", 0x00FFFFFF, 0x0010B981);
    }

    // -----------------------------------------------------------------------
    // 3. Rendering to Client Surface
    // -----------------------------------------------------------------------

    pub fn render_to_surface(&mut self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }

        let bg_color = 0x000F172A;     // Dark Slate
        let bar_bg = 0x001E293B;       // Classic Top Toolbar
        let page_bg = 0x00FFFFFF;      // Content Background (Pure White)
        let border_col = 0x00334155;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Classic Navigation Toolbar (y = 0..30)
        draw_surf_rect(surface_ptr, w, h, 0, 0, w, 30, bar_bg);
        draw_surf_rect(surface_ptr, w, h, 0, 29, w, 1, border_col);

        // Back Button [<]
        let back_enabled = self.history_idx > 0;
        let back_bg = if back_enabled { 0x002563EB } else { 0x00334155 };
        draw_surf_rect(surface_ptr, w, h, 6, 4, 24, 22, back_bg);
        crate::font::draw_text(surface_ptr, w, h, 14, 8, "<", 0x00FFFFFF, back_bg);

        // Forward Button [>]
        let fwd_enabled = self.history_idx + 1 < self.history.len();
        let fwd_bg = if fwd_enabled { 0x002563EB } else { 0x00334155 };
        draw_surf_rect(surface_ptr, w, h, 34, 4, 24, 22, fwd_bg);
        crate::font::draw_text(surface_ptr, w, h, 42, 8, ">", 0x00FFFFFF, fwd_bg);

        // Reload Button [R]
        draw_surf_rect(surface_ptr, w, h, 62, 4, 24, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 70, 8, "R", 0x00E2E8F0, 0x00334155);

        // Home Button [H]
        draw_surf_rect(surface_ptr, w, h, 90, 4, 24, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 98, 8, "H", 0x0038BDF8, 0x00334155);

        // Smart Search / URL Address Input Bar
        let search_btn_w = 44u32;
        let bar_start_x = 120u32;
        let bar_end_x = w.saturating_sub(search_btn_w + 8);
        let url_w = bar_end_x.saturating_sub(bar_start_x);

        let url_bg = if self.url_bar_focused { 0x000F172A } else { 0x00020617 };
        let url_border = if self.url_bar_focused { 0x0038BDF8 } else { 0x00475569 };
        draw_surf_rect(surface_ptr, w, h, bar_start_x, 4, url_w, 22, url_bg);
        draw_surf_rect(surface_ptr, w, h, bar_start_x, 4, url_w, 1, url_border);
        draw_surf_rect(surface_ptr, w, h, bar_start_x, 25, url_w, 1, url_border);

        // Clear '[x]' button inside search bar
        let clear_x = bar_end_x.saturating_sub(18);
        if !self.url_input.is_empty() {
            crate::font::draw_text(surface_ptr, w, h, clear_x, 8, "x", 0x0094A3B8, url_bg);
        }

        // Draw URL / Search Query Text
        let max_visible_chars = (url_w.saturating_sub(26) / 8) as usize;
        let display_text = if self.url_input.len() > max_visible_chars {
            &self.url_input[self.url_input.len() - max_visible_chars..]
        } else {
            &self.url_input
        };
        crate::font::draw_text(surface_ptr, w, h, bar_start_x + 6, 8, display_text, 0x00F8FAFC, url_bg);

        // Cursor in URL / Search bar
        if self.url_bar_focused {
            let cursor_x = bar_start_x + 6 + (self.cursor_pos.min(display_text.len()) as u32) * 8;
            if cursor_x < clear_x - 2 {
                draw_surf_rect(surface_ptr, w, h, cursor_x, 7, 2, 16, 0x0038BDF8);
            }
        }

        // Search / Go Button [Go]
        draw_surf_rect(surface_ptr, w, h, bar_end_x + 4, 4, search_btn_w, 22, 0x0010B981);
        crate::font::draw_text(surface_ptr, w, h, bar_end_x + 14, 8, "Go", 0x00FFFFFF, 0x0010B981);

        // 2. Web Viewport Card (y = 30 .. h - 18)
        let vp_top = 30u32;
        let vp_h = h.saturating_sub(vp_top + 18);
        draw_surf_rect(surface_ptr, w, h, 0, vp_top, w, vp_h, page_bg);

        // Clear dynamic hyperlinks list for hit-testing
        self.links.clear();

        // 3. Render HTML Blocks
        let mut cur_y = (vp_top as i32) + 8 - (self.scroll_y as i32);
        let left_pad = 16i32;
        let right_limit = (w as i32) - 24;

        for block in &self.doc.blocks {
            match block {
                HtmlBlock::Heading(level, text) => {
                    let color = match level {
                        1 => 0x000284C7, // Blue 600
                        2 => 0x000369A1, // Sky 700
                        _ => 0x001E293B, // Slate 800
                    };
                    if cur_y >= (vp_top as i32) - 16 && cur_y + 16 <= (vp_top + vp_h) as i32 {
                        crate::font::draw_text(surface_ptr, w, h, left_pad as u32, cur_y as u32, text, color, page_bg);
                    }
                    cur_y += 18;
                }
                HtmlBlock::Paragraph(text) => {
                    let wrapped_lines = wrap_text(text, ((right_limit - left_pad) / 8) as usize);
                    for line in wrapped_lines {
                        if cur_y >= (vp_top as i32) - 14 && cur_y + 14 <= (vp_top + vp_h) as i32 {
                            crate::font::draw_text(surface_ptr, w, h, left_pad as u32, cur_y as u32, &line, 0x00334155, page_bg);
                        }
                        cur_y += 14;
                    }
                    cur_y += 4;
                }
                HtmlBlock::Link { text, href } => {
                    let link_label = format!("🔗 {}", text);
                    let link_w = (link_label.len() as u32) * 8;
                    if cur_y >= (vp_top as i32) - 14 && cur_y + 14 <= (vp_top + vp_h) as i32 {
                        crate::font::draw_text(surface_ptr, w, h, left_pad as u32, cur_y as u32, &link_label, 0x002563EB, page_bg);
                        draw_surf_rect(surface_ptr, w, h, left_pad as u32, (cur_y + 13) as u32, link_w, 1, 0x002563EB);
                    }
                    self.links.push(HyperlinkHitbox {
                        x: left_pad,
                        y: cur_y,
                        width: link_w,
                        height: 14,
                        href: href.clone(),
                    });
                    cur_y += 16;
                }
                HtmlBlock::ListItem(text) => {
                    let bullet = format!("• {}", text);
                    if cur_y >= (vp_top as i32) - 14 && cur_y + 14 <= (vp_top + vp_h) as i32 {
                        crate::font::draw_text(surface_ptr, w, h, (left_pad + 8) as u32, cur_y as u32, &bullet, 0x001E293B, page_bg);
                    }
                    cur_y += 14;
                }
                HtmlBlock::CodeBlock(code) => {
                    let box_top = cur_y;
                    let lines: Vec<&str> = code.lines().collect();
                    let box_h = (lines.len() as u32) * 14 + 8;
                    if box_top >= (vp_top as i32) - (box_h as i32) && box_top <= (vp_top + vp_h) as i32 {
                        draw_surf_rect(surface_ptr, w, h, left_pad as u32, box_top as u32, (right_limit - left_pad) as u32, box_h, 0x00F1F5F9);
                        for (i, l) in lines.iter().enumerate() {
                            crate::font::draw_text(surface_ptr, w, h, (left_pad + 6) as u32, (box_top + 4 + (i as i32 * 14)) as u32, l, 0x000F172A, 0x00F1F5F9);
                        }
                    }
                    cur_y += (box_h as i32) + 6;
                }
                HtmlBlock::HorizontalRule => {
                    if cur_y >= (vp_top as i32) && cur_y <= (vp_top + vp_h) as i32 {
                        draw_surf_rect(surface_ptr, w, h, left_pad as u32, cur_y as u32 + 4, (right_limit - left_pad) as u32, 1, 0x00E2E8F0);
                    }
                    cur_y += 10;
                }
                HtmlBlock::Button(label) => {
                    let btn_w = (label.len() as u32) * 8 + 16;
                    if cur_y >= (vp_top as i32) - 18 && cur_y + 18 <= (vp_top + vp_h) as i32 {
                        draw_surf_rect(surface_ptr, w, h, left_pad as u32, cur_y as u32, btn_w, 18, 0x00E2E8F0);
                        crate::font::draw_text(surface_ptr, w, h, (left_pad + 8) as u32, (cur_y + 3) as u32, label, 0x000F172A, 0x00E2E8F0);
                    }
                    cur_y += 22;
                }
                HtmlBlock::Text(text) => {
                    if cur_y >= (vp_top as i32) - 12 && cur_y + 12 <= (vp_top + vp_h) as i32 {
                        crate::font::draw_text(surface_ptr, w, h, left_pad as u32, cur_y as u32, text, 0x00475569, page_bg);
                    }
                    cur_y += 14;
                }
            }
        }

        // Calculate max scroll height
        let total_content_h = (cur_y + (self.scroll_y as i32) - (vp_top as i32)).max(0) as u32;
        self.max_scroll = total_content_h.saturating_sub(vp_h);

        // 4. Scroll Indicator Bar
        if self.max_scroll > 0 {
            let bar_h = ((vp_h * vp_h) / total_content_h).max(16);
            let bar_y = vp_top + ((self.scroll_y * (vp_h - bar_h)) / self.max_scroll);
            draw_surf_rect(surface_ptr, w, h, w - 6, bar_y, 4, bar_h, 0x0094A3B8);
        }

        // 5. Status Bar (y = h - 18 .. h)
        let status_y = h.saturating_sub(18);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 18, bar_bg);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 1, border_col);
        crate::font::draw_text(surface_ptr, w, h, 8, status_y + 4, &self.status, 0x0034D399, bar_bg);

        let doc_title = format!("Title: {}", self.doc.title);
        let title_x = w.saturating_sub((doc_title.len() as u32) * 8 + 8);
        if title_x > 180 {
            crate::font::draw_text(surface_ptr, w, h, title_x, status_y + 4, &doc_title, 0x0094A3B8, bar_bg);
        }
    }
}

pub fn draw_surf_rect(surface_ptr: *mut u32, surf_w: u32, surf_h: u32, x: u32, y: u32, rw: u32, rh: u32, color: u32) {
    if surface_ptr.is_null() { return; }
    for r in 0..rh {
        let py = y + r;
        if py >= surf_h { break; }
        for c in 0..rw {
            let px = x + c;
            if px >= surf_w { break; }
            let offset = (py as usize) * (surf_w as usize) + (px as usize);
            unsafe {
                core::ptr::write_volatile(surface_ptr.add(offset), color);
            }
        }
    }
}

/// Helper to detect if a raw string is a domain name (e.g. google.com, sparkos.org)
fn is_direct_domain(s: &str) -> bool {
    if s.contains(' ') { return false; }
    let suffixes = [".com", ".org", ".net", ".io", ".edu", ".gov", ".tr", ".dev", ".me", ".ai"];
    for suffix in &suffixes {
        if s.ends_with(suffix) || s.contains(&format!("{}/", suffix)) {
            return true;
        }
    }
    false
}

/// Converts PS/2 scancode to ASCII char
fn scancode_to_ascii_char(scancode: u8, shift: bool) -> Option<char> {
    const MAP_NORMAL: [u8; 128] = [
        0,    0,   b'1', b'2', b'3', b'4', b'5', b'6',
        b'7', b'8', b'9', b'0', b'-', b'=', 0,    0,
        b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
        b'o', b'p', b'[', b']', 0,    0,   b'a', b's',
        b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';',
        b'\'',b'`', 0,   b'\\',b'z', b'x', b'c', b'v',
        b'b', b'n', b'm', b',', b'.', b'/', 0,   b'*',
        0,   b' ', 0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        b'7', b'8', b'9', b'-', b'4', b'5', b'6', b'+',
        b'1', b'2', b'3', b'0', b'.', 0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
    ];

    const MAP_SHIFT: [u8; 128] = [
        0,    0,   b'!', b'@', b'#', b'$', b'%', b'^',
        b'&', b'*', b'(', b')', b'_', b'+', 0,    0,
        b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',
        b'O', b'P', b'{', b'}', 0,    0,   b'A', b'S',
        b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':',
        b'"', b'~', 0,   b'|', b'Z', b'X', b'C', b'V',
        b'B', b'N', b'M', b'<', b'>', b'?', 0,   b'*',
        0,   b' ', 0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        b'7', b'8', b'9', b'-', b'4', b'5', b'6', b'+',
        b'1', b'2', b'3', b'0', b'.', 0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
        0,    0,    0,    0,    0,    0,    0,    0,
    ];

    let idx = (scancode & 0x7F) as usize;
    let b = if shift { MAP_SHIFT[idx] } else { MAP_NORMAL[idx] };
    if b != 0 { Some(b as char) } else { None }
}

/// Simple word-wrapping helper for fixed width fonts
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if max_chars == 0 { return lines; }

    let mut current_line = String::new();
    for word in text.split_whitespace() {
        if current_line.len() + word.len() + 1 > max_chars {
            if !current_line.is_empty() {
                lines.push(current_line);
                current_line = String::new();
            }
            if word.len() > max_chars {
                lines.push(String::from(word));
                continue;
            }
        }
        if !current_line.is_empty() {
            current_line.push(' ');
        }
        current_line.push_str(word);
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    lines
}

// ---------------------------------------------------------------------------
// 4. Process & Window Lifecycle
// ---------------------------------------------------------------------------

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
    let win_id = crate::wm::WM.lock()
        .create_window_with_meta(pid, surf_id, 90, 60, BROWSER_WIDTH, BROWSER_HEIGHT, String::from("Google - Browser"), crate::app_registry::AppIcon::Browser)
        .map_err(|_| "window creation failed")?;

    let mut state = BrowserState::new(win_id);

    if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == surf_id) {
        let phys_addr = surface.shmem_phys_addr;
        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
        state.render_to_surface(surf_ptr, BROWSER_WIDTH, BROWSER_HEIGHT);
    }

    BROWSER_INSTANCES.lock().insert(win_id, state);

    let _ = crate::surface::present_surface(surf_id, 0, 0, BROWSER_WIDTH, BROWSER_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window {})",
        name, pid, code_base, surf_id, win_id);

    Ok(pid)
}

pub fn cleanup_browser_for_window(window_id: u64) {
    BROWSER_INSTANCES.lock().remove(&window_id);
}
