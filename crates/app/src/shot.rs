//! A picture of deck's own window (`deck window shot`), so a change to the
//! interface can be checked by eye. An app may always capture its own
//! windows, so this needs no Screen Recording permission. Pages are native
//! views inside the window and come out in it too.

use gpui::Window;
use std::path::Path;

/// Write the window as a PNG at `out`. Returns its size in pixels.
pub fn capture(window: &Window, out: &Path) -> Result<(usize, usize), String> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{CGWindowImageOption, CGWindowListOption};
    use objc2_foundation::NSDictionary;

    let number = window_number(window).ok_or("deck's window is not on screen")?;
    // A null rect: the window's own bounds.
    let null = CGRect::new(
        CGPoint::new(f64::INFINITY, f64::INFINITY),
        CGSize::new(0., 0.),
    );
    // Deprecated for ScreenCaptureKit, which asks for permission even for
    // the caller's own windows; this one does not.
    #[allow(deprecated)]
    let image = objc2_core_graphics::CGWindowListCreateImage(
        null,
        CGWindowListOption::OptionIncludingWindow,
        number,
        CGWindowImageOption::BoundsIgnoreFraming | CGWindowImageOption::BestResolution,
    )
    .ok_or("the window could not be captured")?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    let size = (rep.pixelsWide() as usize, rep.pixelsHigh() as usize);
    // SAFETY: an empty properties dictionary is what the method expects.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or("the picture could not be made into a PNG")?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, png.to_vec()).map_err(|e| e.to_string())?;
    Ok(size)
}

fn window_number(window: &Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: gpui's own content view, alive for as long as the window is.
    let view: &objc2_app_kit::NSView = unsafe { h.ns_view.cast().as_ref() };
    let n = view.window()?.windowNumber();
    u32::try_from(n).ok().filter(|n| *n > 0)
}
