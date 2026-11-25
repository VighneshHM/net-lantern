//! The Windows icon must carry every size the tray, taskbar and shell ask for.

use std::fs;

#[test]
fn the_icon_has_every_tray_and_shell_size() {
    let data = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/icon.ico")).unwrap();
    assert_eq!(u16::from_le_bytes([data[0], data[1]]), 0, "reserved");
    assert_eq!(u16::from_le_bytes([data[2], data[3]]), 1, "type is icon");
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    // A width byte of 0 means 256.
    let widths: Vec<u32> = (0..count)
        .map(|i| match data[6 + 16 * i] {
            0 => 256,
            w => u32::from(w),
        })
        .collect();
    for want in [16, 24, 32, 48, 256] {
        assert!(widths.contains(&want), "missing {want}: {widths:?}");
    }
}
