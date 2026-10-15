//! What ⌘V means in the composer when the clipboard holds more than text:
//! files copied in Finder, or an image (a screenshot, "Copy Image" in a
//! browser). gpui's own clipboard reads text first and never sees file URLs,
//! so this asks NSPasteboard directly.

use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypeFileURL,
    NSPasteboardTypeString, NSPasteboardTypeTIFF,
};
use objc2_foundation::{NSDictionary, NSString};
use std::path::PathBuf;

pub enum Pasted {
    Files(Vec<PathBuf>),
    /// Encoded image bytes and the extension they should be saved with.
    Image(Vec<u8>, &'static str),
}

/// Files or an image on the general pasteboard, or `None` when ⌘V should
/// paste text. Text wins over an image unless it is a single word (a URL or
/// a file name riding along with the picture), so copying a table out of
/// Numbers, which also carries a picture of it, still pastes the text.
pub fn read() -> Option<Pasted> {
    let pb = NSPasteboard::generalPasteboard();
    let files: Vec<PathBuf> = pb
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
                .filter_map(|url| file_url_to_path(&url.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if !files.is_empty() {
        return Some(Pasted::Files(files));
    }
    let text = pb
        .stringForType(unsafe { NSPasteboardTypeString })
        .map(|s| s.to_string());
    if text
        .as_deref()
        .is_some_and(|t| t.trim().contains(char::is_whitespace))
    {
        return None;
    }
    // Formats the model accepts as they are; anything else (TIFF from a
    // screenshot or Preview) becomes PNG.
    for (uti, ext) in [
        ("public.png", "png"),
        ("public.jpeg", "jpg"),
        ("com.compuserve.gif", "gif"),
        ("org.webmproject.webp", "webp"),
    ] {
        if let Some(data) = pb.dataForType(&NSString::from_str(uti)) {
            if data.length() > 0 {
                return Some(Pasted::Image(data.to_vec(), ext));
            }
        }
    }
    let tiff = pb.dataForType(unsafe { NSPasteboardTypeTIFF })?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(Pasted::Image(png.to_vec(), "png"))
}

/// `file:///Users/me/My%20Shot.png` → `/Users/me/My Shot.png`.
fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8(out).ok()?;
    // Finder hands folders over with a trailing slash.
    let s = if s.len() > 1 {
        s.trim_end_matches('/')
    } else {
        &s
    };
    Some(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_decode_to_paths() {
        assert_eq!(
            file_url_to_path("file:///Users/me/My%20Shot.png"),
            Some("/Users/me/My Shot.png".into())
        );
        assert_eq!(
            file_url_to_path("file:///tmp/dir/"),
            Some("/tmp/dir".into())
        );
        assert_eq!(
            file_url_to_path("file://localhost/tmp/a%C3%A9.txt"),
            Some("/tmp/aé.txt".into())
        );
        assert_eq!(
            file_url_to_path("file:///tmp/100%"),
            Some("/tmp/100%".into())
        );
        assert_eq!(file_url_to_path("https://example.com"), None);
    }
}
