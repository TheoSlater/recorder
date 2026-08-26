use super::super::{
    cursor::{CursorEvent, CursorSample},
    input::MouseEventKind,
};

pub(super) const CLICK_CLUSTER_GAP_US: u64 = 2_500_000;

const DOUBLE_CLICK_GAP_US: u64 = 500_000;
const MAX_CLICK_DISTANCE: f32 = 0.025;
const SINGLE_CLICK_STRENGTH: u16 = 900;
const DOUBLE_CLICK_STRENGTH: u16 = 1_500;
const CONTEXT_CLICK_STRENGTH: u16 = 1_200;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Click {
    pub(super) timestamp_us: u64,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) strength: u16,
    button: ClickButton,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ClickButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ClickCluster {
    pub(super) start_us: u64,
    pub(super) end_us: u64,
    pub(super) focus: Click,
}

#[derive(Clone, Copy)]
struct NormalizedEvent {
    timestamp_us: u64,
    x: f32,
    y: f32,
    kind: MouseEventKind,
}

#[derive(Clone, Copy)]
struct SamplePoint {
    timestamp_us: u64,
    x: f32,
    y: f32,
}

/// Converts the recorder's button transitions into explicit click interactions.
/// Cursor movement, dwell, and a drag that moves beyond the click tolerance do not
/// create an auto-zoom candidate.
pub(super) fn extract(
    samples: &[CursorSample],
    events: &[CursorEvent],
    duration_us: u64,
) -> Vec<Click> {
    if duration_us == 0 {
        return Vec::new();
    }

    let samples = normalize_samples(samples, duration_us);
    let mut events: Vec<_> = events
        .iter()
        .filter_map(|event| normalize_event(event, duration_us))
        .collect();
    events.sort_by_key(|event| event.timestamp_us);

    let mut active = [None; 3];
    let mut clicks = Vec::new();
    for event in events {
        if let Some(button) = down_button(event.kind) {
            active[button.index()] = Some(event);
            continue;
        }

        let Some(button) = up_button(event.kind) else {
            continue;
        };
        let Some(start) = active[button.index()].take() else {
            continue;
        };
        if is_click(start, event, &samples) {
            clicks.push(Click {
                timestamp_us: start.timestamp_us,
                x: start.x,
                y: start.y,
                strength: button.strength(),
                button,
            });
        }
    }

    mark_double_clicks(&mut clicks);
    clicks.sort_by_key(|click| click.timestamp_us);
    clicks
}

pub(super) fn cluster(mut clicks: Vec<Click>) -> Vec<ClickCluster> {
    clicks.sort_by_key(|click| click.timestamp_us);
    let mut clusters = Vec::new();

    for click in clicks {
        let joins_previous = clusters.last().is_some_and(|cluster: &ClickCluster| {
            click.timestamp_us.saturating_sub(cluster.end_us) <= CLICK_CLUSTER_GAP_US
        });
        if joins_previous {
            let cluster = clusters
                .last_mut()
                .expect("a click can only join an existing cluster");
            cluster.end_us = cluster.end_us.max(click.timestamp_us);
            if click.strength > cluster.focus.strength
                || (click.strength == cluster.focus.strength
                    && click.timestamp_us >= cluster.focus.timestamp_us)
            {
                cluster.focus = click;
            }
        } else {
            clusters.push(ClickCluster {
                start_us: click.timestamp_us,
                end_us: click.timestamp_us,
                focus: click,
            });
        }
    }

    clusters
}

fn normalize_event(event: &CursorEvent, duration_us: u64) -> Option<NormalizedEvent> {
    (event.normalized_x.is_finite() && event.normalized_y.is_finite()).then_some(NormalizedEvent {
        timestamp_us: event.timestamp_us.min(duration_us),
        x: event.normalized_x.clamp(0.0, 1.0),
        y: event.normalized_y.clamp(0.0, 1.0),
        kind: event.kind,
    })
}

fn normalize_samples(samples: &[CursorSample], duration_us: u64) -> Vec<SamplePoint> {
    let mut samples: Vec<_> = samples
        .iter()
        .filter(|sample| sample.normalized_x.is_finite() && sample.normalized_y.is_finite())
        .map(|sample| SamplePoint {
            timestamp_us: sample.timestamp_us.min(duration_us),
            x: sample.normalized_x.clamp(0.0, 1.0),
            y: sample.normalized_y.clamp(0.0, 1.0),
        })
        .collect();
    samples.sort_by_key(|sample| sample.timestamp_us);
    samples
}

fn is_click(start: NormalizedEvent, end: NormalizedEvent, samples: &[SamplePoint]) -> bool {
    if end.timestamp_us < start.timestamp_us
        || distance((start.x, start.y), (end.x, end.y)) > MAX_CLICK_DISTANCE
    {
        return false;
    }

    !samples.iter().any(|sample| {
        (start.timestamp_us..=end.timestamp_us).contains(&sample.timestamp_us)
            && distance((start.x, start.y), (sample.x, sample.y)) > MAX_CLICK_DISTANCE
    })
}

fn mark_double_clicks(clicks: &mut [Click]) {
    let mut last_left = None;
    for click in clicks {
        if click.button != ClickButton::Left {
            continue;
        }
        if last_left.is_some_and(|previous: Click| {
            click.timestamp_us.saturating_sub(previous.timestamp_us) <= DOUBLE_CLICK_GAP_US
        }) {
            click.strength = DOUBLE_CLICK_STRENGTH;
        }
        last_left = Some(*click);
    }
}

fn down_button(kind: MouseEventKind) -> Option<ClickButton> {
    match kind {
        MouseEventKind::LeftDown => Some(ClickButton::Left),
        MouseEventKind::RightDown => Some(ClickButton::Right),
        MouseEventKind::MiddleDown => Some(ClickButton::Middle),
        MouseEventKind::LeftUp | MouseEventKind::RightUp | MouseEventKind::MiddleUp => None,
    }
}

fn up_button(kind: MouseEventKind) -> Option<ClickButton> {
    match kind {
        MouseEventKind::LeftUp => Some(ClickButton::Left),
        MouseEventKind::RightUp => Some(ClickButton::Right),
        MouseEventKind::MiddleUp => Some(ClickButton::Middle),
        MouseEventKind::LeftDown | MouseEventKind::RightDown | MouseEventKind::MiddleDown => None,
    }
}

impl ClickButton {
    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Middle => 2,
        }
    }

    fn strength(self) -> u16 {
        match self {
            Self::Left => SINGLE_CLICK_STRENGTH,
            Self::Right | Self::Middle => CONTEXT_CLICK_STRENGTH,
        }
    }
}

fn distance(left: (f32, f32), right: (f32, f32)) -> f32 {
    let dx = left.0 - right.0;
    let dy = left.1 - right.1;
    (dx * dx + dy * dy).sqrt()
}
