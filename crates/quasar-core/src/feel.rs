//! Game-feel helpers: hit-stop (freeze frames), time scaling and trauma
//! camera shake.
//!
//! These are small, engine-generic state machines with no renderer or
//! window dependency. Feed them the frame's real `dt`; they return the
//! simulation `dt` (for [`HitStop`]) or a camera offset (for [`Trauma`]).
//!
//! ```
//! use quasar_core::feel::{HitStop, Trauma, TraumaSettings};
//!
//! let mut stop = HitStop::new(0.3);
//! stop.trigger(0.08);                  // a big hit freezes the game briefly
//! assert_eq!(stop.scale_dt(1.0 / 60.0), 0.0);
//!
//! let mut shake = Trauma::new(TraumaSettings::default(), 7);
//! shake.add(0.6);
//! shake.update(1.0 / 60.0);
//! let offset = shake.offset();         // add to the camera target / eye
//! assert!(offset.iter().any(|v| *v != 0.0));
//! ```
use crate::noise::Noise;

/// Freeze-frame ("hit-stop") with an optional slow-motion tail and a
/// cooldown so rapid hits do not stall the game.
#[derive(Clone, Debug, PartialEq)]
pub struct HitStop {
    remaining: f32,
    slow_remaining: f32,
    slow_scale: f32,
    cooldown: f32,
    cooldown_left: f32,
    enabled: bool,
}

impl HitStop {
    /// `cooldown` seconds must pass after a stop starts before another can.
    pub fn new(cooldown: f32) -> Self {
        Self {
            remaining: 0.0,
            slow_remaining: 0.0,
            slow_scale: 1.0,
            cooldown: cooldown.max(0.0),
            cooldown_left: 0.0,
            enabled: true,
        }
    }
    /// Turns stops off (for example from an accessibility option); pending
    /// stops end at once.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.clear();
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    /// Freezes for `seconds` (extending, never shortening, an active stop).
    /// Returns `false` when disabled or cooling down.
    pub fn trigger(&mut self, seconds: f32) -> bool {
        if !self.enabled || self.cooldown_left > 0.0 || seconds.is_nan() || seconds <= 0.0 {
            return false;
        }
        self.remaining = self.remaining.max(seconds);
        self.cooldown_left = self.cooldown;
        true
    }
    /// Plays the game at `scale` (0..1) for `seconds` after any freeze ends.
    pub fn slow_motion(&mut self, seconds: f32, scale: f32) {
        if self.enabled && seconds > 0.0 {
            self.slow_remaining = self.slow_remaining.max(seconds);
            self.slow_scale = scale.clamp(0.0, 1.0);
        }
    }
    /// True while frozen: skip simulation ticks (keep rendering and audio).
    pub fn frozen(&self) -> bool {
        self.remaining > 0.0
    }
    /// Current time scale: 0 while frozen, the slow-motion scale during the
    /// tail, otherwise 1.
    pub fn time_scale(&self) -> f32 {
        if self.remaining > 0.0 {
            0.0
        } else if self.slow_remaining > 0.0 {
            self.slow_scale
        } else {
            1.0
        }
    }
    /// Advances the timers by real `dt` and returns the scaled simulation
    /// time for this frame. Time spent inside a freeze is not simulated.
    pub fn scale_dt(&mut self, dt: f32) -> f32 {
        let dt = dt.max(0.0);
        self.cooldown_left = (self.cooldown_left - dt).max(0.0);
        let mut left = dt;
        let frozen = left.min(self.remaining);
        self.remaining -= frozen;
        left -= frozen;
        let slow = left.min(self.slow_remaining);
        self.slow_remaining -= slow;
        left -= slow;
        slow * self.slow_scale + left
    }
    /// Ends any freeze and slow motion (for example on a scene change).
    pub fn clear(&mut self) {
        self.remaining = 0.0;
        self.slow_remaining = 0.0;
        self.cooldown_left = 0.0;
    }
}

impl Default for HitStop {
    fn default() -> Self {
        Self::new(0.3)
    }
}

/// Settings for [`Trauma`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraumaSettings {
    /// Trauma lost per second.
    pub decay: f32,
    /// Upper bound on accumulated trauma.
    pub max_trauma: f32,
    /// Shake = trauma^exponent; 2 keeps small bumps subtle while big hits
    /// clearly jolt the view.
    pub exponent: f32,
    /// World-space offset at shake 1, per axis.
    pub amplitude: [f32; 3],
    /// Angular shake at 1 (radians: yaw, pitch, roll).
    pub angles: [f32; 3],
    /// Noise frequency in Hz.
    pub frequency: f32,
}

impl Default for TraumaSettings {
    fn default() -> Self {
        Self {
            decay: 1.2,
            max_trauma: 1.0,
            exponent: 2.0,
            amplitude: [0.4, 0.25, 0.4],
            angles: [0.02, 0.02, 0.03],
            frequency: 18.0,
        }
    }
}

/// Trauma camera shake (Eiserloh's model): events add trauma, trauma decays
/// linearly, and the shake is smooth noise scaled by `trauma^exponent`.
#[derive(Clone, Debug, PartialEq)]
pub struct Trauma {
    settings: TraumaSettings,
    trauma: f32,
    time: f32,
    noise: Noise,
    scale: f32,
}

impl Trauma {
    pub fn new(settings: TraumaSettings, seed: u64) -> Self {
        Self {
            settings,
            trauma: 0.0,
            time: 0.0,
            noise: Noise::new(seed),
            scale: 1.0,
        }
    }
    pub fn settings(&self) -> TraumaSettings {
        self.settings
    }
    pub fn set_settings(&mut self, settings: TraumaSettings) {
        self.settings = settings;
        self.trauma = self.trauma.min(settings.max_trauma);
    }
    /// Multiplies the final shake (0 disables it, for example from options).
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.max(0.0);
    }
    /// Adds trauma (clamped to `max_trauma`).
    pub fn add(&mut self, amount: f32) {
        if amount > 0.0 {
            self.trauma = (self.trauma + amount).min(self.settings.max_trauma);
        }
    }
    /// Raises trauma to at least `amount`.
    pub fn at_least(&mut self, amount: f32) {
        self.trauma = self.trauma.max(amount.min(self.settings.max_trauma));
    }
    pub fn trauma(&self) -> f32 {
        self.trauma
    }
    /// Current shake strength, `trauma^exponent * scale`.
    pub fn shake(&self) -> f32 {
        self.trauma.max(0.0).powf(self.settings.exponent) * self.scale
    }
    pub fn update(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        self.time += dt;
        // Keep the noise coordinate small so f32 precision never coarsens
        // the motion during long sessions.
        if self.time > 4096.0 {
            self.time -= 4096.0;
        }
        self.trauma = (self.trauma - self.settings.decay * dt).max(0.0);
    }
    fn channel(&self, index: u32) -> f32 {
        let t = self.time * self.settings.frequency;
        self.noise.derive(index).perlin2(t, index as f32 * 7.31)
    }
    /// World-space offset for the camera target and eye.
    pub fn offset(&self) -> [f32; 3] {
        let s = self.shake();
        if s == 0.0 {
            return [0.0; 3];
        }
        let a = self.settings.amplitude;
        [
            self.channel(0) * a[0] * s,
            self.channel(1) * a[1] * s,
            self.channel(2) * a[2] * s,
        ]
    }
    /// Rotation offsets in radians (yaw, pitch, roll).
    pub fn angles(&self) -> [f32; 3] {
        let s = self.shake();
        if s == 0.0 {
            return [0.0; 3];
        }
        let a = self.settings.angles;
        [
            self.channel(3) * a[0] * s,
            self.channel(4) * a[1] * s,
            self.channel(5) * a[2] * s,
        ]
    }
    pub fn clear(&mut self) {
        self.trauma = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_stop_freezes_then_resumes_and_cools_down() {
        let mut stop = HitStop::new(0.3);
        assert!(stop.trigger(0.05));
        assert!(stop.frozen());
        assert!(!stop.trigger(0.2), "cooldown blocks a second stop");
        assert_eq!(stop.scale_dt(0.02), 0.0);
        // 0.03 s of freeze left; the rest of a 0.05 s frame is simulated.
        let sim = stop.scale_dt(0.05);
        assert!((sim - 0.02).abs() < 1e-6, "{sim}");
        assert!(!stop.frozen());
        assert_eq!(stop.scale_dt(0.1), 0.1);
        stop.scale_dt(0.3);
        assert!(stop.trigger(0.01));
        stop.set_enabled(false);
        assert!(!stop.frozen());
        assert!(!stop.trigger(1.0));
    }

    #[test]
    fn slow_motion_follows_the_freeze() {
        let mut stop = HitStop::new(0.0);
        stop.trigger(0.1);
        stop.slow_motion(0.2, 0.25);
        assert_eq!(stop.time_scale(), 0.0);
        let sim = stop.scale_dt(0.2); // 0.1 frozen + 0.1 at quarter speed
        assert!((sim - 0.025).abs() < 1e-6, "{sim}");
        assert_eq!(stop.time_scale(), 0.25);
        let sim = stop.scale_dt(0.2); // 0.1 slow + 0.1 normal
        assert!((sim - 0.125).abs() < 1e-6, "{sim}");
        assert_eq!(stop.time_scale(), 1.0);
    }

    #[test]
    fn trauma_decays_and_shake_grows_faster_than_trauma() {
        let mut shake = Trauma::new(TraumaSettings::default(), 3);
        assert_eq!(shake.offset(), [0.0; 3]);
        shake.add(0.5);
        let half = shake.shake();
        shake.add(0.5);
        assert!(shake.shake() > half * 3.0);
        shake.add(5.0);
        assert_eq!(shake.trauma(), 1.0);
        let mut moved = false;
        let mut last = shake.offset();
        for _ in 0..30 {
            shake.update(1.0 / 60.0);
            let now = shake.offset();
            moved |= now != last;
            for (n, l) in now.iter().zip(last) {
                assert!((n - l).abs() < 0.2, "smooth motion");
            }
            last = now;
        }
        assert!(moved);
        shake.update(10.0);
        assert_eq!(shake.trauma(), 0.0);
        assert_eq!(shake.offset(), [0.0; 3]);
        assert_eq!(shake.angles(), [0.0; 3]);
    }

    #[test]
    fn trauma_scale_disables_shake() {
        let mut shake = Trauma::new(TraumaSettings::default(), 3);
        shake.add(1.0);
        shake.update(0.01);
        shake.set_scale(0.0);
        assert_eq!(shake.offset(), [0.0; 3]);
    }
}
