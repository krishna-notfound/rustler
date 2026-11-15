//! The app icon, drawn in code (no asset pipeline): an orange rounded square with a white
//! "play" triangle, anti-aliased by 4x4 supersampling. RGBA8, row-major.

/// Returns `size * size * 4` RGBA bytes.
pub fn rgba(size: u32) -> Vec<u8> {
    let s = size as f64;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    const SS: u32 = 4;
    let radius = s * 0.22;
    let inset = s * 0.04;
    // Play triangle (pointing right), slightly right of centre for optical balance.
    let tri = [(s * 0.38, s * 0.28), (s * 0.38, s * 0.72), (s * 0.74, s * 0.50)];
    for y in 0..size {
        for x in 0..size {
            let (mut bg, mut fg) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f64 + (sx as f64 + 0.5) / SS as f64;
                    let py = y as f64 + (sy as f64 + 0.5) / SS as f64;
                    if in_rounded_rect(px, py, inset, s - inset, radius) {
                        bg += 1;
                        if in_triangle(px, py, tri) {
                            fg += 1;
                        }
                    }
                }
            }
            let n = (SS * SS) as f64;
            let cover = bg as f64 / n;
            let white = if bg == 0 { 0.0 } else { fg as f64 / bg as f64 };
            // Vertical gradient #ff7a3d -> #e8432f.
            let t = y as f64 / s;
            let (r0, g0, b0) = (255.0 + (232.0 - 255.0) * t, 122.0 + (67.0 - 122.0) * t, 61.0 + (47.0 - 61.0) * t);
            let mix = |c: f64| (c + (255.0 - c) * white).round().clamp(0.0, 255.0) as u8;
            out.extend_from_slice(&[mix(r0), mix(g0), mix(b0), (cover * 255.0).round() as u8]);
        }
    }
    out
}

fn in_rounded_rect(x: f64, y: f64, lo: f64, hi: f64, r: f64) -> bool {
    if x < lo || y < lo || x > hi || y > hi {
        return false;
    }
    let cx = x.clamp(lo + r, hi - r);
    let cy = y.clamp(lo + r, hi - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn in_triangle(x: f64, y: f64, t: [(f64, f64); 3]) -> bool {
    let sign = |(ax, ay): (f64, f64), (bx, by): (f64, f64)| (x - bx) * (ay - by) - (ax - bx) * (y - by);
    let d1 = sign(t[0], t[1]);
    let d2 = sign(t[1], t[2]);
    let d3 = sign(t[2], t[0]);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_has_transparent_corners_white_centre_orange_edge() {
        let n = 64u32;
        let px = super::rgba(n);
        assert_eq!(px.len(), (n * n * 4) as usize);
        let at = |x: u32, y: u32| &px[((y * n + x) * 4) as usize..((y * n + x) * 4 + 4) as usize];
        assert_eq!(at(0, 0)[3], 0, "corner transparent");
        assert_eq!(at(34, 32), &[255, 255, 255, 255], "triangle is white");
        let edge = at(10, 32);
        assert_eq!(edge[3], 255);
        assert!(edge[0] > 200 && edge[2] < 100, "edge is orange: {edge:?}");
    }
}
