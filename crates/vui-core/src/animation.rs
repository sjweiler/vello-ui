use crate::{Color, Point, Rect, Size};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationFrame {
    pub elapsed: Duration,
    pub delta: Duration,
    pub reduced_motion: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
}

impl Easing {
    #[must_use]
    pub fn sample(self, progress: f64) -> f64 {
        let progress = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => progress,
            Self::EaseIn => progress * progress,
            Self::EaseOut => 1.0 - (1.0 - progress) * (1.0 - progress),
            Self::EaseInOut if progress < 0.5 => 2.0 * progress * progress,
            Self::EaseInOut => 1.0 - (-2.0 * progress + 2.0).powi(2) * 0.5,
        }
    }
}

pub trait Interpolate: Clone {
    #[must_use]
    fn interpolate(&self, other: &Self, progress: f64) -> Self;
}

impl Interpolate for f64 {
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        self + (other - self) * progress
    }
}

impl Interpolate for f32 {
    #[allow(clippy::cast_possible_truncation)]
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        let progress = progress as f32;
        self + (other - self) * progress
    }
}

impl Interpolate for Point {
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        Self::new(
            self.x.interpolate(&other.x, progress),
            self.y.interpolate(&other.y, progress),
        )
    }
}

impl Interpolate for Size {
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        Self::new(
            self.width.interpolate(&other.width, progress),
            self.height.interpolate(&other.height, progress),
        )
    }
}

impl Interpolate for Rect {
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        Self::new(
            self.x.interpolate(&other.x, progress),
            self.y.interpolate(&other.y, progress),
            self.width.interpolate(&other.width, progress),
            self.height.interpolate(&other.height, progress),
        )
    }
}

impl Interpolate for Color {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        fn channel(from: u8, to: u8, progress: f64) -> u8 {
            let value = f64::from(from).interpolate(&f64::from(to), progress);
            value.round().clamp(0.0, 255.0) as u8
        }
        Self::rgba(
            channel(self.r, other.r, progress),
            channel(self.g, other.g, progress),
            channel(self.b, other.b, progress),
            channel(self.a, other.a, progress),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tween<T> {
    from: T,
    to: T,
    value: T,
    duration: Duration,
    delay: Duration,
    elapsed: Duration,
    easing: Easing,
}

impl<T: Interpolate> Tween<T> {
    #[must_use]
    pub fn new(from: T, to: T, duration: Duration) -> Self {
        Self {
            value: from.clone(),
            from,
            to,
            duration,
            delay: Duration::ZERO,
            elapsed: Duration::ZERO,
            easing: Easing::default(),
        }
    }

    #[must_use]
    pub const fn easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    #[must_use]
    pub const fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn retarget(&mut self, target: T, duration: Duration) {
        self.from.clone_from(&self.value);
        self.to = target;
        self.duration = duration;
        self.delay = Duration::ZERO;
        self.elapsed = Duration::ZERO;
    }

    pub fn advance(&mut self, delta: Duration, reduced_motion: bool) -> bool {
        if reduced_motion {
            self.value.clone_from(&self.to);
            self.elapsed = self.delay.saturating_add(self.duration);
            return false;
        }
        self.elapsed = self.elapsed.saturating_add(delta);
        let active = self.elapsed.saturating_sub(self.delay);
        let progress = if self.duration.is_zero() {
            1.0
        } else {
            active.as_secs_f64() / self.duration.as_secs_f64()
        };
        self.value = self
            .from
            .interpolate(&self.to, self.easing.sample(progress));
        !self.is_finished()
    }

    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.elapsed >= self.delay.saturating_add(self.duration)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    value: f64,
    target: f64,
    velocity: f64,
    response: f64,
    damping: f64,
}

impl Spring {
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self {
            value,
            target: value,
            velocity: 0.0,
            response: 18.0,
            damping: 1.0,
        }
    }

    #[must_use]
    pub fn response(mut self, response: f64) -> Self {
        self.response = response.max(0.01);
        self
    }

    #[must_use]
    pub fn damping(mut self, damping: f64) -> Self {
        self.damping = damping.max(0.0);
        self
    }

    pub const fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    pub fn advance(&mut self, delta: Duration, reduced_motion: bool) -> bool {
        if reduced_motion {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        let seconds = delta.as_secs_f64().min(0.05);
        let acceleration = (self.target - self.value) * self.response * self.response
            - 2.0 * self.damping * self.response * self.velocity;
        self.velocity += acceleration * seconds;
        self.value += self.velocity * seconds;
        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }

    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }

    #[must_use]
    pub fn is_settled(self) -> bool {
        (self.target - self.value).abs() < 0.001 && self.velocity.abs() < 0.001
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_preserves_endpoints() {
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
        ] {
            assert!(easing.sample(0.0).abs() < f64::EPSILON);
            assert!((easing.sample(1.0) - 1.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn tween_honors_delay_and_can_reverse_from_its_current_value() {
        let mut tween = Tween::new(0.0_f64, 10.0, Duration::from_millis(100))
            .delay(Duration::from_millis(50))
            .easing(Easing::Linear);
        assert!(tween.advance(Duration::from_millis(40), false));
        assert!(tween.value().abs() < f64::EPSILON);
        assert!(tween.advance(Duration::from_millis(60), false));
        assert!((*tween.value() - 5.0).abs() < f64::EPSILON);
        tween.retarget(0.0, Duration::from_millis(100));
        assert!(tween.advance(Duration::from_millis(50), false));
        assert!((*tween.value() - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn reduced_motion_settles_tweens_and_springs_immediately() {
        let mut tween = Tween::new(0.0_f64, 1.0, Duration::from_secs(1));
        assert!(!tween.advance(Duration::ZERO, true));
        assert!((*tween.value() - 1.0).abs() < f64::EPSILON);

        let mut spring = Spring::new(0.0);
        spring.set_target(1.0);
        assert!(!spring.advance(Duration::ZERO, true));
        assert!((spring.value() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn spring_converges_on_its_target() {
        let mut spring = Spring::new(0.0).response(16.0).damping(1.0);
        spring.set_target(1.0);
        for _ in 0..240 {
            spring.advance(Duration::from_millis(8), false);
        }
        assert!(spring.is_settled());
        assert!((spring.value() - 1.0).abs() < f64::EPSILON);
    }
}
