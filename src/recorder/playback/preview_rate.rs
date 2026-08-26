#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum PreviewRate {
    Fps24,
    Fps30,
    #[default]
    Fps60,
}

pub(super) const PREVIEW_RATES: [PreviewRate; 3] =
    [PreviewRate::Fps24, PreviewRate::Fps30, PreviewRate::Fps60];

impl PreviewRate {
    pub(super) fn fps(self) -> u32 {
        match self {
            Self::Fps24 => 24,
            Self::Fps30 => 30,
            Self::Fps60 => 60,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Fps24 => "24 FPS",
            Self::Fps30 => "30 FPS",
            Self::Fps60 => "60 FPS",
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Fps24 => "preview-fps-24",
            Self::Fps30 => "preview-fps-30",
            Self::Fps60 => "preview-fps-60",
        }
    }

    /// Maps a media timestamp to the nearest frame slot at this preview rate.
    /// Slots let 24 FPS select the correct 2/3-frame cadence from 60 FPS media.
    pub(super) fn frame_slot(self, seconds: f64) -> Option<u64> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        Some((seconds * f64::from(self.fps())).round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewRate;

    #[test]
    fn keeps_preview_rates_distinct() {
        assert_eq!(PreviewRate::Fps24.fps(), 24);
        assert_eq!(PreviewRate::Fps30.fps(), 30);
        assert_eq!(PreviewRate::Fps60.fps(), 60);
    }

    #[test]
    fn maps_media_time_to_rate_slots() {
        assert_eq!(PreviewRate::Fps24.frame_slot(1.0 / 24.0), Some(1));
        assert_eq!(PreviewRate::Fps30.frame_slot(1.0 / 30.0), Some(1));
        assert_eq!(PreviewRate::Fps60.frame_slot(1.0 / 60.0), Some(1));
        assert_eq!(PreviewRate::Fps24.frame_slot(f64::NAN), None);
    }
}
