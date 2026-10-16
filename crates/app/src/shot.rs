//! A picture of deck's own window (`deck window shot`), so a change to the
//! interface can be checked by eye. An app may always capture its own
//! windows, so this needs no Screen Recording permission. Pages are native
//! views inside the window and come out in it too.

use gpui::{Bounds, Pixels, Window};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use std::path::Path;

/// Write the window as a PNG at `out`. Returns its size in pixels.
pub fn capture(window: &Window, out: &Path) -> Result<(usize, usize), String> {
    let image = window_image(window)?;
    write_png(&image, out)
}

/// Write just `area` of the window (gpui's coordinates: logical pixels from
/// the top left of its view) as a PNG at `out`: one pane, say.
pub fn capture_area(
    window: &Window,
    area: Bounds<Pixels>,
    out: &Path,
) -> Result<(usize, usize), String> {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    let view = content_view(window).ok_or("deck's window is not on screen")?;
    let win = view.window().ok_or("deck's window is not on screen")?;
    let image = window_image(window)?;
    // The picture is of the window's frame; gpui's view may start below a
    // title bar, and its pixels are the backing scale's.
    let frame = win.frame();
    let inner = view.convertRect_toView(view.bounds(), None);
    let left = inner.origin.x;
    let top = frame.size.height - (inner.origin.y + inner.size.height);
    let scale = CGImage::width(Some(&image)) as f64 / frame.size.width.max(1.);
    let (x, y) = (f32::from(area.origin.x) as f64, f32::from(area.origin.y) as f64);
    let (w, h) = (f32::from(area.size.width) as f64, f32::from(area.size.height) as f64);
    if w < 1. || h < 1. {
        return Err("the pane has no size on screen".into());
    }
    let rect = CGRect::new(
        CGPoint::new(((left + x) * scale).round(), ((top + y) * scale).round()),
        CGSize::new((w * scale).round(), (h * scale).round()),
    );
    let part = CGImage::with_image_in_rect(Some(&image), rect)
        .ok_or("the pane is outside the window's picture")?;
    write_png(&part, out)
}

fn window_image(window: &Window) -> Result<CFRetained<CGImage>, String> {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{CGWindowImageOption, CGWindowListOption};

    let number = window_number(window).ok_or("deck's window is not on screen")?;
    // A null rect: the window's own bounds.
    let null = CGRect::new(
        CGPoint::new(f64::INFINITY, f64::INFINITY),
        CGSize::new(0., 0.),
    );
    // Deprecated for ScreenCaptureKit, which asks for permission even for
    // the caller's own windows; this one does not.
    #[allow(deprecated)]
    objc2_core_graphics::CGWindowListCreateImage(
        null,
        CGWindowListOption::OptionIncludingWindow,
        number,
        CGWindowImageOption::BoundsIgnoreFraming | CGWindowImageOption::BestResolution,
    )
    .ok_or_else(|| "the window could not be captured".into())
}

/// Write `image` as a PNG at `out`. Returns its size in pixels.
fn write_png(image: &CGImage, out: &Path) -> Result<(usize, usize), String> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::NSDictionary;

    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
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

/// gpui's view: the one in the window's raw handle.
fn content_view(window: &Window) -> Option<&objc2_app_kit::NSView> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: gpui's own content view, alive for as long as the window is.
    Some(unsafe { h.ns_view.cast().as_ref() })
}

fn window_number(window: &Window) -> Option<u32> {
    let n = content_view(window)?.window()?.windowNumber();
    u32::try_from(n).ok().filter(|n| *n > 0)
}
