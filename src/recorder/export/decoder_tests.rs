use super::FrameRate;

#[test]
fn frame_rate_generates_original_timestamps() {
    let rate = FrameRate {
        numerator: 60,
        denominator: 1,
    };

    assert_eq!(rate.frame_count(10_000_000), 60);
    assert_eq!(rate.timestamp(0), 0);
    assert_eq!(rate.timestamp(1), 166_666);
    assert_eq!(rate.frame_duration(0), 166_666);
}

#[test]
fn frame_duration_preserves_fractional_rates() {
    let rate = FrameRate {
        numerator: 30_000,
        denominator: 1_001,
    };

    assert_eq!(rate.frame_duration(0), 333_666);
    assert_eq!(rate.frame_duration(1), 333_667);
}
