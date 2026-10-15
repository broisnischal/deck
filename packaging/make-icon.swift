// Draws deck's app icon (1024×1024 PNG with alpha) with CoreGraphics:
// a near-black rounded square on Apple's icon grid, a faint top-lit edge,
// and a white prompt chevron with a cursor.
//
//     swift packaging/make-icon.swift packaging/icon-1024.png
import AppKit

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon-1024.png"
let size = 1024
let cs = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                    space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!

// Apple's grid: an 824pt body centred in 1024, corner radius ≈ 185.
let body = CGRect(x: 100, y: 100, width: 824, height: 824)
let shape = CGPath(roundedRect: body, cornerWidth: 185, cornerHeight: 185, transform: nil)

// Soft drop shadow under the body.
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 36, color: CGColor(gray: 0, alpha: 0.45))
ctx.addPath(shape)
ctx.setFillColor(CGColor(gray: 0.05, alpha: 1))
ctx.fillPath()
ctx.restoreGState()

// Body: top-lit vertical gradient.
ctx.saveGState()
ctx.addPath(shape)
ctx.clip()
let gradient = CGGradient(colorsSpace: cs, colors: [
    CGColor(gray: 0.17, alpha: 1), CGColor(gray: 0.03, alpha: 1),
] as CFArray, locations: [0, 1])!
ctx.drawLinearGradient(gradient, start: CGPoint(x: 0, y: 924), end: CGPoint(x: 0, y: 100), options: [])
ctx.restoreGState()

// Hairline edge, brighter at the top.
ctx.saveGState()
ctx.addPath(CGPath(roundedRect: body.insetBy(dx: 1.5, dy: 1.5), cornerWidth: 183, cornerHeight: 183, transform: nil))
ctx.setLineWidth(3)
ctx.replacePathWithStrokedPath()
ctx.clip()
let edge = CGGradient(colorsSpace: cs, colors: [
    CGColor(gray: 1, alpha: 0.28), CGColor(gray: 1, alpha: 0.04),
] as CFArray, locations: [0, 1])!
ctx.drawLinearGradient(edge, start: CGPoint(x: 0, y: 924), end: CGPoint(x: 0, y: 100), options: [])
ctx.restoreGState()

// The prompt: a chevron and a cursor bar (CG's y axis points up).
ctx.setStrokeColor(CGColor(gray: 1, alpha: 1))
ctx.setLineWidth(58)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.move(to: CGPoint(x: 322, y: 652))
ctx.addLine(to: CGPoint(x: 466, y: 512))
ctx.addLine(to: CGPoint(x: 322, y: 372))
ctx.strokePath()
ctx.setFillColor(CGColor(gray: 1, alpha: 0.92))
ctx.addPath(CGPath(roundedRect: CGRect(x: 520, y: 352, width: 196, height: 58), cornerWidth: 29, cornerHeight: 29, transform: nil))
ctx.fillPath()

let image = ctx.makeImage()!
let rep = NSBitmapImageRep(cgImage: image)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
print("wrote \(out)")
