//! TextGlow brand: one palette and one logo, shared by the tray icon, the
//! window icon, and every in-app surface. The look is "ember on charcoal":
//! a warm near-black background, amber as the single accent, and a logo of
//! a glowing T with a spark riding its shoulder.

use egui::Color32;

// ---- UI palette ----------------------------------------------------------------

/// The accent: warm amber, the color of the glow.
pub const ACCENT: Color32 = Color32::from_rgb(240, 158, 62);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(251, 176, 84);
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(222, 136, 44);
/// Text on top of the amber accent.
pub const ON_ACCENT: Color32 = Color32::from_rgb(43, 26, 8);
/// Dark-amber fill behind selected text / checked boxes.
pub const SELECTION: Color32 = Color32::from_rgb(122, 78, 24);
pub const ON_SELECTION: Color32 = Color32::from_rgb(255, 240, 220);

/// Warm charcoal surfaces, dark to dark.
pub const PANEL: Color32 = Color32::from_rgb(24, 20, 16);
/// Text-edit wells and other recessed areas.
pub const EXTREME: Color32 = Color32::from_rgb(13, 11, 8);
/// Barely-there striping tint.
pub const FAINT: Color32 = Color32::from_rgb(34, 28, 21);
/// Resting widget fill (buttons, inputs).
pub const SURFACE: Color32 = Color32::from_rgb(38, 31, 23);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(51, 41, 29);
pub const SURFACE_ACTIVE: Color32 = Color32::from_rgb(64, 50, 32);
/// Hairline borders.
pub const BORDER: Color32 = Color32::from_rgb(66, 55, 41);
pub const BORDER_HOVER: Color32 = Color32::from_rgb(122, 94, 55);

/// Warm off-white body text.
pub const TEXT: Color32 = Color32::from_rgb(237, 228, 211);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(160, 149, 132);

// Semantic status colors (also mirrored on the tray icon).
pub const SUCCESS: Color32 = Color32::from_rgb(140, 235, 160);
pub const WARN: Color32 = Color32::from_rgb(250, 196, 108);
pub const DANGER: Color32 = Color32::from_rgb(255, 130, 130);

// ---- Logo ----------------------------------------------------------------------

/// Colors for one rendering of the logo mark. Each app state tints the same
/// geometry, so the tray always reads as TextGlow whatever its health.
pub struct IconTheme {
    pub tile_top: [f32; 3],
    pub tile_bottom: [f32; 3],
    /// The T body.
    pub mark: [f32; 3],
    /// The spark, brighter than the body.
    pub spark: [f32; 3],
    /// Soft halo behind the T.
    pub glow: [f32; 3],
    pub glow_alpha: f32,
}

/// Ready: ember amber.
pub const READY: IconTheme = IconTheme {
    tile_top: [46.0, 36.0, 24.0],
    tile_bottom: [18.0, 14.0, 10.0],
    mark: [245.0, 166.0, 74.0],
    spark: [255.0, 218.0, 150.0],
    glow: [250.0, 176.0, 80.0],
    glow_alpha: 0.34,
};

/// Not configured yet: neutral gray.
pub const UNCONFIGURED: IconTheme = IconTheme {
    tile_top: [40.0, 38.0, 36.0],
    tile_bottom: [17.0, 16.0, 15.0],
    mark: [170.0, 170.0, 178.0],
    spark: [226.0, 226.0, 234.0],
    glow: [200.0, 200.0, 210.0],
    glow_alpha: 0.16,
};

/// Needs attention (hotkey conflict): red.
pub const HOTKEY_FAILED: IconTheme = IconTheme {
    tile_top: [48.0, 24.0, 22.0],
    tile_bottom: [20.0, 10.0, 9.0],
    mark: [240.0, 100.0, 92.0],
    spark: [255.0, 180.0, 168.0],
    glow: [245.0, 110.0, 95.0],
    glow_alpha: 0.32,
};

/// Render the logo mark as straight-alpha RGBA, `size` x `size` pixels.
///
/// The mark is a rounded tile holding a glowing "T" (text) with a four-point
/// spark at its top-right shoulder ("glow up"). All geometry is defined on a
/// 32-unit design grid and scaled, so any size renders identically.
pub fn icon_rgba(size: u32, theme: &IconTheme) -> Vec<u8> {
    // Design-grid (32 units) -> pixels.
    let u = size as f32 / 32.0;

    let tile_inset = 0.5 * u;
    let tile_radius = 7.5 * u;

    // The T: rounded crossbar + stem.
    let bar = (vec2(9.8, 10.8), vec2(19.2, 10.8), 2.3);
    let stem = (vec2(14.5, 10.8), vec2(14.5, 22.6), 2.3);

    // Four-point spark at the T's top-right shoulder.
    let spark_c = vec2(23.2, 8.2);
    let spark_arm = 4.4;
    let spark_half_width = 1.35;

    // Soft halo centered on the T.
    let glow_c = vec2(14.6, 14.2);
    let glow_sigma = 6.5 * u;

    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let p = vec2(px as f32 + 0.5, py as f32 + 0.5);

            // Tile coverage.
            let d_tile = sd_rounded_rect(p, size as f32, tile_inset, tile_radius);
            let tile_cov = coverage(d_tile);
            let tile_t = (p.y - tile_inset) / (size as f32 - 2.0 * tile_inset);
            let tile = lerp3(theme.tile_top, theme.tile_bottom, tile_t.clamp(0.0, 1.0));

            // Halo, clipped to the tile.
            let d2 = dist2(p, scale(glow_c, u));
            let glow_a =
                theme.glow_alpha * (-d2 / (2.0 * glow_sigma * glow_sigma)).exp() * tile_cov;

            // T body.
            let d_bar = sd_capsule(p, bar, u);
            let d_stem = sd_capsule(p, stem, u);
            let mark_cov = coverage(d_bar.min(d_stem));

            // Spark: two crossed thin diamonds.
            let d_spark = sd_diamond(p, spark_c, spark_arm, spark_half_width, u);
            let spark_cov = coverage(d_spark);

            // Composite (premultiplied), spark on top.
            let mut a = tile_cov;
            let mut r = tile[0] * tile_cov;
            let mut g = tile[1] * tile_cov;
            let mut b = tile[2] * tile_cov;
            for (col, cov) in [
                (theme.glow, glow_a),
                (theme.mark, mark_cov),
                (theme.spark, spark_cov),
            ] {
                r = col[0] * cov + r * (1.0 - cov);
                g = col[1] * cov + g * (1.0 - cov);
                b = col[2] * cov + b * (1.0 - cov);
                a = cov + a * (1.0 - cov);
            }

            let (sr, sg, sb) = if a > 0.0 {
                (r / a, g / a, b / a)
            } else {
                (0.0, 0.0, 0.0)
            };
            rgba.extend_from_slice(&[
                sr.round() as u8,
                sg.round() as u8,
                sb.round() as u8,
                (a * 255.0).round() as u8,
            ]);
        }
    }
    rgba
}

// ---- geometry helpers (f32 pair kept tuple-free for clarity) -------------------

#[derive(Clone, Copy)]
struct V2 {
    x: f32,
    y: f32,
}

fn vec2(x: f32, y: f32) -> V2 {
    V2 { x, y }
}

fn scale(p: V2, s: f32) -> V2 {
    V2 {
        x: p.x * s,
        y: p.y * s,
    }
}

fn dist2(p: V2, q: V2) -> f32 {
    let dx = p.x - q.x;
    let dy = p.y - q.y;
    dx * dx + dy * dy
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Anti-aliased coverage from a signed distance in pixels.
fn coverage(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Signed distance to a rounded rect of `size` on a side, inset and rounded.
fn sd_rounded_rect(p: V2, size: f32, inset: f32, radius: f32) -> f32 {
    let half = (size - 2.0 * inset) / 2.0;
    let c = size / 2.0;
    let dx = (p.x - c).abs() - (half - radius);
    let dy = (p.y - c).abs() - (half - radius);
    (dx.max(0.0).hypot(dy.max(0.0)) + dx.min(0.0).max(dy.min(0.0))) - radius
}

/// Signed distance (pixels) to a capsule defined on the 32-unit grid.
fn sd_capsule(p: V2, seg: (V2, V2, f32), u: f32) -> f32 {
    let a = scale(seg.0, u);
    let b = scale(seg.1, u);
    let r = seg.2 * u;
    let pax = p.x - a.x;
    let pay = p.y - a.y;
    let bax = b.x - a.x;
    let bay = b.y - a.y;
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    dx.hypot(dy) - r
}

/// Approximate signed distance (pixels) to a diamond centered at `c` with
/// half-length `half_len` and half-width `half_w`, both on the 32-unit grid.
fn sd_diamond(p: V2, c: V2, half_len: f32, half_w: f32, u: f32) -> f32 {
    let cx = c.x * u;
    let cy = c.y * u;
    let v = sd_diamond_half(p, cx, cy, half_w * u, half_len * u);
    let h = sd_diamond_half(p, cx, cy, half_len * u, half_w * u);
    v.min(h)
}

fn sd_diamond_half(p: V2, cx: f32, cy: f32, a: f32, b: f32) -> f32 {
    // |dx|/a + |dy|/b <= 1, scaled to pixel distance.
    ((p.x - cx).abs() / a + (p.y - cy).abs() / b - 1.0) * a.min(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px_at(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn icon_has_expected_length() {
        for size in [16, 32, 64, 128] {
            assert_eq!(icon_rgba(size, &READY).len(), (size * size * 4) as usize);
        }
    }

    #[test]
    fn tile_corners_are_transparent() {
        let rgba = icon_rgba(32, &READY);
        for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31)] {
            assert_eq!(
                px_at(&rgba, 32, x, y)[3],
                0,
                "corner ({x},{y}) not transparent"
            );
        }
    }

    #[test]
    fn t_body_is_opaque_and_amber() {
        let rgba = icon_rgba(32, &READY);
        // Stem center: grid (14.5, 16) -> pixel (14, 16).
        let p = px_at(&rgba, 32, 14, 16);
        assert_eq!(p[3], 255);
        assert!(p[0] > p[1] && p[1] > p[2], "stem should be warm, got {p:?}");
        // Crossbar center: grid (12, 10.8) -> pixel (12, 10).
        let p = px_at(&rgba, 32, 12, 10);
        assert_eq!(p[3], 255);
        assert!(p[0] > p[2], "bar should be warm, got {p:?}");
    }

    #[test]
    fn spark_is_brighter_than_body() {
        let rgba = icon_rgba(32, &READY);
        // Spark center: grid (23.2, 8.2) -> pixel (23, 8).
        let p = px_at(&rgba, 32, 23, 8);
        assert_eq!(p[3], 255);
        let body = px_at(&rgba, 32, 14, 16);
        assert!(
            p[0] >= body[0] && p[1] > body[1],
            "spark should be brighter"
        );
    }

    #[test]
    fn states_render_differently() {
        assert_ne!(icon_rgba(32, &READY), icon_rgba(32, &UNCONFIGURED));
        assert_ne!(icon_rgba(32, &READY), icon_rgba(32, &HOTKEY_FAILED));
    }

    #[test]
    fn ready_mark_is_not_violet() {
        // The old violet/pink icon had blue above red; amber is the reverse.
        let rgba = icon_rgba(32, &READY);
        let p = px_at(&rgba, 32, 14, 16);
        assert!(p[0] > p[2] + 50, "ready mark must be warm amber, got {p:?}");
    }
}
