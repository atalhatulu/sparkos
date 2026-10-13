//! SparkOS Desktop V1.14 — Damage Tracking & Dirty Rectangle Compositor
//!
//! Provides region clipping, dirty rectangle aggregation, and optimized partial
//! compositor redraws to minimize memory bandwidth overhead.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRegion {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DamageRegion {
    pub const fn empty() -> Self {
        Self { x: 0, y: 0, width: 0, height: 0 }
    }

    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Combines two damage rectangles into their bounding box.
    pub fn union(&self, other: &DamageRegion) -> DamageRegion {
        if self.is_empty() { return *other; }
        if other.is_empty() { return *self; }

        let min_x = self.x.min(other.x);
        let min_y = self.y.min(other.y);
        let max_x = (self.x + self.width as i32).max(other.x + other.width as i32);
        let max_y = (self.y + self.height as i32).max(other.y + other.height as i32);

        DamageRegion {
            x: min_x,
            y: min_y,
            width: (max_x - min_x) as u32,
            height: (max_y - min_y) as u32,
        }
    }

    /// Clamps damage region strictly to framebuffer boundaries.
    pub fn clamp_to_screen(&self, max_w: u32, max_h: u32) -> DamageRegion {
        if self.is_empty() || self.x >= max_w as i32 || self.y >= max_h as i32 {
            return Self::empty();
        }

        let clamped_x = self.x.max(0);
        let clamped_y = self.y.max(0);

        let end_x = (self.x + self.width as i32).clamp(0, max_w as i32);
        let end_y = (self.y + self.height as i32).clamp(0, max_h as i32);

        if end_x <= clamped_x || end_y <= clamped_y {
            return Self::empty();
        }

        DamageRegion {
            x: clamped_x,
            y: clamped_y,
            width: (end_x - clamped_x) as u32,
            height: (end_y - clamped_y) as u32,
        }
    }

    /// Tests if a point is contained within this damage region.
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.width as i32 && py >= self.y && py < self.y + self.height as i32
    }

    /// Returns area in pixels.
    pub fn area(&self) -> u64 {
        (self.width as u64) * (self.height as u64)
    }
}

/// Accumulates dirty rectangles during frame lifecycle (Multi-rect Dirty Tracking).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageTracker {
    pub rects: [DamageRegion; 4],
    pub count: usize,
    pub current_damage: Option<DamageRegion>,
    pub force_full_redraw: bool,
}

impl DamageTracker {
    pub const fn new() -> Self {
        Self {
            rects: [DamageRegion::empty(); 4],
            count: 0,
            current_damage: None,
            force_full_redraw: true, // Initial boot frame requires full paint
        }
    }

    /// Adds a damage rectangle to the accumulated dirty regions with smart multi-rect isolation.
    pub fn add_rect(&mut self, rect: DamageRegion) {
        if rect.is_empty() { return; }

        // Genel bounding box'ı daima güncelle (geriye uyumluluk ve tekil kırpma için)
        self.current_damage = match self.current_damage {
            Some(curr) => Some(curr.union(&rect)),
            None => Some(rect),
        };

        // Merge only when the bounding rectangle costs no more than two draws.
        // Restart after merging so overlapping chains do not leave duplicate work.
        let mut pending = rect;
        let mut i = 0;
        while i < self.count {
            let merged = self.rects[i].union(&pending);
            if merged.area() <= self.rects[i].area() + pending.area() {
                pending = merged;
                self.count -= 1;
                self.rects[i] = self.rects[self.count];
                i = 0;
            } else { i += 1; }
        }
        if self.count < self.rects.len() {
            self.rects[self.count] = pending;
            self.count += 1;
        } else {
            // Bounded storage: choose the merge introducing the least extra area.
            let best = (0..self.count).min_by_key(|&i|
                self.rects[i].union(&pending).area().saturating_sub(self.rects[i].area())).unwrap();
            self.rects[best] = self.rects[best].union(&pending);
        }
    }

    /// Drain bounded regions without filling the unchanged space between them.
    pub fn take_regions(&mut self, screen_w: u32, screen_h: u32) -> ([DamageRegion; 4], usize) {
        let mut result = [DamageRegion::empty(); 4];
        let mut count = 0;
        if self.force_full_redraw {
            result[0] = DamageRegion::new(0, 0, screen_w, screen_h);
            count = 1;
        } else {
            for rect in &self.rects[..self.count] {
                let clipped = rect.clamp_to_screen(screen_w, screen_h);
                if !clipped.is_empty() { result[count] = clipped; count += 1; }
            }
        }
        self.reset();
        (result, count)
    }

    /// Adds damage by coordinates and dimensions.
    pub fn add_bounds(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.add_rect(DamageRegion::new(x, y, width, height));
    }

    /// Forces a full screen redraw on next frame.
    pub fn add_full_screen(&mut self, screen_w: u32, screen_h: u32) {
        self.force_full_redraw = true;
        self.add_bounds(0, 0, screen_w, screen_h);
    }

    /// Takes accumulated damage for consumption by compositor, resetting tracker.
    pub fn take_damage(&mut self) -> Option<DamageRegion> {
        let is_full = self.force_full_redraw;
        self.force_full_redraw = false;
        self.count = 0;
        let dmg = self.current_damage.take();
        if is_full {
            #[cfg(not(test))]
            {
                let sw = unsafe { crate::gui::VESA.width as u32 };
                let sh = unsafe { crate::gui::VESA.height as u32 };
                Some(DamageRegion::new(0, 0, sw, sh))
            }
            #[cfg(test)]
            {
                Some(DamageRegion::new(0, 0, 1280, 720))
            }
        } else {
            dmg
        }
    }

    /// Peeks current damage without consuming.
    pub fn peek_damage(&self) -> Option<DamageRegion> {
        if self.force_full_redraw {
            #[cfg(not(test))]
            {
                let sw = unsafe { crate::gui::VESA.width as u32 };
                let sh = unsafe { crate::gui::VESA.height as u32 };
                Some(DamageRegion::new(0, 0, sw, sh))
            }
            #[cfg(test)]
            {
                Some(DamageRegion::new(0, 0, 1280, 720))
            }
        } else {
            self.current_damage
        }
    }

    pub fn is_damaged(&self) -> bool {
        self.force_full_redraw || self.current_damage.map(|d| !d.is_empty()).unwrap_or(false)
    }

    pub fn reset(&mut self) {
        self.current_damage = None;
        self.count = 0;
        self.force_full_redraw = false;
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;

    #[test]
    fn separated_damage_does_not_fill_gap() {
        let mut tracker = DamageTracker::new();
        tracker.reset();
        tracker.add_bounds(101, 180, 16, 16);
        tracker.add_bounds(1001, 650, 16, 16);
        let (regions, count) = tracker.take_regions(1280, 720);
        assert_eq!(count, 2);
        assert_eq!(regions[..count].iter().map(|r| r.area()).sum::<u64>(), 512);
        assert!(!tracker.is_damaged());
    }

    #[test]
    fn bounded_regions_cover_every_input_even_after_overflow() {
        let mut tracker = DamageTracker::new();
        tracker.reset();
        for n in 0..12 { tracker.add_bounds(n * 80, n * 40, 10, 10); }
        let (regions, count) = tracker.take_regions(1280, 720);
        assert!(count <= 4);
        for n in 0..12 {
            for (dx, dy) in [(0, 0), (9, 9)] {
                assert!(regions[..count].iter().any(|r| r.contains_point(n * 80 + dx, n * 40 + dy)));
            }
        }
    }

    #[test]
    fn full_damage_and_offscreen_regions_are_consumed() {
        let mut tracker = DamageTracker::new();
        assert_eq!(tracker.take_regions(800, 600).0[0], DamageRegion::new(0, 0, 800, 600));
        tracker.add_bounds(-10, -10, 20, 20);
        tracker.add_bounds(2000, 2000, 10, 10);
        let (regions, count) = tracker.take_regions(800, 600);
        assert_eq!(count, 1);
        assert_eq!(regions[0], DamageRegion::new(0, 0, 10, 10));
        assert_eq!(tracker.take_regions(800, 600).1, 0);
    }
}
