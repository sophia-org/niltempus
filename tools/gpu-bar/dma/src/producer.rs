//! Producer selection changes pixel construction, never slot ownership.
use super::{Result, Source, canvas};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Producer {
    Gpu,
    Cpu,
}

impl Producer {
    pub(crate) fn open(self, connection_epoch: u64, source: Source, profile: bool) -> Result<()> {
        match self {
            Self::Gpu => {
                canvas::open(connection_epoch, source.label())?;
                canvas::with_gpu(|gpu| {
                    gpu.set_dma_timing_enabled(profile);
                    Ok(())
                })?;
            }
            Self::Cpu => {
                eprintln!(
                    "cpu_bar schema=1 source=owned_ring producer=cpu_shm fill=shade_mod128_a255"
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn fill_cpu(pixels: &mut [u8], generation: u64) {
    assert!(pixels.len().is_multiple_of(4));
    let shade = (generation % 128) as u8;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[shade, shade, shade, 255]);
    }
}

#[cfg(test)]
#[path = "../tests/support/producer.rs"]
mod tests;
