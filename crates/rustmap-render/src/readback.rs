use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::target::OutputTarget;

const SLOTS: usize = 3;

// Slot states.
const FREE: u8 = 0;
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const READY: u8 = 3;
const FAILED: u8 = 4;

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    frame: u64,
}

/// A finished readback: RGBA8 rows of `stride` bytes (padded to wgpu's 256-byte row alignment).
pub struct ReadbackFrame<'a> {
    pub size: [u32; 2],
    pub stride: u32,
    pub data: &'a [u8],
}

/// Copies an output target back to the CPU without stalling the GPU.
///
/// Each frame: [`capture`](Self::capture) records a copy into a free buffer,
/// [`after_submit`](Self::after_submit) starts mapping it, and a later frame
/// picks it up with [`read_latest`](Self::read_latest). With three buffers
/// the result is one or two frames old, which is normal for network outputs.
pub struct FrameReadback {
    size: [u32; 2],
    stride: u32,
    slots: Vec<Slot>,
    next_frame: u64,
}

impl FrameReadback {
    pub fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let stride = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("output readback"),
                    size: u64::from(stride) * u64::from(size[1]),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
                frame: 0,
            })
            .collect();
        Self {
            size,
            stride,
            slots,
            next_frame: 1,
        }
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Records a copy of `target` into a free buffer. Returns `false` (frame
    /// skipped) if all buffers are still busy.
    pub fn capture(&mut self, encoder: &mut wgpu::CommandEncoder, target: &OutputTarget) -> bool {
        if target.size() != self.size {
            return false;
        }
        let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.state.load(Ordering::Acquire) == FREE)
        else {
            return false;
        };
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: Some(self.size[1]),
                },
            },
            target.texture.size(),
        );
        slot.state.store(COPIED, Ordering::Release);
        slot.frame = self.next_frame;
        self.next_frame += 1;
        true
    }

    /// Starts mapping the buffers copied this frame. Call after `queue.submit`.
    pub fn after_submit(&mut self) {
        for slot in &self.slots {
            if slot.state.load(Ordering::Acquire) == COPIED {
                slot.state.store(MAPPING, Ordering::Release);
                let state = slot.state.clone();
                slot.buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        state.store(
                            if result.is_ok() { READY } else { FAILED },
                            Ordering::Release,
                        );
                    });
            }
        }
    }

    /// Hands the newest finished frame to `f`, then recycles every finished
    /// buffer. Needs `device.poll` to have run so mapping callbacks fire.
    pub fn read_latest<R>(&mut self, f: impl FnOnce(ReadbackFrame<'_>) -> R) -> Option<R> {
        let newest = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.state.load(Ordering::Acquire) == READY)
            .max_by_key(|(_, s)| s.frame)
            .map(|(i, _)| i);

        let result = newest.and_then(|i| {
            let slot = &self.slots[i];
            let view = slot.buffer.slice(..).get_mapped_range().ok()?;
            Some(f(ReadbackFrame {
                size: self.size,
                stride: self.stride,
                data: &view,
            }))
        });

        for slot in &self.slots {
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    slot.buffer.unmap();
                    slot.state.store(FREE, Ordering::Release);
                }
                FAILED => slot.state.store(FREE, Ordering::Release),
                _ => {}
            }
        }
        result
    }
}
