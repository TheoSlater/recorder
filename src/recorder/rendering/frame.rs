/// Identity of a decoded frame, carried from the decoder to the compositor.
///
/// The renderer never reasons about media time; it only needs enough identity
/// to reject work the playback pipeline has already superseded. This mirrors
/// the rules the existing playback view applies before presenting a frame, so
/// the native path inherits latest-request-wins seeking rather than re-deriving
/// it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameId {
    /// Seek generation the frame was decoded for. A frame from an older
    /// generation belongs to a seek the user has already replaced.
    pub(crate) generation: u64,
    /// Monotonic decode order within a generation.
    pub(crate) sequence: u64,
    pub(crate) timestamp_us: u64,
}

impl FrameId {
    pub(crate) fn new(generation: u64, sequence: u64, timestamp_us: u64) -> Self {
        Self {
            generation,
            sequence,
            timestamp_us,
        }
    }
}

/// Single-slot frame hand-off to the compositor.
///
/// The queue holds at most one frame on purpose. A GPU frame queue that grew
/// with decoder throughput would present stale pictures during a seek and pin
/// texture memory; keeping the newest valid frame instead means a late decode
/// is dropped rather than shown.
#[derive(Debug, Default)]
pub(crate) struct FrameQueue<T> {
    pending: Option<(FrameId, T)>,
    /// The newest generation a frame has been accepted for. A decode that
    /// finishes after the user has moved on belongs to a request that no longer
    /// exists, so it is dropped rather than shown.
    generation: u64,
    dropped: u64,
}

impl<T> FrameQueue<T> {
    pub(crate) fn new() -> Self {
        Self {
            pending: None,
            generation: 0,
            dropped: 0,
        }
    }

    /// Offers a decoded frame. Returns false when the frame was rejected as
    /// stale, either because its seek generation is obsolete or because a newer
    /// frame from the same generation is already waiting.
    pub(crate) fn offer(&mut self, id: FrameId, frame: T) -> bool {
        if id.generation < self.generation {
            self.dropped += 1;
            return false;
        }
        self.generation = id.generation;
        if let Some((pending, _)) = self.pending.as_ref()
            && pending.generation == id.generation
            && pending.sequence > id.sequence
        {
            self.dropped += 1;
            return false;
        }
        if self.pending.replace((id, frame)).is_some() {
            // The previous frame never reached the screen: coalesced, not shown.
            self.dropped += 1;
        }
        true
    }

    /// Takes the newest frame that is still valid to present.
    pub(crate) fn take(&mut self) -> Option<(FrameId, T)> {
        let (id, frame) = self.pending.take()?;
        if id.generation < self.generation {
            self.dropped += 1;
            return None;
        }
        Some((id, frame))
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}
