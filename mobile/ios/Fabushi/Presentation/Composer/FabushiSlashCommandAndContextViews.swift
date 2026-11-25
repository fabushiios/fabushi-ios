import Foundation
import SwiftUI

// MARK: - 50+ Hermes/Fabushi Slash Commands Grouping & Category Labels (`#16`, `D11-01`)

public enum FabushiSlashCommandCatalogHelper: Sendable {
    public static let orderedCategories: [String] = [
        "session",
        "model_and_provider",
        "tools_and_skills",
        "memory_and_persona",
        "workflow_and_automation",
        "safety_and_control",
        "inspection_and_export",
        "modes_and_ui",
    ]

    public static func categoryStringKey(for rawCategory: String) -> FabushiMessagingStrings.Key {
        switch rawCategory.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "session": return .slashCategorySession
        case "model_and_provider": return .slashCategoryModel
        case "tools_and_skills": return .slashCategoryTools
        case "memory_and_persona": return .slashCategoryMemory
        case "workflow_and_automation": return .slashCategoryWorkflow
        case "safety_and_control": return .slashCategorySafety
        case "inspection_and_export": return .slashCategoryInspection
        case "modes_and_ui": return .slashCategoryModes
        default: return .slashCategoryTools
        }
    }

    public static func filterCommands(
        _ commands: [FabushiSlashCommand],
        prefix: String,
        selectedCategory: String? = nil
    ) -> [FabushiSlashCommand] {
        let normalizedPrefix = prefix.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let stripped = normalizedPrefix.hasPrefix("/") ? String(normalizedPrefix.dropFirst()) : normalizedPrefix

        return commands.filter { cmd in
            if let selectedCategory, !selectedCategory.isEmpty,
               cmd.category.lowercased() != selectedCategory.lowercased() {
                return false
            }
            guard !stripped.isEmpty else { return true }
            if cmd.name.lowercased().contains(stripped) || cmd.slashName.lowercased().contains(stripped) {
                return true
            }
            if cmd.aliases.contains(where: { $0.lowercased().contains(stripped) }) {
                return true
            }
            return cmd.descriptionText.lowercased().contains(stripped)
        }
    }

    /// Formats active `@file`/`@dir`/`@diff`/`@git:`/`@url` context badges cleanly onto outgoing text.
    public static func composeMessageText(body: String, badges: [FabushiContextBadge]) -> String {
        let trimmedBody = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !badges.isEmpty else { return trimmedBody }
        let header = badges.map(\.formattedToken).joined(separator: " ")
        if trimmedBody.isEmpty {
            return header
        }
        return "\(header)\n\(trimmedBody)"
    }
}

// MARK: - Live Slash Command Autocomplete Popover (`#16`, `D11-01`)

@MainActor
public struct FabushiSlashCommandPopoverView: View {
    public let commands: [FabushiSlashCommand]
    public let prefix: String
    public let onSelectCommand: (FabushiSlashCommand) -> Void
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var selectedCategory: String? = nil

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var filtered: [FabushiSlashCommand] {
        FabushiSlashCommandCatalogHelper.filterCommands(
            commands,
            prefix: prefix,
            selectedCategory: selectedCategory
        )
    }

    public init(
        commands: [FabushiSlashCommand],
        prefix: String,
        onSelectCommand: @escaping (FabushiSlashCommand) -> Void,
        onDismiss: @escaping () -> Void
    ) {
        self.commands = commands
        self.prefix = prefix
        self.onSelectCommand = onSelectCommand
        self.onDismiss = onDismiss
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("\(theme.messagingString(.slashCommandsTitle)) (\(filtered.count))")
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textSecondary)
                Spacer()
                Button(action: onDismiss) {
                    Image(systemName: "xmark.circle.fill")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textMuted)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.localized("hig.common.close"))
            }
            .padding(.horizontal, 12)
            .padding(.top, 8)

            // 8-Category Filter Pill Bar
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    categoryChip(nil, title: theme.messagingString(.allFolder))
                    ForEach(FabushiSlashCommandCatalogHelper.orderedCategories, id: \.self) { category in
                        categoryChip(
                            category,
                            title: theme.messagingString(FabushiSlashCommandCatalogHelper.categoryStringKey(for: category))
                        )
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
            }

            Divider().overlay(theme.separatorHairline)

            if filtered.isEmpty {
                Text(theme.messagingString(.slashNoMatches))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .center)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(filtered.prefix(60).enumerated()), id: \.element.id) { index, cmd in
                            Button {
                                onSelectCommand(cmd)
                            } label: {
                                HStack(alignment: .top, spacing: 10) {
                                    VStack(alignment: .leading, spacing: 3) {
                                        HStack(spacing: 6) {
                                            Text(cmd.syntax.isEmpty ? cmd.slashName : cmd.syntax)
                                                .fabushiFont(.subheadline)
                                                .fontWeight(.semibold)
                                                .foregroundStyle(theme.accentColor)
                                            if !cmd.aliases.isEmpty {
                                                Text(cmd.aliases.map { "/\($0)" }.joined(separator: ", "))
                                                    .fabushiFont(.caption2)
                                                    .foregroundStyle(theme.textMuted)
                                            }
                                        }
                                        if !cmd.descriptionText.isEmpty {
                                            Text(cmd.descriptionText)
                                                .fabushiFont(.caption)
                                                .foregroundStyle(theme.textSecondary)
                                                .lineLimit(2)
                                        }
                                    }
                                    Spacer(minLength: 8)
                                    Text(
                                        theme.messagingString(
                                            FabushiSlashCommandCatalogHelper.categoryStringKey(for: cmd.category)
                                        )
                                    )
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.textMuted)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 3)
                                    .background(theme.surfaceInput, in: Capsule())
                                }
                                .padding(.horizontal, 12)
                                .padding(.vertical, 10)
                                .contentShape(Rectangle())
                                .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier(
                                index == 0
                                    ? FabushiAccessibilityIDs.Composer.slashCommandFirstItem
                                    : "fabushi.composer.slashCommand.\(index)"
                            )

                            if index < min(filtered.count, 60) - 1 {
                                Divider().overlay(theme.separatorHairline).padding(.leading, 12)
                            }
                        }
                    }
                }
                .frame(maxHeight: 240)
            }
        }
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius)
        .padding(.horizontal, 10)
        .padding(.bottom, 6)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.slashCommandPopover)
    }

    private func categoryChip(_ category: String?, title: String) -> some View {
        let isSelected = selectedCategory == category
        return Button {
            selectedCategory = category
        } label: {
            Text(title)
                .fabushiFont(.caption2)
                .fontWeight(isSelected ? .semibold : .regular)
                .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textSecondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(isSelected ? theme.accentColor : theme.surfaceInput, in: Capsule())
        }
        .buttonStyle(.plain)
    }
}

// MARK: - `@` Context Badge Strip & Picker Sheet (`#16`)

@MainActor
public struct FabushiContextBadgeStripView: View {
    public let badges: [FabushiContextBadge]
    public let onRemove: (String) -> Void
    public let onAddTapped: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        badges: [FabushiContextBadge],
        onRemove: @escaping (String) -> Void,
        onAddTapped: @escaping () -> Void
    ) {
        self.badges = badges
        self.onRemove = onRemove
        self.onAddTapped = onAddTapped
    }

    public var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(badges) { badge in
                    HStack(spacing: 6) {
                        Image(systemName: iconName(for: badge.kind))
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.accentColor)
                        Text(badge.formattedToken)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textPrimary)
                            .lineLimit(1)
                        Button {
                            onRemove(badge.id)
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textMuted)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(theme.localized("hig.common.close"))
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.surfaceInput, in: Capsule())
                }

                Button(action: onAddTapped) {
                    Label(theme.messagingString(.addContextBadgeAction), systemImage: "plus")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.accentColor)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(theme.accentColor.opacity(0.14), in: Capsule())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.contextBadgeStrip)
    }

    private func iconName(for kind: FabushiContextBadgeKind) -> String {
        switch kind {
        case .file: return "doc.text"
        case .directory: return "folder"
        case .diff: return "plus.forwardslash.minus"
        case .git: return "arrow.triangle.branch"
        case .url: return "link"
        }
    }
}

@MainActor
public struct FabushiContextBadgePickerSheetView: View {
    public let onAddBadge: (FabushiContextBadge) -> Void
    public let onCancel: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var selectedKind: FabushiContextBadgeKind = .file
    @State private var valueText: String = ""

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        onAddBadge: @escaping (FabushiContextBadge) -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.onAddBadge = onAddBadge
        self.onCancel = onCancel
    }

    public var body: some View {
        NavigationStack {
            Form {
                Section(header: Text(theme.messagingString(.contextBadgesTitle))) {
                    Picker(theme.messagingString(.contextBadgesTitle), selection: $selectedKind) {
                        ForEach(FabushiContextBadgeKind.allCases, id: \.self) { kind in
                            Text(kind.rawValue).tag(kind)
                        }
                    }
                    .pickerStyle(.segmented)

                    TextField(theme.messagingString(.contextBadgeValuePlaceholder), text: $valueText)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled(true)
                }
            }
            .navigationTitle(theme.messagingString(.addContextBadgeAction))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel"), action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.localized("hig.common.apply")) {
                        let trimmed = valueText.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard !trimmed.isEmpty else { return }
                        onAddBadge(
                            FabushiContextBadge(
                                kind: selectedKind,
                                label: trimmed,
                                value: trimmed
                            )
                        )
                    }
                    .disabled(valueText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}
