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

/// Width and height from a PNG's IHDR chunk.
fn png_size(name: &str) -> (u32, u32) {
    let path = format!("{}/icons/{name}", env!("CARGO_MANIFEST_DIR"));
    let data = fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n", "{name} is not a PNG");
    let be = |i: usize| u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    (be(16), be(20))
}

#[test]
fn the_pngs_have_the_sizes_in_their_names() {
    assert_eq!(png_size("32x32.png"), (32, 32));
    assert_eq!(png_size("128x128.png"), (128, 128));
    assert_eq!(png_size("128x128@2x.png"), (256, 256));
    assert_eq!(png_size("icon.png"), (512, 512));
}
