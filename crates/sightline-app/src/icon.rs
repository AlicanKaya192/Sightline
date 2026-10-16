//! The Sightline icon, built into the program: the tray uses it directly, and an
//! installation writes it next to the program for the Start menu and the apps list.

/// An .ico file whose images are PNGs (16, 24, 32, 48 and 256 pixels).
pub const ICO: &[u8] = include_bytes!("../ui/icon.ico");

const HEADER_BYTES: usize = 6;
const ENTRY_BYTES: usize = 16;
/// An entry's width byte is 0 for 256 pixels.
const ZERO_WIDTH: u32 = 256;

/// The image of exactly `size` pixels, else the smallest larger one, else the largest.
pub fn image(ico: &[u8], size: u32) -> Option<&[u8]> {
    let count = usize::from(u16::from_le_bytes(
        ico.get(4..HEADER_BYTES)?.try_into().ok()?,
    ));
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let start = HEADER_BYTES + index * ENTRY_BYTES;
        let entry = ico.get(start..start + ENTRY_BYTES)?;
        let width = match u32::from(entry[0]) {
            0 => ZERO_WIDTH,
            width => width,
        };
        let length = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        entries.push((width, ico.get(offset..offset.checked_add(length)?)?));
    }
    let larger = entries
        .iter()
        .filter(|(width, _)| *width >= size)
        .min_by_key(|(width, _)| *width);
    larger
        .or_else(|| entries.iter().max_by_key(|(width, _)| *width))
        .map(|(_, image)| *image)
}

/// A Windows icon handle of about `size` pixels, for the tray. Owned by the caller.
pub fn handle(size: u32) -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, LR_DEFAULTCOLOR};
    /// The resource format version `CreateIconFromResourceEx` expects.
    const ICON_FORMAT_VERSION: u32 = 0x0003_0000;

    let image = image(ICO, size)?;
    let side = i32::try_from(size).ok()?;
    // SAFETY: `image` is a complete PNG icon image that outlives the call.
    let icon = unsafe {
        CreateIconFromResourceEx(
            image.as_ptr(),
            u32::try_from(image.len()).ok()?,
            1,
            ICON_FORMAT_VERSION,
            side,
            side,
            LR_DEFAULTCOLOR,
        )
    };
    (!icon.is_null()).then_some(icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_SIGNATURE: &[u8] = b"\x89PNG";

    #[test]
    fn an_exact_size_is_found() {
        let image = image(ICO, 32).expect("32 pixel image");

        assert!(image.starts_with(PNG_SIGNATURE));
        assert_eq!(png_width(image), 32);
    }

    #[test]
    fn a_missing_size_takes_the_next_larger_one() {
        assert_eq!(png_width(image(ICO, 20).expect("image")), 24);
    }

    #[test]
    fn a_size_above_every_image_takes_the_largest() {
        assert_eq!(png_width(image(ICO, 512).expect("image")), 256);
    }

    #[test]
    fn a_damaged_icon_has_no_image() {
        assert_eq!(image(&ICO[..10], 32), None);
        assert_eq!(image(b"not an icon", 32), None);
    }

    fn png_width(png: &[u8]) -> u32 {
        u32::from_be_bytes(png[16..20].try_into().expect("IHDR width"))
    }
}
