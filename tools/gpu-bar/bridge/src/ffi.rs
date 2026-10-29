use crate::GpuBarBridge;
use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

/// One fresh, producer-completed XR24/LINEAR frame; C owns `fd` on success.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GpuBarFrame {
    pub token: u64,
    pub generation: u64,
    pub offset: u64,
    pub source_bytes: u64,
    pub modifier: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub fd: i32,
}

impl Default for GpuBarFrame {
    fn default() -> Self {
        Self {
            token: 0,
            generation: 0,
            offset: 0,
            source_bytes: 0,
            modifier: 0,
            width: 0,
            height: 0,
            stride: 0,
            format: 0,
            fd: -1,
        }
    }
}

/// Verified external device identity; no Sophia authority or grant fields.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuBarDevice {
    pub schema: u32,
    pub major: u32,
    pub minor: u32,
    pub pci_domain: u32,
    pub pci_bus: u32,
    pub pci_slot: u32,
    pub pci_function: u32,
}

unsafe fn error_text(dst: *mut c_char, capacity: usize, text: &str) {
    if !dst.is_null() && capacity > 0 {
        let count = text.len().min(capacity - 1);
        // SAFETY: the C caller supplies a writable buffer of `capacity` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), dst.cast(), count);
            dst.add(count).write(0);
        }
    }
}

unsafe fn boundary(
    error: *mut c_char,
    capacity: usize,
    work: impl FnOnce() -> Result<(), String>,
) -> i32 {
    // SAFETY: inherited from each exported entry point's buffer contract.
    unsafe { error_text(error, capacity, "") };
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(Ok(())) => 0,
        Ok(Err(text)) => {
            unsafe { error_text(error, capacity, &text) };
            -1
        }
        Err(_) => {
            // GPU state after an unexpected unwind is not recoverable evidence.
            eprintln!("gpu_bar_bridge fatal=panic");
            std::process::abort();
        }
    }
}

unsafe fn text<'a>(ptr: *const c_char) -> Result<&'a str, String> {
    if ptr.is_null() {
        return Err("null input string".into());
    }
    // SAFETY: caller supplies a readable NUL-terminated string.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| "input is not UTF-8".into())
}

/// Open one external producer, returning a null owner on failure.
///
/// # Safety
/// Strings must be NUL-terminated; output and error storage must be valid,
/// writable and disjoint. See `gpu_bar_bridge.h` for the common C contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_bar_bridge_open(
    node: *const c_char,
    pci: *const c_char,
    out: *mut *mut GpuBarBridge,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            if out.is_null() {
                return Err("null owner output".into());
            }
            out.write(std::ptr::null_mut());
            let owner = GpuBarBridge::open(Path::new(text(node)?), text(pci)?)?;
            out.write(Box::into_raw(Box::new(owner)));
            Ok(())
        })
    }
}

/// Render the next generation; output remains empty with fd=-1 on failure.
///
/// # Safety
/// `owner` must be live and exclusively borrowed for this call. Output/error
/// storage must be valid, writable and disjoint from each other and the owner.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_bar_bridge_render(
    owner: *mut GpuBarBridge,
    generation: u64,
    out: *mut GpuBarFrame,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            if out.is_null() {
                return Err("null frame output".into());
            }
            out.write(GpuBarFrame::default());
            let owner = owner.as_mut().ok_or("null owner")?;
            out.write(owner.render(generation)?);
            Ok(())
        })
    }
}

/// Drop exactly one retained frame after the consumer releases it.
///
/// # Safety
/// Owner must be live and exclusive; error buffer valid. The caller must prove
/// consumer release and close/transfer every duplicate FD before releasing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_bar_bridge_release(
    owner: *mut GpuBarBridge,
    token: u64,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            let owner = owner.as_mut().ok_or("null owner")?;
            drop(owner.frames.release(token)?);
            Ok(())
        })
    }
}

/// Read the constructor's verified external-device evidence.
///
/// # Safety
/// Owner must be live and exclusive. Output/error storage must be writable,
/// valid, and disjoint from each other and the owner.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_bar_bridge_device(
    owner: *mut GpuBarBridge,
    out: *mut GpuBarDevice,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            if out.is_null() {
                return Err("null device output".into());
            }
            out.write(GpuBarDevice::default());
            let device = &owner.as_ref().ok_or("null owner")?.device;
            // The normalized PCI string was validated before Vulkan initialization.
            let part = |r: std::ops::Range<usize>| u32::from_str_radix(&device.pci[r], 16).unwrap();
            out.write(GpuBarDevice {
                schema: 1,
                major: device.major,
                minor: device.minor,
                pci_domain: part(0..4),
                pci_bus: part(5..7),
                pci_slot: part(8..10),
                pci_function: part(11..12),
            });
            Ok(())
        })
    }
}

/// Close only a clean, drained producer; leave its pointer intact on refusal.
///
/// # Safety
/// `owner` must point to a live owner pointer, with no concurrent access; error
/// storage must be valid, writable and disjoint. Success invalidates all aliases.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_bar_bridge_close(
    owner: *mut *mut GpuBarBridge,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            let ptr = owner.as_mut().ok_or("null owner output")?;
            ptr.as_ref().ok_or("null owner")?.frames.can_close()?;
            let value = Box::from_raw(*ptr);
            *ptr = std::ptr::null_mut();
            drop(value);
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "../tests/support/ffi.rs"]
mod tests;
