//! GPU replacement for the comparator peer's CPU canvas (see PROVENANCE.txt).
//! Lom shell-gpu renders the same opaque shade per generation on the granted
//! render node: arms I and S read it back, arm G exports it as a DMA-BUF.
//! GPU execution time is not measured; frame cadence from the journal is the
//! primary result and the stderr preparation span is host wall-clock only.
#![allow(dead_code)] // arms I/S use the readback half, arm G the export half
use imaging::{Painter, kurbo::Rect, peniko::Color, record::Scene};
use sophia_shell_gpu::{GpuGrant, GpuPreview};
use std::{cell::RefCell, sync::OnceLock};

thread_local! {
    static GPU: RefCell<Option<GpuPreview>> = const { RefCell::new(None) };
}
static ARM: OnceLock<&'static str> = OnceLock::new();

/// Opens the granted device once ContentLimits names the connection epoch.
/// Stdout carries only the journal, so the admission goes to stderr.
pub fn open(connection_epoch: u64, arm: &'static str) -> Result<(), String> {
    ARM.set(arm).map_err(|_| "GPU already opened")?;
    let grant = GpuGrant::from_environment(connection_epoch)?;
    let gpu = GpuPreview::new(&grant)?;
    eprintln!(
        "gpu_bar schema=1 arm={arm} producer=vello_wgpu gpu_time=unavailable {}",
        gpu.evidence(&grant)?.record("gpu_bar")?
    );
    GPU.with(|cell| *cell.borrow_mut() = Some(gpu));
    Ok(())
}

/// One upload's preparation (render plus readback or export) on the journal's
/// monotonic clock. Stderr only, so the schema 2 journal is unchanged.
pub fn prepared(generation: u64, resource_id: u64, start_usec: u64, end_usec: u64) {
    eprintln!(
        "gpu_bar_prepare schema=1 arm={} generation={generation} resource_id={resource_id} start_usec={start_usec} end_usec={end_usec}",
        ARM.get().copied().unwrap_or("unopened")
    );
}

fn with_gpu<T>(work: impl FnOnce(&mut GpuPreview) -> Result<T, String>) -> Result<T, String> {
    GPU.with(|cell| work(cell.borrow_mut().as_mut().ok_or("GPU was not opened")?))
}

/// The comparator's pixels: opaque, every channel the generation's shade.
/// A grey is byte-identical in RGBA readback and XR24 export.
fn scene(width: u32, height: u32, generation: u64) -> Scene {
    let shade = (generation % 128) as u8;
    let mut scene = Scene::new();
    {
        let mut painter = Painter::new(&mut scene);
        let rect = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        painter.fill_rect(rect, Color::from_rgb8(shade, shade, shade));
    }
    scene
}

/// Bytes of a 4-byte-per-pixel image, or None when the size overflows.
pub fn rgba_bytes(width: u32, height: u32) -> Option<u64> {
    u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(4)
}

pub struct Canvas {
    bytes: Vec<u8>,
    capacity: usize,
}

impl Canvas {
    /// Bounds every upload by the largest configured one; nothing is allocated here.
    pub fn new(capacity: usize) -> Self {
        Self {
            bytes: Vec::new(),
            capacity,
        }
    }

    /// Renders and reads back one packed RGBA frame for the byte path.
    pub fn fill(
        &mut self,
        length: usize,
        width: u32,
        height: u32,
        generation: u64,
    ) -> Result<&[u8], String> {
        if length == 0 || length > self.capacity {
            return Err("upload exceeds the canvas".into());
        }
        self.bytes =
            with_gpu(|gpu| gpu.read_rgba(&mut scene(width, height, generation), width, height))?;
        if self.bytes.len() != length {
            return Err("GPU readback length differs from the upload".into());
        }
        Ok(&self.bytes)
    }
}

/// Renders, reads back and copies one frame into mapped sealed backing.
pub fn fill_pixels(
    bytes: &mut [u8],
    width: u32,
    height: u32,
    generation: u64,
) -> Result<(), String> {
    let pixels =
        with_gpu(|gpu| gpu.read_rgba(&mut scene(width, height, generation), width, height))?;
    if pixels.len() != bytes.len() {
        return Err("GPU readback length differs from the sealed backing".into());
    }
    bytes.copy_from_slice(&pixels);
    Ok(())
}

/// Renders one producer-completed frame for arm G; it holds no readback.
pub fn render_dma(
    width: u32,
    height: u32,
    generation: u64,
) -> Result<sophia_shell_gpu::DmaFrame, String> {
    with_gpu(|gpu| gpu.render_dma_buf(&mut scene(width, height, generation), width, height))
}
