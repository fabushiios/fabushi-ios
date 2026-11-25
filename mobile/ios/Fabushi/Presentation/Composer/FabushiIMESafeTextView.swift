import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Strict CJK IME Marked-Text Composition Fence (`#15`)

/// Pure testable guard preventing Return-to-send and `/` or `@` popup completion while CJK IME
/// composition (`UITextView.markedTextRange != nil`) is active.
public enum FabushiIMECompositionGuard: Sendable {
    /// Returns true ONLY when no IME marked text range is active, Shift is not held, and text is non-empty.
    public static func shouldAllowReturnToSend(
        hasMarkedText: Bool,
        isShiftPressed: Bool,
        text: String
    ) -> Bool {
        guard !hasMarkedText, !isShiftPressed else { return false }
        return !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Returns true ONLY when no IME marked text range is active.
    public static func shouldAllowCommandOrMentionCommit(hasMarkedText: Bool) -> Bool {
        !hasMarkedText
    }

    /// Detects whether composer text should open the `/` slash command popover (ignores filesystem paths).
    public static func activeSlashPrefix(text: String, hasMarkedText: Bool) -> String? {
        guard !hasMarkedText else { return nil }
        guard text.hasPrefix("/") else { return nil }
        // Ignore filesystem paths like `/Users/alice/main.swift` or `/usr/local/bin`
        let afterLeadingSlash = text.dropFirst()
        if afterLeadingSlash.contains("/") || afterLeadingSlash.contains(" ") || afterLeadingSlash.contains("\n") {
            return nil
        }
        return text
    }

    /// Detects an active trailing `@` token for mention / context badge lookup.
    public static func activeMentionQuery(text: String, hasMarkedText: Bool) -> String? {
        guard !hasMarkedText else { return nil }
        guard let lastSpace = text.lastIndex(where: { $0.isWhitespace }) else {
            return text.hasPrefix("@") ? String(text.dropFirst()) : nil
        }
        let token = text[text.index(after: lastSpace)...]
        return token.hasPrefix("@") ? String(token.dropFirst()) : nil
    }
}

#if canImport(UIKit)
// MARK: - Hardware-Keyboard & IME-Aware `UITextView` Subclass

@MainActor
public final class FabushiComposerPlatformTextView: UITextView {
    public var onSubmitReturn: (@MainActor () -> Void)?
    public var onMarkedTextStateChanged: (@MainActor (Bool) -> Void)?

    public var hasActiveMarkedText: Bool {
        markedTextRange != nil
    }

    public override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        for press in presses {
            guard let key = press.key else { continue }
            if key.keyCode == .keyboardReturnOrEnter {
                let shiftPressed = key.modifierFlags.contains(.shift)
                if FabushiIMECompositionGuard.shouldAllowReturnToSend(
                    hasMarkedText: hasActiveMarkedText,
                    isShiftPressed: shiftPressed,
                    text: text ?? ""
                ) {
                    onSubmitReturn?()
                    return
                }
            }
        }
        super.pressesBegan(presses, with: event)
    }

    public override func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        super.setMarkedText(markedText, selectedRange: selectedRange)
        onMarkedTextStateChanged?(markedTextRange != nil)
    }

    public override func unmarkText() {
        super.unmarkText()
        onMarkedTextStateChanged?(markedTextRange != nil)
    }
}

// MARK: - SwiftUI `UIViewRepresentable` IME-Safe Auto-Growing TextView

@MainActor
public struct FabushiIMESafeTextView: UIViewRepresentable {
    @Binding public var text: String
    @Binding public var isComposingMarkedText: Bool
    @Binding public var measuredHeight: CGFloat
    public let placeholder: String
    public let textColor: UIColor
    public let tintColor: UIColor
    public let minHeight: CGFloat
    public let maxHeight: CGFloat
    public let accessibilityID: String
    public let onSubmitReturn: () -> Void

    public init(
        text: Binding<String>,
        isComposingMarkedText: Binding<Bool>,
        measuredHeight: Binding<CGFloat>,
        placeholder: String,
        textColor: UIColor,
        tintColor: UIColor,
        minHeight: CGFloat = 36,
        maxHeight: CGFloat = 132,
        accessibilityID: String = FabushiAccessibilityIDs.Composer.textField,
        onSubmitReturn: @escaping () -> Void
    ) {
        self._text = text
        self._isComposingMarkedText = isComposingMarkedText
        self._measuredHeight = measuredHeight
        self.placeholder = placeholder
        self.textColor = textColor
        self.tintColor = tintColor
        self.minHeight = minHeight
        self.maxHeight = maxHeight
        self.accessibilityID = accessibilityID
        self.onSubmitReturn = onSubmitReturn
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    public func makeUIView(context: Context) -> FabushiComposerPlatformTextView {
        let textView = FabushiComposerPlatformTextView()
        textView.delegate = context.coordinator
        textView.backgroundColor = .clear
        textView.font = UIFont.preferredFont(forTextStyle: .body)
        textView.adjustsFontForContentSizeCategory = true
        textView.textColor = textColor
        textView.tintColor = tintColor
        textView.isScrollEnabled = false
        textView.textContainerInset = UIEdgeInsets(top: 8, left: 4, bottom: 8, right: 4)
        textView.textContainer.lineFragmentPadding = 2
        textView.returnKeyType = .default
        textView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        textView.accessibilityIdentifier = accessibilityID
        textView.accessibilityLabel = placeholder
        textView.text = text
        textView.onSubmitReturn = { [weak coordinator = context.coordinator] in
            coordinator?.parent.onSubmitReturn()
        }
        textView.onMarkedTextStateChanged = { [weak coordinator = context.coordinator] composing in
            coordinator?.updateMarkedTextState(composing)
        }
        return textView
    }

    public func updateUIView(_ uiView: FabushiComposerPlatformTextView, context: Context) {
        context.coordinator.parent = self
        uiView.textColor = textColor
        uiView.tintColor = tintColor
        uiView.accessibilityIdentifier = accessibilityID
        uiView.accessibilityLabel = placeholder

        // NEVER overwrite text while CJK IME markedTextRange is active!
        if uiView.markedTextRange == nil && uiView.text != text {
            uiView.text = text
        }
        context.coordinator.recalculateHeight(for: uiView)
    }

    @MainActor
    public final class Coordinator: NSObject, UITextViewDelegate {
        public var parent: FabushiIMESafeTextView

        public init(parent: FabushiIMESafeTextView) {
            self.parent = parent
        }

        public func textViewDidChange(_ textView: UITextView) {
            let composing = textView.markedTextRange != nil
            updateMarkedTextState(composing)
            parent.text = textView.text ?? ""
            recalculateHeight(for: textView)
        }

        public func textViewDidChangeSelection(_ textView: UITextView) {
            updateMarkedTextState(textView.markedTextRange != nil)
        }

        public func textView(
            _ textView: UITextView,
            shouldChangeTextIn range: NSRange,
            replacementText text: String
        ) -> Bool {
            if text == "\n",
               FabushiIMECompositionGuard.shouldAllowReturnToSend(
                   hasMarkedText: textView.markedTextRange != nil,
                   isShiftPressed: false,
                   text: textView.text ?? ""
               )
            {
                // On iOS software keyboards, Return inserts newline cleanly; hardware keyboard
                // Return submits via `pressesBegan` above while respecting `markedTextRange != nil`.
                return true
            }
            return true
        }

        func recalculateHeight(for uiView: UITextView) {
            let availableWidth = uiView.bounds.width > 0 ? uiView.bounds.width : 260
            let fitting = uiView.sizeThatFits(CGSize(width: availableWidth, height: .greatestFiniteMagnitude))
            let clamped = min(parent.maxHeight, max(parent.minHeight, ceil(fitting.height)))
            uiView.isScrollEnabled = fitting.height > parent.maxHeight
            if abs(parent.measuredHeight - clamped) > 0.5 {
                Task { @MainActor [weak self] in
                    self?.parent.measuredHeight = clamped
                }
            }
        }

        func updateMarkedTextState(_ composing: Bool) {
            if parent.isComposingMarkedText != composing {
                Task { @MainActor [weak self] in
                    self?.parent.isComposingMarkedText = composing
                }
            }
        }
    }
}
#endif
