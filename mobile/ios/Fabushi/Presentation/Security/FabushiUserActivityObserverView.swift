import SwiftUI
#if canImport(UIKit)
import UIKit
import UIKit.UIGestureRecognizerSubclass

/// Passive, non-interfering `UIWindow` gesture recognizer that observes touches and hardware
/// key presses to keep the passcode idle timer accurate without ever claiming or blocking gestures.
internal final class FabushiActivityObservationGestureRecognizer: UIGestureRecognizer {
    var onActivity: @MainActor () -> Void

    init(onActivity: @escaping @MainActor () -> Void) {
        self.onActivity = onActivity
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
        requiresExclusiveTouchType = false
    }

    override func canPrevent(_ preventedGestureRecognizer: UIGestureRecognizer) -> Bool {
        false
    }

    override func canBePrevented(by preventingGestureRecognizer: UIGestureRecognizer) -> Bool {
        false
    }

    override func shouldRequireFailure(of otherGestureRecognizer: UIGestureRecognizer) -> Bool {
        false
    }

    override func shouldBeRequiredToFail(by otherGestureRecognizer: UIGestureRecognizer) -> Bool {
        false
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        onActivity()
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesMoved(touches, with: event)
        onActivity()
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesEnded(touches, with: event)
        onActivity()
        state = .failed
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesCancelled(touches, with: event)
        state = .failed
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent) {
        super.pressesBegan(presses, with: event)
        onActivity()
    }
}

/// Zero-interference SwiftUI bridge that attaches a passive activity observer to the hosting
/// `UIWindow` plus `UITextField`/`UITextView` typing notifications (covering software keyboard taps
/// routed through `UIRemoteKeyboardWindow`) so active usage resets the passcode idle auto-lock timer.
@MainActor
public struct FabushiUserActivityObserverView: UIViewRepresentable {
    public let onActivity: @MainActor () -> Void

    public init(onActivity: @escaping @MainActor () -> Void) {
        self.onActivity = onActivity
    }

    public final class AnchorView: UIView {
        var onWindowChange: (@MainActor (UIWindow?) -> Void)?

        public override init(frame: CGRect) {
            super.init(frame: frame)
            isUserInteractionEnabled = false
            backgroundColor = .clear
            isOpaque = false
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("init(coder:) has not been implemented")
        }

        public override func didMoveToWindow() {
            super.didMoveToWindow()
            onWindowChange?(window)
        }

        public override func willMove(toWindow newWindow: UIWindow?) {
            super.willMove(toWindow: newWindow)
            if newWindow == nil {
                onWindowChange?(nil)
            }
        }
    }

    @MainActor
    public final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        public var onActivity: @MainActor () -> Void
        private weak var attachedWindow: UIWindow?
        private var recognizer: FabushiActivityObservationGestureRecognizer?
        private var notificationTokens: [NSObjectProtocol] = []

        public init(onActivity: @escaping @MainActor () -> Void) {
            self.onActivity = onActivity
            super.init()
        }

        public func attach(to window: UIWindow?) {
            if attachedWindow === window {
                return
            }
            if let previous = attachedWindow, let existingRecognizer = recognizer {
                previous.removeGestureRecognizer(existingRecognizer)
            }
            attachedWindow = window

            guard let window else {
                stopTextNotifications()
                return
            }

            let activeRecognizer: FabushiActivityObservationGestureRecognizer
            if let existing = recognizer {
                activeRecognizer = existing
            } else {
                let created = FabushiActivityObservationGestureRecognizer { [weak self] in
                    self?.handleActivity()
                }
                created.delegate = self
                recognizer = created
                activeRecognizer = created
            }
            window.addGestureRecognizer(activeRecognizer)
            startTextNotificationsIfNeeded()
        }

        public func detach() {
            if let previous = attachedWindow, let existingRecognizer = recognizer {
                previous.removeGestureRecognizer(existingRecognizer)
            }
            attachedWindow = nil
            stopTextNotifications()
        }

        private func handleActivity() {
            onActivity()
        }

        private func startTextNotificationsIfNeeded() {
            guard notificationTokens.isEmpty else { return }
            let center = NotificationCenter.default
            let names: [Notification.Name] = [
                UITextField.textDidChangeNotification,
                UITextView.textDidChangeNotification,
            ]
            for name in names {
                let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                    Task { @MainActor [weak self] in
                        self?.handleActivity()
                    }
                }
                notificationTokens.append(token)
            }
        }

        private func stopTextNotifications() {
            guard !notificationTokens.isEmpty else { return }
            let center = NotificationCenter.default
            for token in notificationTokens {
                center.removeObserver(token)
            }
            notificationTokens.removeAll()
        }

        public func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            true
        }

        public func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldReceive touch: UITouch
        ) -> Bool {
            true
        }
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(onActivity: onActivity)
    }

    public func makeUIView(context: Context) -> AnchorView {
        let view = AnchorView(frame: .zero)
        let coordinator = context.coordinator
        view.onWindowChange = { window in
            coordinator.attach(to: window)
        }
        return view
    }

    public func updateUIView(_ uiView: AnchorView, context: Context) {
        context.coordinator.onActivity = onActivity
        context.coordinator.attach(to: uiView.window)
    }

    public static func dismantleUIView(_ uiView: AnchorView, coordinator: Coordinator) {
        uiView.onWindowChange = nil
        coordinator.detach()
    }
}
#else
@MainActor
public struct FabushiUserActivityObserverView: View {
    public let onActivity: @MainActor () -> Void

    public init(onActivity: @escaping @MainActor () -> Void) {
        self.onActivity = onActivity
    }

    public var body: some View {
        EmptyView()
    }
}
#endif
