use super::*;
use std::os::unix::process::ExitStatusExt as _;

#[test]
fn c_boundary_refusals_clear_outputs_and_bound_error_writes() {
    let mut error = [42_i8; 5];
    let mut owner = std::ptr::null_mut();
    // SAFETY: all inputs point to live disjoint storage; no GPU path is valid.
    unsafe {
        assert_eq!(
            gpu_bar_bridge_open(
                c"/dev/null".as_ptr(),
                c"0000:03:00.0".as_ptr(),
                &mut owner,
                error.as_mut_ptr(),
                error.len()
            ),
            -1
        );
        assert!(owner.is_null());
        assert_eq!(error[4], 0);
        let mut frame = GpuBarFrame {
            fd: 99,
            token: 99,
            ..Default::default()
        };
        assert_eq!(
            gpu_bar_bridge_render(owner, 1, &mut frame, std::ptr::null_mut(), 0),
            -1
        );
        assert_eq!(frame.fd, -1);
        assert_eq!(frame.token, 0);
        let mut device = GpuBarDevice {
            schema: 99,
            ..Default::default()
        };
        assert_eq!(
            gpu_bar_bridge_device(owner, &mut device, error.as_mut_ptr(), 1),
            -1
        );
        assert_eq!(device.schema, 0);
        assert_eq!(error[0], 0);
        assert_eq!(
            gpu_bar_bridge_release(owner, 1, std::ptr::null_mut(), 0),
            -1
        );
        assert_eq!(
            gpu_bar_bridge_close(&mut owner, std::ptr::null_mut(), 0),
            -1
        );
        assert!(owner.is_null());
        let mut sentry = [7_i8; 8];
        assert_eq!(
            boundary(sentry.as_mut_ptr().add(2), 4, || Err("long error".into())),
            -1
        );
        assert_eq!(sentry, [7, 7, b'l' as i8, b'o' as i8, b'n' as i8, 0, 7, 7]);
    }
}

#[test]
fn panic_aborts_instead_of_returning_uncertain_gpu_state() {
    const CHILD: &str = "GPU_BAR_PANIC_CHILD";
    if std::env::var_os(CHILD).is_some() {
        unsafe { boundary(std::ptr::null_mut(), 0, || panic!("test boundary")) };
        unreachable!();
    }
    // Disable core dumps only in this test child; no GPU is constructed.
    let result = std::process::Command::new("/bin/sh")
        .args(["-c", "ulimit -c 0; exec \"$@\"", "panic-child"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ffi::tests::panic_aborts_instead_of_returning_uncertain_gpu_state",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert_eq!(result.status.signal(), Some(6));
    assert!(String::from_utf8_lossy(&result.stderr).contains("gpu_bar_bridge fatal=panic"));
}
