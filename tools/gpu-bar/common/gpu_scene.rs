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
