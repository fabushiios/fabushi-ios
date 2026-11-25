import CoreGraphics
import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif
#if canImport(PencilKit)
import PencilKit
#endif

// MARK: - Pure Image Editing Engine (Crop, Rotate & Annotation Flattening)

public enum FabushiPhotoCropAspect: String, CaseIterable, Sendable, Equatable, Identifiable {
    case original
    case square
    case fourThree
    case sixteenNine

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .original: return "1:1+"
        case .square: return "1:1"
        case .fourThree: return "4:3"
        case .sixteenNine: return "16:9"
        }
    }

    public var ratio: CGFloat? {
        switch self {
        case .original: return nil
        case .square: return 1.0
        case .fourThree: return 4.0 / 3.0
        case .sixteenNine: return 16.0 / 9.0
        }
    }
}

public struct FabushiAnnotationStroke: Sendable, Equatable {
    public let points: [CGPoint]
    public let colorHex: UInt32
    public let lineWidth: CGFloat

    public init(points: [CGPoint], colorHex: UInt32 = 0xFF3B30, lineWidth: CGFloat = 4) {
        self.points = points
        self.colorHex = colorHex
        self.lineWidth = lineWidth
    }
}

#if canImport(UIKit)
public enum FabushiPhotoEditorEngine: Sendable {
    /// Applies deterministic 90° clockwise rotations, centered aspect-ratio crop, and vector markup
    /// flattening, returning a real rendered `UIImage` and JPEG `Data`.
    public static func renderEditedImage(
        source: UIImage,
        quarterTurnsClockwise: Int,
        cropAspect: FabushiPhotoCropAspect,
        strokes: [FabushiAnnotationStroke] = [],
        pencilDrawingImage: UIImage? = nil,
        jpegCompressionQuality: CGFloat = 0.88
    ) -> (image: UIImage, jpegData: Data)? {
        let normalizedTurns = ((quarterTurnsClockwise % 4) + 4) % 4
        let rotated = rotateImage(source, quarterTurnsClockwise: normalizedTurns)
        let cropped = cropImage(rotated, aspect: cropAspect)

        let canvasSize = cropped.size
        guard canvasSize.width > 0, canvasSize.height > 0 else { return nil }

        let format = UIGraphicsImageRendererFormat()
        format.scale = max(1.0, cropped.scale)
        format.opaque = true

        let renderer = UIGraphicsImageRenderer(size: canvasSize, format: format)
        let finalImage = renderer.image { ctx in
            cropped.draw(in: CGRect(origin: .zero, size: canvasSize))

            if let pencilDrawingImage {
                pencilDrawingImage.draw(in: CGRect(origin: .zero, size: canvasSize))
            }

            let cgContext = ctx.cgContext
            cgContext.setLineCap(.round)
            cgContext.setLineJoin(.round)
            for stroke in strokes where stroke.points.count >= 2 {
                let r = CGFloat((stroke.colorHex >> 16) & 0xFF) / 255.0
                let g = CGFloat((stroke.colorHex >> 8) & 0xFF) / 255.0
                let b = CGFloat(stroke.colorHex & 0xFF) / 255.0
                cgContext.setStrokeColor(red: r, green: g, blue: b, alpha: 1.0)
                cgContext.setLineWidth(stroke.lineWidth)
                cgContext.beginPath()
                cgContext.move(to: stroke.points[0])
                for pt in stroke.points.dropFirst() {
                    cgContext.addLine(to: pt)
                }
                cgContext.strokePath()
            }
        }

        guard let jpegData = finalImage.jpegData(compressionQuality: jpegCompressionQuality) else {
            return nil
        }
        return (finalImage, jpegData)
    }

    private static func rotateImage(_ image: UIImage, quarterTurnsClockwise: Int) -> UIImage {
        guard quarterTurnsClockwise != 0 else { return image }
        let angle = CGFloat(quarterTurnsClockwise) * (.pi / 2.0)
        let originalSize = image.size
        let targetSize = (quarterTurnsClockwise % 2 == 1)
            ? CGSize(width: originalSize.height, height: originalSize.width)
            : originalSize

        let format = UIGraphicsImageRendererFormat()
        format.scale = max(1.0, image.scale)
        let renderer = UIGraphicsImageRenderer(size: targetSize, format: format)
        return renderer.image { ctx in
            ctx.cgContext.translateBy(x: targetSize.width / 2.0, y: targetSize.height / 2.0)
            ctx.cgContext.rotate(by: angle)
            image.draw(
                in: CGRect(
                    x: -originalSize.width / 2.0,
                    y: -originalSize.height / 2.0,
                    width: originalSize.width,
                    height: originalSize.height
                )
            )
        }
    }

    private static func cropImage(_ image: UIImage, aspect: FabushiPhotoCropAspect) -> UIImage {
        guard let targetRatio = aspect.ratio else { return image }
        let width = image.size.width
        let height = image.size.height
        guard width > 0, height > 0 else { return image }
        let currentRatio = width / height
        let cropRect: CGRect
        if currentRatio > targetRatio {
            let newWidth = height * targetRatio
            cropRect = CGRect(x: (width - newWidth) / 2.0, y: 0, width: newWidth, height: height)
        } else {
            let newHeight = width / targetRatio
            cropRect = CGRect(x: 0, y: (height - newHeight) / 2.0, width: width, height: newHeight)
        }

        let format = UIGraphicsImageRendererFormat()
        format.scale = max(1.0, image.scale)
        let renderer = UIGraphicsImageRenderer(size: cropRect.size, format: format)
        return renderer.image { _ in
            image.draw(at: CGPoint(x: -cropRect.origin.x, y: -cropRect.origin.y))
        }
    }
}

#if canImport(PencilKit)
@MainActor
private struct FabushiPencilMarkupCanvasRepresentable: UIViewRepresentable {
    @Binding var drawing: PKDrawing
    let inkColor: UIColor

    func makeUIView(context: Context) -> PKCanvasView {
        let canvas = PKCanvasView()
        canvas.backgroundColor = .clear
        canvas.isOpaque = false
        canvas.drawingPolicy = .anyInput
        canvas.tool = PKInkingTool(.pen, color: inkColor, width: 5)
        canvas.drawing = drawing
        canvas.delegate = context.coordinator
        return canvas
    }

    func updateUIView(_ uiView: PKCanvasView, context: Context) {
        uiView.tool = PKInkingTool(.pen, color: inkColor, width: 5)
        if uiView.drawing != drawing {
            uiView.drawing = drawing
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(drawing: $drawing)
    }

    @MainActor
    final class Coordinator: NSObject, PKCanvasViewDelegate {
        private var drawing: Binding<PKDrawing>

        init(drawing: Binding<PKDrawing>) {
            self.drawing = drawing
        }

        func canvasViewDrawingDidChange(_ canvasView: PKCanvasView) {
            drawing.wrappedValue = canvasView.drawing
        }
    }
}
#endif

// MARK: - Native Apple HIG Photo Editor View (`#20`)

@MainActor
public struct FabushiPhotoEditorView: View {
    public let sourceImage: UIImage
    public let onCancel: () -> Void
    public let onComplete: (UIImage, Data) -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var quarterTurns: Int = 0
    @State private var selectedAspect: FabushiPhotoCropAspect = .original
    @State private var selectedInkHex: UInt32 = 0xFF3B30
    #if canImport(PencilKit)
    @State private var drawing = PKDrawing()
    #endif

    private static let inkPalette: [UInt32] = [0xFF3B30, 0xFFCC00, 0x34C759, 0x007AFF, 0xFFFFFF]

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        sourceImage: UIImage,
        onCancel: @escaping () -> Void,
        onComplete: @escaping (UIImage, Data) -> Void
    ) {
        self.sourceImage = sourceImage
        self.onCancel = onCancel
        self.onComplete = onComplete
    }

    public var body: some View {
        let preview = FabushiPhotoEditorEngine.renderEditedImage(
            source: sourceImage,
            quarterTurnsClockwise: quarterTurns,
            cropAspect: selectedAspect
        )?.image ?? sourceImage

        NavigationStack {
            VStack(spacing: 12) {
                GeometryReader { proxy in
                    ZStack {
                        Image(uiImage: preview)
                            .resizable()
                            .scaledToFit()
                            .frame(maxWidth: proxy.size.width, maxHeight: proxy.size.height)

                        #if canImport(PencilKit)
                        FabushiPencilMarkupCanvasRepresentable(
                            drawing: $drawing,
                            inkColor: uiColor(fromHex: selectedInkHex)
                        )
                        .frame(maxWidth: proxy.size.width, maxHeight: proxy.size.height)
                        #endif
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Color.black)
                }

                VStack(spacing: 12) {
                    HStack(spacing: 12) {
                        Button {
                            quarterTurns = (quarterTurns + 1) % 4
                        } label: {
                            Label(theme.messagingString(.photoEditorRotateAction), systemImage: "rotate.right")
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textPrimary)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 8)
                                .background(theme.surfaceInput, in: Capsule())
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)

                        Picker(theme.messagingString(.photoEditorCropAction), selection: $selectedAspect) {
                            ForEach(FabushiPhotoCropAspect.allCases) { aspect in
                                Text(aspect.label).tag(aspect)
                            }
                        }
                        .pickerStyle(.segmented)

                        #if canImport(PencilKit)
                        Button {
                            drawing = PKDrawing()
                        } label: {
                            Label(theme.messagingString(.photoEditorClearMarkupAction), systemImage: "eraser")
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textSecondary)
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        #endif
                    }

                    HStack(spacing: 14) {
                        ForEach(Self.inkPalette, id: \.self) { hex in
                            Button {
                                selectedInkHex = hex
                            } label: {
                                Circle()
                                    .fill(Color(fabushiHex: hex))
                                    .frame(width: 26, height: 26)
                                    .overlay(
                                        Circle()
                                            .strokeBorder(
                                                selectedInkHex == hex ? theme.accentColor : theme.hairlineBorder,
                                                lineWidth: selectedInkHex == hex ? 3 : 1
                                            )
                                    )
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(FabushiColorHex.format(hex))
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 12)
            }
            .background(theme.surfacePrimary)
            .navigationTitle(theme.messagingString(.photoEditorTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel"), action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.messagingString(.photoEditorApplyAction)) {
                        #if canImport(PencilKit)
                        let drawingImg: UIImage? = drawing.strokes.isEmpty
                            ? nil
                            : drawing.image(from: CGRect(origin: .zero, size: preview.size), scale: max(1.0, preview.scale))
                        #else
                        let drawingImg: UIImage? = nil
                        #endif
                        if let rendered = FabushiPhotoEditorEngine.renderEditedImage(
                            source: sourceImage,
                            quarterTurnsClockwise: quarterTurns,
                            cropAspect: selectedAspect,
                            pencilDrawingImage: drawingImg
                        ) {
                            onComplete(rendered.image, rendered.jpegData)
                        } else if let fallback = sourceImage.jpegData(compressionQuality: 0.88) {
                            onComplete(sourceImage, fallback)
                        }
                    }
                }
            }
        }
    }

    private func uiColor(fromHex hex: UInt32) -> UIColor {
        UIColor(
            red: CGFloat((hex >> 16) & 0xFF) / 255.0,
            green: CGFloat((hex >> 8) & 0xFF) / 255.0,
            blue: CGFloat(hex & 0xFF) / 255.0,
            alpha: 1.0
        )
    }
}
#endif
