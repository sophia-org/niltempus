//! Shared opaque scene for the Sophia peer and external GPU bridge.
use imaging::{Painter, kurbo::Rect, peniko::Color, record::Scene};

/// The comparator's pixels: opaque, every channel the generation's shade.
/// RGB agrees in RGBA readback and XR24 export; XR24 padding is unspecified.
pub(crate) fn scene(width: u32, height: u32, generation: u64) -> Scene {
    let shade = (generation % 128) as u8;
    let mut scene = Scene::new();
    {
        let mut painter = Painter::new(&mut scene);
        let rect = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        painter.fill_rect(rect, Color::from_rgb8(shade, shade, shade));
    }
    scene
}

/// Untimed consumer oracle. The repeated first pattern needs a generation join.
#[allow(dead_code)] // Shared source also belongs to ordinary comparator builds.
pub(crate) fn colour_scene(width: u32, height: u32, generation: u64) -> Result<Scene, String> {
    if width != 1920 || height != 24 || !(1..=3).contains(&generation) {
        return Err("colour hold requires 1920x24 and generations 1..3".into());
    }
    let (left, right) = if generation == 2 {
        (Color::from_rgb8(0, 255, 0), Color::from_rgb8(255, 255, 255))
    } else {
        (Color::from_rgb8(255, 0, 0), Color::from_rgb8(0, 0, 255))
    };
    let mut scene = Scene::new();
    {
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(Rect::new(0.0, 0.0, 960.0, 24.0), left);
        painter.fill_rect(Rect::new(960.0, 0.0, 1920.0, 24.0), right);
    }
    Ok(scene)
}
