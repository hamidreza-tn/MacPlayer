import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

guard CommandLine.arguments.count == 2 else {
    fputs("usage: generate-icon.swift <iconset-directory>\n", stderr)
    exit(2)
}

let outputDirectory = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: outputDirectory, withIntermediateDirectories: true)

let iconSizes: [(name: String, pixels: Int)] = [
    ("icon_16x16", 16),
    ("icon_16x16@2x", 32),
    ("icon_32x32", 32),
    ("icon_32x32@2x", 64),
    ("icon_128x128", 128),
    ("icon_128x128@2x", 256),
    ("icon_256x256", 256),
    ("icon_256x256@2x", 512),
    ("icon_512x512", 512),
    ("icon_512x512@2x", 1024),
]

func color(_ red: CGFloat, _ green: CGFloat, _ blue: CGFloat, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(red: red, green: green, blue: blue, alpha: alpha)
}

func drawIcon(pixels: Int) -> CGImage? {
    let size = CGFloat(pixels)
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    guard let context = CGContext(
        data: nil,
        width: pixels,
        height: pixels,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        return nil
    }

    context.setAllowsAntialiasing(true)
    context.setShouldAntialias(true)
    context.interpolationQuality = .high
    context.translateBy(x: 0, y: size)
    context.scaleBy(x: 1, y: -1)

    let inset = size * 0.055
    let card = CGRect(x: inset, y: inset, width: size - inset * 2, height: size - inset * 2)
    let radius = size * 0.22
    let cardPath = CGPath(roundedRect: card, cornerWidth: radius, cornerHeight: radius, transform: nil)
    context.addPath(cardPath)
    context.clip()

    let gradientColors = [
        color(0.16, 0.16, 0.17),
        color(0.09, 0.08, 0.07),
        color(0.025, 0.027, 0.03),
    ] as CFArray
    let locations: [CGFloat] = [0, 0.52, 1]
    if let gradient = CGGradient(colorsSpace: colorSpace, colors: gradientColors, locations: locations) {
        context.drawLinearGradient(
            gradient,
            start: CGPoint(x: size * 0.08, y: size),
            end: CGPoint(x: size * 0.92, y: 0),
            options: []
        )
    }

    context.saveGState()
    context.setBlendMode(.screen)
    context.setStrokeColor(color(1, 0.82, 0.18, 0.10))
    context.setLineWidth(max(1, size * 0.012))
    context.move(to: CGPoint(x: size * 0.08, y: size * 0.88))
    context.addLine(to: CGPoint(x: size * 0.42, y: size * 0.08))
    context.strokePath()
    context.move(to: CGPoint(x: size * 0.38, y: size * 0.92))
    context.addLine(to: CGPoint(x: size * 0.72, y: size * 0.12))
    context.strokePath()
    context.restoreGState()

    let screen = CGRect(
        x: size * 0.15,
        y: size * 0.19,
        width: size * 0.70,
        height: size * 0.58
    )
    let screenRadius = size * 0.085
    let screenPath = CGPath(
        roundedRect: screen,
        cornerWidth: screenRadius,
        cornerHeight: screenRadius,
        transform: nil
    )
    context.setFillColor(color(0.025, 0.027, 0.03, 0.84))
    context.addPath(screenPath)
    context.fillPath()
    context.setStrokeColor(color(0.98, 0.78, 0.05, 0.92))
    context.setLineWidth(max(1, size * 0.022))
    context.addPath(screenPath)
    context.strokePath()

    let center = CGPoint(x: size * 0.5, y: size * 0.48)
    let buttonRadius = size * 0.205
    context.setFillColor(color(0.98, 0.78, 0.05, 1))
    context.fillEllipse(in: CGRect(
        x: center.x - buttonRadius,
        y: center.y - buttonRadius,
        width: buttonRadius * 2,
        height: buttonRadius * 2
    ))

    let triangle = CGMutablePath()
    triangle.move(to: CGPoint(x: center.x - size * 0.055, y: center.y - size * 0.095))
    triangle.addLine(to: CGPoint(x: center.x + size * 0.105, y: center.y))
    triangle.addLine(to: CGPoint(x: center.x - size * 0.055, y: center.y + size * 0.095))
    triangle.closeSubpath()
    context.setFillColor(color(0.035, 0.034, 0.03))
    context.addPath(triangle)
    context.fillPath()

    context.setStrokeColor(color(1, 0.86, 0.28, 0.74))
    context.setLineWidth(max(1, size * 0.012))
    context.setLineCap(.round)
    context.move(to: CGPoint(x: size * 0.24, y: size * 0.85))
    context.addLine(to: CGPoint(x: size * 0.76, y: size * 0.85))
    context.strokePath()

    return context.makeImage()
}

for icon in iconSizes {
    guard let image = drawIcon(pixels: icon.pixels) else {
        fputs("could not render icon at \(icon.pixels)px\n", stderr)
        exit(1)
    }
    let destinationURL = outputDirectory.appendingPathComponent("\(icon.name).png")
    guard let destination = CGImageDestinationCreateWithURL(
        destinationURL as CFURL,
        UTType.png.identifier as CFString,
        1,
        nil
    ) else {
        fputs("could not create \(destinationURL.path)\n", stderr)
        exit(1)
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        fputs("could not write \(destinationURL.path)\n", stderr)
        exit(1)
    }
}
