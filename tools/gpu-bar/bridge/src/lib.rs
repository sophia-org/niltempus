//! In-process fresh DMA-BUF producer for the matched external benchmark.
mod device;
mod ffi;
#[path = "../../common/gpu_scene.rs"]
mod gpu_scene;
mod slots;

pub use ffi::*;
use sophia_shell_gpu::{DmaFrame, GpuPreview};
use std::os::fd::IntoRawFd as _;
use std::path::Path;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 24;
const XR24: u32 = u32::from_le_bytes(*b"XR24");

/// Opaque C owner. Calls must be serialized; consumers own release correctness.
pub struct GpuBarBridge {
    renderer: GpuPreview,
    frames: slots::Slots<DmaFrame>,
    device: device::Device,
}

impl GpuBarBridge {
    fn open(path: &Path, pci: &str) -> Result<Self, String> {
        let device = device::Device::resolve(path, pci)?;
        let (renderer, evidence) = GpuPreview::new_external(path, device.major, device.minor)?;
        device.verify(&evidence)?;
        Ok(Self {
            renderer,
            frames: slots::Slots::new(),
            device,
        })
    }

    fn render(&mut self, generation: u64) -> Result<GpuBarFrame, String> {
        let (slot, token) = self.frames.prepare(generation)?;
        let rendered = self.renderer.render_dma_buf(
            &mut gpu_scene::scene(WIDTH, HEIGHT, generation),
            WIDTH,
            HEIGHT,
        );
        let frame = match rendered {
            Ok(frame) => frame,
            Err(error) => {
                // Keep the renderer and any uncertain GPU work until process exit.
                self.frames.poisoned = true;
                return Err(error);
            }
        };
        let fd = match frame.dup_fd() {
            Ok(fd) => fd,
            Err(error) => {
                self.frames.poisoned = true;
                return Err(error);
            }
        };
        let mut info = GpuBarFrame {
            token,
            generation,
            offset: u64::from(frame.offset()),
            source_bytes: frame.source_bytes(),
            modifier: 0,
            width: frame.width(),
            height: frame.height(),
            stride: frame.stride(),
            format: XR24,
            fd: -1,
        };
        self.frames.commit(slot, token, generation, frame);
        // No fallible operation after transferring the duplicate descriptor to C.
        info.fd = fd.into_raw_fd();
        Ok(info)
    }
}
