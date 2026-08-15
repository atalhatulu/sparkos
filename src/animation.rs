//! SparkOS Desktop V1.18 — Window & Transition Animation Engine
//!
//! Provides frame-based window transition animations (Opening, Closing, Minimizing, Maximizing)
//! and alpha fade blending strictly within the Window Manager / Compositor layer without
//! microkernel pollution.

use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationState {
    None,
    Opening,
    Closing,
    Minimizing,
    Maximizing,
}

#[derive(Debug, Clone, Copy)]
pub struct WindowAnimation {
    pub window_id: u64,
    pub state: AnimationState,
    pub current_frame: u8,
    pub total_frames: u8,
    pub start_x: i32,
    pub start_y: i32,
    pub start_w: u32,
    pub start_h: u32,
    pub target_x: i32,
    pub target_y: i32,
    pub target_w: u32,
    pub target_h: u32,
}

pub struct AnimationEngine {
    pub animations: [Option<WindowAnimation>; 16],
}

impl AnimationEngine {
    pub const fn new() -> Self {
        Self {
            animations: [None; 16],
        }
    }

    pub fn start_animation(&mut self, window_id: u64, state: AnimationState, sx: i32, sy: i32, sw: u32, sh: u32, tx: i32, ty: i32, tw: u32, th: u32) {
        let anim = WindowAnimation {
            window_id,
            state,
            current_frame: 0,
            total_frames: 6, // 6 frames smooth transition
            start_x: sx,
            start_y: sy,
            start_w: sw,
            start_h: sh,
            target_x: tx,
            target_y: ty,
            target_w: tw,
            target_h: th,
        };

        for slot in &mut self.animations {
            if slot.is_none() || slot.as_ref().map(|a| a.window_id == window_id).unwrap_or(false) {
                *slot = Some(anim);
                return;
            }
        }
    }

    pub fn step_frames(&mut self) {
        for slot in &mut self.animations {
            if let Some(anim) = slot {
                anim.current_frame += 1;
                if anim.current_frame >= anim.total_frames {
                    *slot = None; // Animation finished
                }
            }
        }
    }

    /// Computes the interpolated geometry (x, y, w, h, alpha) for a window
    pub fn get_interpolated_geometry(&self, window_id: u64, default_x: i32, default_y: i32, default_w: u32, default_h: u32) -> (i32, i32, u32, u32, u8) {
        for slot in &self.animations {
            if let Some(anim) = slot {
                if anim.window_id == window_id {
                    let t = anim.current_frame as i32;
                    let total = anim.total_frames as i32;

                    let cur_x = anim.start_x + ((anim.target_x - anim.start_x) * t) / total;
                    let cur_y = anim.start_y + ((anim.target_y - anim.start_y) * t) / total;
                    let cur_w = (anim.start_w as i32 + ((anim.target_w as i32 - anim.start_w as i32) * t) / total).max(1) as u32;
                    let cur_h = (anim.start_h as i32 + ((anim.target_h as i32 - anim.start_h as i32) * t) / total).max(1) as u32;
                    let alpha = ((255 * anim.current_frame as u32) / anim.total_frames as u32) as u8;

                    return (cur_x, cur_y, cur_w, cur_h, alpha);
                }
            }
        }
        (default_x, default_y, default_w, default_h, 255)
    }

    pub fn is_animating(&self, window_id: u64) -> bool {
        self.animations.iter().any(|a| a.as_ref().map(|anim| anim.window_id == window_id).unwrap_or(false))
    }
}

pub static ANIMATION_ENGINE: Mutex<AnimationEngine> = Mutex::new(AnimationEngine::new());
