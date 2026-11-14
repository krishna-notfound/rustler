//! Mapping between source time and output (clip) time.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::ids::{SegmentId, SourceId};
use crate::model::Segment;
use serde::Serialize;

/// One segment placed on the output timeline.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct MappedSegment {
    pub segment_id: SegmentId,
    pub source_id: SourceId,
    pub src_start: f64,
    pub src_end: f64,
    pub out_start: f64,
    pub out_end: f64,
}

#[derive(Clone, Debug, Default)]
pub struct TimeMap {
    segments: Vec<MappedSegment>,
}

impl TimeMap {
    pub fn from_segments(segs: &[Segment]) -> Self {
        let mut out = 0.0;
        let segments = segs
            .iter()
            .map(|s| {
                let len = (s.end - s.start).max(0.0);
                let m = MappedSegment {
                    segment_id: s.id.clone(),
                    source_id: s.source_id.clone(),
                    src_start: s.start,
                    src_end: s.end,
                    out_start: out,
                    out_end: out + len,
                };
                out += len;
                m
            })
            .collect();
        Self { segments }
    }

    /// Total output duration in seconds.
    pub fn duration(&self) -> f64 {
        self.segments.last().map_or(0.0, |s| s.out_end)
    }

    /// Output time for source time `t` of `source_id` (first segment containing it, `[start, end)`).
    pub fn to_output(&self, source_id: &SourceId, t: f64) -> Option<f64> {
        self.segments
            .iter()
            .find(|s| &s.source_id == source_id && t >= s.src_start && t < s.src_end)
            .map(|s| s.out_start + (t - s.src_start))
    }

    /// Source id and source time for output time `out_t` (`[out_start, out_end)` per segment).
    pub fn to_source(&self, out_t: f64) -> Option<(SourceId, f64)> {
        self.segments
            .iter()
            .find(|s| out_t >= s.out_start && out_t < s.out_end)
            .map(|s| (s.source_id.clone(), s.src_start + (out_t - s.out_start)))
    }

    /// Segments in output order with their output placement.
    pub fn segments(&self) -> &[MappedSegment] {
        &self.segments
    }
}
