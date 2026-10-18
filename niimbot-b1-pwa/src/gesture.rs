use std::ops::RangeInclusive;

const RIGHT_ANGLE_SNAP: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pinch {
    scale: f64,
    turn: f64,
    pub(crate) shift: (f64, f64),
}

impl Pinch {
    pub(crate) fn between([a, b]: [(f64, f64); 2], [c, d]: [(f64, f64); 2]) -> Self {
        let (before, after) = ((b.0 - a.0, b.1 - a.1), (d.0 - c.0, d.1 - c.1));
        Self {
            scale: after.0.hypot(after.1) / before.0.hypot(before.1).max(1.0),
            turn: (after.1.atan2(after.0) - before.1.atan2(before.0)).to_degrees(),
            shift: ((c.0 + d.0 - a.0 - b.0) / 2.0, (c.1 + d.1 - a.1 - b.1) / 2.0),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the size is clamped into a u16 range first"
    )]
    pub(crate) fn resize(&self, size: u16, range: RangeInclusive<u16>) -> u16 {
        (f64::from(size) * self.scale)
            .round()
            .clamp(f64::from(*range.start()), f64::from(*range.end())) as u16
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the angle is wrapped into -180..=180 degrees first"
    )]
    pub(crate) fn rotate(&self, rotation: i16) -> i16 {
        let degrees = (f64::from(rotation) + self.turn + 180.0).rem_euclid(360.0) - 180.0;
        let right_angle = (degrees / 90.0).round() * 90.0;
        if (degrees - right_angle).abs() <= RIGHT_ANGLE_SNAP {
            right_angle as i16
        } else {
            degrees.round() as i16
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreading_the_fingers_scales_around_their_midpoint() {
        let pinch = Pinch::between([(0.0, 0.0), (80.0, 0.0)], [(-20.0, 0.0), (100.0, 0.0)]);
        assert_eq!(pinch.shift, (0.0, 0.0));
        assert_eq!(pinch.rotate(0), 0);
        assert_eq!(pinch.resize(100, 10..=400), 150);
        assert_eq!(pinch.resize(300, 10..=400), 400);
    }

    #[test]
    fn twisting_sticks_to_right_angles() {
        let nearly_square =
            Pinch::between([(0.0, 0.0), (80.0, 0.0)], [(38.0, -40.0), (42.0, 40.0)]);
        assert_eq!(nearly_square.rotate(0), 90);
        assert_eq!(nearly_square.resize(100, 10..=400), 100);
        let sixty = Pinch::between(
            [(0.0, 0.0), (2.0, 0.0)],
            [(0.0, 0.0), (1.0, 3.0_f64.sqrt())],
        );
        assert_eq!(sixty.rotate(0), 60);
        assert_eq!(sixty.rotate(150), -150);
    }

    #[test]
    fn moving_both_fingers_shifts_by_the_midpoint() {
        let pinch = Pinch::between([(0.0, 0.0), (80.0, 0.0)], [(10.0, 20.0), (90.0, 20.0)]);
        assert_eq!(pinch.shift, (10.0, 20.0));
        assert_eq!(pinch.resize(100, 10..=400), 100);
    }

    #[test]
    fn fingers_that_start_together_do_not_blow_up() {
        let pinch = Pinch::between([(5.0, 5.0), (5.0, 5.0)], [(0.0, 5.0), (10.0, 5.0)]);
        assert_eq!(pinch.resize(100, 10..=400), 400);
        assert!(pinch.rotate(0).abs() <= 180);
    }
}
