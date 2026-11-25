import Foundation
import SwiftUI

// MARK: - Sealed `MessagingModuleProviding` Conformance Factory (`M3` Entrypoint for `M6`)

/// Production Milestone M3 module factory conforming to the sealed M2 `MessagingModuleProviding`
/// contract (`Presentation/Shell/FabushiFeatureModuleProtocols.swift`).
///
/// Ready for 1-line M6 registration in `FabushiSceneRoot.swift`:
/// `FabushiFeatureModuleRegistry.shared.register(messaging: FabushiMessagingModule())`
@MainActor
public final class FabushiMessagingModule: MessagingModuleProviding {
    public init() {}

    private func sharedStore(for coordinator: FabushiCoordinator) -> FabushiMessagingStore {
        FabushiMessagingStoreRegistry.shared.store(for: coordinator)
    }

    public func makeChatSidebarView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiChatListView(
                coordinator: coordinator,
                router: router,
                store: sharedStore(for: coordinator)
            )
        )
    }

    public func makeContactsView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiContactsView(
                coordinator: coordinator,
                router: router,
                store: sharedStore(for: coordinator)
            )
        )
    }

    public func makeConversationStageView(
        chatId: Int64?,
        messageId: Int64?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiConversationStageView(
                chatId: chatId,
                messageId: messageId,
                coordinator: coordinator,
                router: router,
                store: sharedStore(for: coordinator)
            )
        )
    }

    public func makeMultiSelectComposeView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiMultiSelectComposeView(
                coordinator: coordinator,
                router: router,
                store: sharedStore(for: coordinator)
            )
        )
    }

    public func makeContextInspectorView(
        target: InspectorTarget,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        AnyView(
            FabushiContextInspectorView(
                target: target,
                coordinator: coordinator,
                router: router,
                store: sharedStore(for: coordinator)
            )
        )
    }

    public func makeUniversalSearchView(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        let store = sharedStore(for: coordinator)
        return AnyView(
            FabushiUniversalSearchView(
                coordinator: coordinator,
                router: router,
                store: store,
                inChatId: store.overlayController.activeSearchChatId,
                onDismiss: { store.overlayController.dismissUniversalSearch() }
            )
        )
    }

    public func makeMediaViewerOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        let store = sharedStore(for: coordinator)
        return AnyView(
            FabushiMediaLightboxView(
                coordinator: coordinator,
                router: router,
                store: store,
                target: store.overlayController.presentedMediaTarget,
                onDismiss: { store.overlayController.dismissMediaLightbox() }
            )
        )
    }

    public func makeStoriesOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        let store = sharedStore(for: coordinator)
        return AnyView(
            FabushiStoriesView(
                coordinator: coordinator,
                router: router,
                store: store,
                onDismiss: { store.overlayController.dismissStories() }
            )
        )
    }

    public func makeInstantViewOverlay(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) -> AnyView {
        let store = sharedStore(for: coordinator)
        return AnyView(
            FabushiInstantViewReaderView(
                coordinator: coordinator,
                router: router,
                store: store,
                document: store.overlayController.presentedInstantView,
                onDismiss: { store.overlayController.dismissInstantView() }
            )
        )
    }
}
