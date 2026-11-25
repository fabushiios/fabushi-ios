import SwiftUI

/// `CreateManagedBotBox` modal implementing `D10-12` `originBoxId: UUID` close-race guard:
/// dismissing the modal mid-flight strictly suppresses route hijack and stale error display
/// while refreshing the catalog (if generation still matches) and emitting a non-chat toast.
@MainActor
public struct FabushiCreateManagedBotSheetView: View {
    public let originBoxId: UUID
    @Bindable public var viewModel: FabushiMiniAppHostViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var promptText: String = ""

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(originBoxId: UUID, viewModel: FabushiMiniAppHostViewModel) {
        self.originBoxId = originBoxId
        self.viewModel = viewModel
    }

    public var body: some View {
        NavigationStack {
            Form {
                Section(FabushiPlatformStrings.text(.miniAppCreateBotPromptLabel, locale: locale)) {
                    TextField(
                        FabushiPlatformStrings.text(.miniAppCreateBotPromptPlaceholder, locale: locale),
                        text: $promptText,
                        axis: .vertical
                    )
                    .lineLimit(3...6)
                    .fabushiFont(.body)
                }

                if let error = viewModel.createBotModalError,
                   viewModel.activeCreateBoxId == originBoxId
                {
                    Section {
                        Text(error)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.destructiveColor)
                    }
                }
            }
            .navigationTitle(FabushiPlatformStrings.text(.miniAppCreateBotTitle, locale: locale))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(FabushiPlatformStrings.text(.cancel, locale: locale)) {
                        viewModel.closeCreateManagedBotBox(boxId: originBoxId)
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(FabushiPlatformStrings.text(.miniAppCreateBotSubmit, locale: locale)) {
                        let capturedPrompt = promptText
                        let capturedBox = originBoxId
                        Task {
                            _ = await viewModel.submitCreateManagedBot(
                                prompt: capturedPrompt,
                                originBoxId: capturedBox,
                                locale: locale
                            )
                        }
                    }
                    .disabled(
                        promptText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            || viewModel.isCreatingManagedBot
                    )
                    .fabushiMinimumTouchTarget()
                }
            }
            .onDisappear {
                if viewModel.activeCreateBoxId == originBoxId {
                    viewModel.closeCreateManagedBotBox(boxId: originBoxId)
                }
            }
        }
    }
}
