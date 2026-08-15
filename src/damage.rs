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

    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
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

    /// Tests if a point is contained within this damage region.
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.width as i32 && py >= self.y && py < self.y + self.height as i32
    }
}
