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
        let _ = segs;
        todo!("Task 3: TimeMap::from_segments")
    }

    /// Total output duration in seconds.
    pub fn duration(&self) -> f64 {
        todo!("Task 3: TimeMap::duration")
    }

    /// Output time for source time `t` of `source_id` (first segment containing it, `[start, end)`).
    pub fn to_output(&self, source_id: &SourceId, t: f64) -> Option<f64> {
        let _ = (source_id, t);
        todo!("Task 3: TimeMap::to_output")
    }

    /// Source id and source time for output time `out_t`.
    pub fn to_source(&self, out_t: f64) -> Option<(SourceId, f64)> {
        let _ = out_t;
        todo!("Task 3: TimeMap::to_source")
    }

    /// Segments in output order with their output placement.
    pub fn segments(&self) -> &[MappedSegment] {
        &self.segments
    }
}
