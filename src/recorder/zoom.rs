use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_ZOOM_REGION_DURATION_US: u64 = 1_000_000;
pub(crate) const DEFAULT_ZOOM_REGION_SCALE: f32 = 1.5;
pub(crate) const MIN_ZOOM_REGION_DURATION_US: u64 = 50_000;
pub(crate) const MIN_ZOOM_REGION_SCALE: f32 = 1.0;
pub(crate) const MAX_ZOOM_REGION_SCALE: f32 = 4.0;
const DEFAULT_TRANSITION_RATIO: u64 = 5;

mod cursor_size;

#[cfg(test)]
pub(crate) use cursor_size::CursorSizeEasing;
pub(crate) use cursor_size::{
    CursorSizeRegion, MIN_CURSOR_SIZE_REGION_DURATION_US, cursor_scale_at,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ZoomTarget {
    #[default]
    Cursor,
    CanvasCenter,
    #[serde(other)]
    Invalid,
}

impl ZoomTarget {
    pub(crate) fn normalized(self) -> Self {
        match self {
            Self::Cursor | Self::CanvasCenter => self,
            Self::Invalid => Self::CanvasCenter,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ZoomEasing {
    #[default]
    EaseInOut,
    #[serde(other)]
    Invalid,
}

impl ZoomEasing {
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
pub(crate) struct ZoomRegion {
    #[serde(default)]
    pub(crate) start_us: u64,
    #[serde(default)]
    pub(crate) end_us: u64,
    #[serde(default = "default_scale")]
    pub(crate) scale: f32,
    #[serde(default)]
    pub(crate) target: ZoomTarget,
    #[serde(default)]
    pub(crate) easing: ZoomEasing,
    #[serde(default)]
    pub(crate) zoom_in_end_us: Option<u64>,
    #[serde(default)]
    pub(crate) zoom_out_start_us: Option<u64>,
}

impl ZoomRegion {
    pub(crate) fn new_at(playhead_us: u64, duration_us: u64) -> Option<Self> {
        if duration_us == 0 {
            return None;
        }

        let start_us = playhead_us.min(duration_us);
        let end_us = start_us
            .saturating_add(DEFAULT_ZOOM_REGION_DURATION_US)
            .min(duration_us);
        let (start_us, end_us) = if end_us > start_us {
            (start_us, end_us)
        } else {
            (
                duration_us.saturating_sub(DEFAULT_ZOOM_REGION_DURATION_US),
                duration_us,
            )
        };

        (end_us > start_us).then_some(Self {
            start_us,
            end_us,
            scale: DEFAULT_ZOOM_REGION_SCALE,
            target: ZoomTarget::Cursor,
            easing: ZoomEasing::EaseInOut,
            zoom_in_end_us: None,
            zoom_out_start_us: None,
        })
    }

    pub(crate) fn normalized(mut self) -> Option<Self> {
        if self.end_us <= self.start_us {
            return None;
        }
        self.scale = if self.scale.is_finite() {
            self.scale
                .clamp(MIN_ZOOM_REGION_SCALE, MAX_ZOOM_REGION_SCALE)
        } else {
            DEFAULT_ZOOM_REGION_SCALE
        };
        self.target = self.target.normalized();
        self.easing = self.easing.normalized();
        let (zoom_in_end_us, zoom_out_start_us) = self.transition_points();
        self.zoom_in_end_us = Some(zoom_in_end_us);
        self.zoom_out_start_us = Some(zoom_out_start_us);
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
            .zoom_in_end_us
            .is_some_and(|time| time >= region.start_us && time <= region.end_us)
            && region
                .zoom_out_start_us
                .is_some_and(|time| time >= region.start_us && time <= region.end_us)
            && region.zoom_in_end_us <= region.zoom_out_start_us;
        if !transitions_fit {
            region.zoom_in_end_us = None;
            region.zoom_out_start_us = None;
        }
        region.normalized()
    }

    pub(crate) fn duration_us(self) -> u64 {
        self.end_us.saturating_sub(self.start_us)
    }

    pub(crate) fn transition_points(self) -> (u64, u64) {
        let duration = self.duration_us();
        let default_ramp = duration / DEFAULT_TRANSITION_RATIO;
        let default_zoom_in_end = self.start_us.saturating_add(default_ramp);
        let default_zoom_out_start = self.end_us.saturating_sub(default_ramp);
        let zoom_in_end = self
            .zoom_in_end_us
            .unwrap_or(default_zoom_in_end)
            .clamp(self.start_us, self.end_us);
        let zoom_out_start = self
            .zoom_out_start_us
            .unwrap_or(default_zoom_out_start)
            .clamp(self.start_us, self.end_us);

        if zoom_in_end <= zoom_out_start {
            (zoom_in_end, zoom_out_start)
        } else {
            let midpoint = self.start_us.saturating_add(duration / 2);
            (midpoint, midpoint)
        }
    }

    pub(crate) fn effect_at(self, playhead_us: u64) -> Option<ZoomEffect> {
        if playhead_us < self.start_us || playhead_us > self.end_us {
            return None;
        }

        if self.duration_us() == 0 {
            return None;
        }
        let (zoom_in_end_us, zoom_out_start_us) = self.transition_points();
        let envelope = match self.easing.normalized() {
            ZoomEasing::EaseInOut | ZoomEasing::Invalid => {
                if playhead_us <= self.start_us
                    || (playhead_us >= self.end_us && zoom_out_start_us < self.end_us)
                {
                    0.0
                } else if playhead_us <= zoom_in_end_us {
                    transition_progress(
                        playhead_us.saturating_sub(self.start_us),
                        zoom_in_end_us.saturating_sub(self.start_us),
                    )
                } else if zoom_out_start_us >= self.end_us {
                    // There is no presentation time after a region clipped to the
                    // recording end. Keep its final frame zoomed instead of making
                    // the last timestamp snap back to the identity camera.
                    1.0
                } else if playhead_us < zoom_out_start_us {
                    1.0
                } else {
                    transition_progress(
                        self.end_us.saturating_sub(playhead_us),
                        self.end_us.saturating_sub(zoom_out_start_us),
                    )
                }
            }
        };
        Some(ZoomEffect {
            scale: 1.0 + (self.scale - 1.0) * envelope,
            target: self.target.normalized(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ZoomEffect {
    pub(crate) scale: f32,
    pub(crate) target: ZoomTarget,
}

pub(crate) fn effect_at(regions: &[ZoomRegion], playhead_us: u64) -> Option<ZoomEffect> {
    regions
        .iter()
        .rev()
        .find_map(|region| region.effect_at(playhead_us))
}

fn transition_progress(elapsed_us: u64, duration_us: u64) -> f32 {
    if duration_us == 0 {
        1.0
    } else {
        smootherstep(elapsed_us as f64 / duration_us as f64)
    }
}

/// Eases both the velocity and acceleration to zero at each end of a ramp.
///
/// The quintic curve keeps the midpoint unchanged while spending a little
/// longer settling into and out of a zoom than cubic smoothstep does.
fn smootherstep(value: f64) -> f32 {
    let value = value.clamp(0.0, 1.0);
    (value * value * value * (value * (value * 6.0 - 15.0) + 10.0)) as f32
}

fn default_scale() -> f32 {
    DEFAULT_ZOOM_REGION_SCALE
}

#[cfg(test)]
#[path = "zoom/tests.rs"]
mod tests;
