use std::time::Instant;

pub mod state;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepeatInfo {
    /// The rate of repeating keys in characters per second
    pub rate: i32,
    /// Delay in milliseconds since key down until repeating starts
    pub delay: i32,
}

pub const DEFAULT_REPEAT_INFO: RepeatInfo = RepeatInfo {
    rate: 20,
    delay: 200,
};

impl RepeatInfo {
    pub fn from_protocol(rate: i32, delay: i32, fallback_on_zero: bool) -> Option<Self> {
        let delay = if delay >= 0 {
            delay
        } else {
            DEFAULT_REPEAT_INFO.delay
        };

        match rate {
            1.. => Some(Self { rate, delay }),
            0 if fallback_on_zero => Some(Self {
                rate: DEFAULT_REPEAT_INFO.rate,
                delay,
            }),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum PressState {
    /// User is pressing no key, or user lifted last pressed key. But kime-wayland is ready for key
    /// long-press.
    NotPressing,
    /// User is pressing a key.
    Pressing {
        /// User started pressing a key at this moment.
        pressed_at: Instant,
        /// `false` if user just started pressing a key. Soon, key repeating will be begin. `true`
        /// if user have pressed a key for a long enough time, key repeating is happening right
        /// now.
        is_repeating: bool,

        /// Key code used by wayland
        key: u32,
        /// Timestamp with millisecond granularity used by wayland. Their base is undefined, so
        /// they can't be compared against system time (as obtained with clock_gettime or
        /// gettimeofday). They can be compared with each other though, and for instance be used to
        /// identify sequences of button presses as double or triple clicks.
        ///
        /// #### Reference
        /// - https://wayland.freedesktop.org/docs/html/ch04.html#sect-Protocol-Input
        wayland_time: u32,
    },
}

impl PressState {
    pub fn is_pressing(&self, query_key: u32) -> bool {
        if let PressState::Pressing { key, .. } = self {
            *key == query_key
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RepeatInfo, DEFAULT_REPEAT_INFO};

    #[test]
    fn protocol_repeat_info_uses_compositor_values() {
        assert_eq!(
            RepeatInfo::from_protocol(30, 250, false),
            Some(RepeatInfo {
                rate: 30,
                delay: 250
            })
        );
    }

    #[test]
    fn zero_rate_disables_repeat_without_fallback() {
        assert_eq!(RepeatInfo::from_protocol(0, 600, false), None);
    }

    #[test]
    fn zero_rate_uses_fallback_and_preserves_delay_when_requested() {
        assert_eq!(
            RepeatInfo::from_protocol(0, 600, true),
            Some(RepeatInfo {
                rate: DEFAULT_REPEAT_INFO.rate,
                delay: 600
            })
        );
    }

    #[test]
    fn invalid_values_cannot_create_a_negative_duration_or_rate() {
        assert_eq!(RepeatInfo::from_protocol(-1, -1, true), None);
        assert_eq!(
            RepeatInfo::from_protocol(20, -1, false),
            Some(DEFAULT_REPEAT_INFO)
        );
    }
}
