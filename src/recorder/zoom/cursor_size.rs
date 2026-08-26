use serde::{Deserialize, Serialize};

use super::super::cursor_settings::{MAX_CURSOR_SCALE, MIN_CURSOR_SCALE};

pub(crate) const DEFAULT_CURSOR_SIZE_REGION_DURATION_US: u64 = 1_000_000;
pub(crate) const MIN_CURSOR_SIZE_REGION_DURATION_US: u64 = 50_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CursorSizeEasing {
    #[default]
    EaseInOut,
    #[serde(other)]
    Invalid,
}

impl CursorSizeEasing {
    fn normalized(self) -> Self {
        match self {
            Self::EaseInOut | Self::Invalid => Self::EaseInOut,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self.normalized() {
            Self::EaseInOut | Self::Invalid => "Ease in/out",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct CursorSizeRegion {
    #[serde(default)]
    pub(crate) start_us: u64,
    #[serde(default)]
    pub(crate) end_us: u64,
    #[serde(default = "default_cursor_start_scale")]
    pub(crate) start_scale: f32,
    #[serde(default = "default_cursor_end_scale")]
    pub(crate) end_scale: f32,
    #[serde(default)]
    pub(crate) easing: CursorSizeEasing,
    #[serde(default)]
    pub(crate) ease_in_end_us: Option<u64>,
    #[serde(default)]
    pub(crate) ease_out_start_us: Option<u64>,
}

impl CursorSizeRegion {
    pub(crate) fn new_at(playhead_us: u64, duration_us: u64) -> Option<Self> {
        if duration_us == 0 {
            return None;
        }

        let start_us = playhead_us.min(duration_us);
        let end_us = start_us
            .saturating_add(DEFAULT_CURSOR_SIZE_REGION_DURATION_US)
            .min(duration_us);
        let (start_us, end_us) = if end_us > start_us {
            (start_us, end_us)
        } else {
            (
                duration_us.saturating_sub(DEFAULT_CURSOR_SIZE_REGION_DURATION_US),
                duration_us,
            )
        };

        (end_us > start_us).then_some(Self {
            start_us,
            end_us,
            start_scale: 1.0,
            end_scale: 1.5,
            easing: CursorSizeEasing::EaseInOut,
            ease_in_end_us: None,
            ease_out_start_us: None,
        })
    }

    pub(crate) fn normalized(mut self) -> Option<Self> {
        if self.end_us <= self.start_us {
            return None;
        }
        self.start_scale = if self.start_scale.is_finite() {
            self.start_scale.clamp(MIN_CURSOR_SCALE, MAX_CURSOR_SCALE)
        } else {
            1.0
        };
        self.end_scale = if self.end_scale.is_finite() {
            self.end_scale.clamp(MIN_CURSOR_SCALE, MAX_CURSOR_SCALE)
        } else {
            1.5
        };
        self.easing = self.easing.normalized();
        let (ease_in_end_us, ease_out_start_us) = self.transition_points();
        self.ease_in_end_us = Some(ease_in_end_us);
        self.ease_out_start_us = Some(ease_out_start_us);
        Some(self)
    }

    pub(crate) fn normalized_for_duration(self, duration_us: u64) -> Option<Self> {
        if duration_us == 0 {
            return None;
        }
        let mut region = self;
        region.start_us = region.start_us.min(duration_us);
        region.end_us = region.end_us.min(duration_us);
        let transitions_fit = region
            .ease_in_end_us
            .is_some_and(|time| time >= region.start_us && time <= region.end_us)
            && region
                .ease_out_start_us
                .is_some_and(|time| time >= region.start_us && time <= region.end_us)
            && region.ease_in_end_us <= region.ease_out_start_us;
        if !transitions_fit {
            region.ease_in_end_us = None;
            region.ease_out_start_us = None;
        }
        region.normalized()
    }

    pub(crate) fn duration_us(self) -> u64 {
        self.end_us.saturating_sub(self.start_us)
    }

    pub(crate) fn transition_points(self) -> (u64, u64) {
        let duration = self.duration_us();
        let default_ramp = duration / super::DEFAULT_TRANSITION_RATIO;
        let default_ease_in_end = self.start_us.saturating_add(default_ramp);
        let default_ease_out_start = self.end_us.saturating_sub(default_ramp);
        let ease_in_end = self
            .ease_in_end_us
            .unwrap_or(default_ease_in_end)
            .clamp(self.start_us, self.end_us);
        let ease_out_start = self
            .ease_out_start_us
            .unwrap_or(default_ease_out_start)
            .clamp(self.start_us, self.end_us);

        if ease_in_end <= ease_out_start {
            (ease_in_end, ease_out_start)
        } else {
            let midpoint = self.start_us.saturating_add(duration / 2);
            (midpoint, midpoint)
        }
    }

    pub(crate) fn scale_at(self, playhead_us: u64) -> Option<f32> {
        if playhead_us < self.start_us || playhead_us > self.end_us {
            return None;
        }

        if self.duration_us() == 0 {
            return None;
        }
        let (ease_in_end_us, ease_out_start_us) = self.transition_points();
        let envelope = match self.easing.normalized() {
            CursorSizeEasing::EaseInOut | CursorSizeEasing::Invalid => {
                if playhead_us <= self.start_us || playhead_us >= self.end_us {
                    0.0
                } else if playhead_us <= ease_in_end_us {
                    super::transition_progress(
                        playhead_us.saturating_sub(self.start_us),
                        ease_in_end_us.saturating_sub(self.start_us),
                    )
                } else if playhead_us < ease_out_start_us {
                    1.0
                } else {
                    super::transition_progress(
                        self.end_us.saturating_sub(playhead_us),
                        self.end_us.saturating_sub(ease_out_start_us),
                    )
                }
            }
        };
        Some(self.start_scale + (self.end_scale - self.start_scale) * envelope)
    }
}

pub(crate) fn cursor_scale_at(
    regions: &[CursorSizeRegion],
    playhead_us: u64,
    base_scale: f32,
) -> f32 {
    regions
        .iter()
        .rev()
        .find_map(|region| region.scale_at(playhead_us))
        .unwrap_or(base_scale)
}

fn default_cursor_start_scale() -> f32 {
    1.0
}

fn default_cursor_end_scale() -> f32 {
    1.5
}

#[cfg(test)]
#[path = "cursor_size_tests.rs"]
mod tests;
