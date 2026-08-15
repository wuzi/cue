use std::{fs, path::Path};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[test]
fn packaged_application_icons_are_rgba_pngs_at_each_declared_size() {
    for size in [128_u32, 256, 512] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "data/icons/hicolor/{size}x{size}/apps/io.github.wuzi.RemindMe.png"
        ));
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));

        assert!(
            bytes.len() >= 26,
            "{} has no complete PNG header",
            path.display()
        );
        assert_eq!(
            &bytes[..8],
            PNG_SIGNATURE,
            "{} is not a PNG",
            path.display()
        );
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            size,
            "{} has the wrong width",
            path.display()
        );
        assert_eq!(
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
            size,
            "{} has the wrong height",
            path.display()
        );
        assert_eq!(bytes[24], 8, "{} is not 8-bit", path.display());
        assert_eq!(bytes[25], 6, "{} is not RGBA", path.display());
    }
}
