//! Task 3 timemap tests.

use rustler_core::model::*;
use rustler_core::*;

fn seg(source: &SourceId, start: f64, end: f64) -> Segment {
    Segment {
        id: SegmentId::new(),
        source_id: source.clone(),
        start,
        end,
        layout: Layout { preset: None, regions: vec![], background: Background::Black },
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn maps_two_segments() {
    let a = SourceId::new();
    let b = SourceId::new();
    let tm = TimeMap::from_segments(&[seg(&a, 10.0, 20.0), seg(&b, 50.0, 55.0)]);
    assert!(close(tm.duration(), 15.0));
    assert!(close(tm.to_output(&a, 12.0).unwrap(), 2.0));
    assert!(close(tm.to_output(&b, 51.0).unwrap(), 11.0));
    assert_eq!(tm.to_output(&a, 25.0), None);
    assert_eq!(tm.to_output(&b, 12.0), None); // right time, wrong source
    // Output 10..15 is segment B (50..55). The plan text says to_source(12.0) == (B, 51.0),
    // which contradicts its own to_output(B, 51) == 11; 12.0 s of output is source 52.0.
    let (sid, t) = tm.to_source(12.0).unwrap();
    assert_eq!(sid, b);
    assert!(close(t, 52.0));
    let (sid, t) = tm.to_source(11.0).unwrap();
    assert_eq!(sid, b);
    assert!(close(t, 51.0));
    let (sid, t) = tm.to_source(0.0).unwrap();
    assert_eq!(sid, a);
    assert!(close(t, 10.0));
}

#[test]
fn boundaries_are_half_open() {
    let a = SourceId::new();
    let b = SourceId::new();
    let tm = TimeMap::from_segments(&[seg(&a, 10.0, 20.0), seg(&b, 50.0, 55.0)]);
    assert_eq!(tm.to_output(&a, 20.0), None);
    assert_eq!(tm.to_source(15.0), None); // end of timeline
    assert_eq!(tm.to_source(-0.1), None);
    let (sid, _) = tm.to_source(10.0).unwrap(); // boundary belongs to the next segment
    assert_eq!(sid, b);
}

#[test]
fn empty_and_placement() {
    let tm = TimeMap::from_segments(&[]);
    assert_eq!(tm.duration(), 0.0);
    assert_eq!(tm.to_source(0.0), None);
    let a = SourceId::new();
    let tm = TimeMap::from_segments(&[seg(&a, 1.0, 3.0), seg(&a, 7.0, 8.5)]);
    let s = tm.segments();
    assert_eq!(s.len(), 2);
    assert!(close(s[0].out_start, 0.0) && close(s[0].out_end, 2.0));
    assert!(close(s[1].out_start, 2.0) && close(s[1].out_end, 3.5));
    // Same source twice: first segment containing the time wins.
    assert!(close(tm.to_output(&a, 7.5).unwrap(), 2.5));
}
