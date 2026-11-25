@preconcurrency import AVFoundation
import Foundation
import os
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

/// Official OAuth 2.1 Remote MCP Server Grants, QR-Poll Approval, Redacted Audit Log & Kill-Switch Console.
///
/// Strictly ZERO nested root `NavigationStack` and ZERO conflicting `.topBarLeading` toolbar items so it
/// renders cleanly both pushed onto Settings' `NavigationStack` AND presented inside Agent Studio's `.sheet`.
/// Mounts `FabushiAccessibilityIDs.Studio.mcpAuditLogList` and `FabushiAccessibilityIDs.Studio.mcpApproveButton`.
@MainActor
public struct FabushiOfficialMcpConsoleView: View {
    @State private var viewModel: FabushiMcpViewModel
    @State private var isQrScannerPresented: Bool = false
    @State private var isKillSwitchConfirmPresented: Bool = false
    @State private var isRegisterServerSheetPresented: Bool = false
    @State private var isOfficialCatalogExpanded: Bool = false

    // Register remote server sheet fields
    @State private var draftServerName: String = ""
    @State private var draftServerTransport: FabushiMcpTransportKind = .streamableHttp
    @State private var draftServerUrl: String = ""

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self._viewModel = State(
            initialValue: FabushiMcpViewModel(
                coordinator: coordinator,
                router: router,
                cleanupHub: cleanupHub
            )
        )
    }

    public var body: some View {
        let locale = theme.locale
        let isRegularWidth = horizontalSizeClass == .regular

        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let errorText = viewModel.lastErrorMessage, !errorText.isEmpty {
                    errorBanner(text: errorText, locale: locale)
                }

                stdioSandboxNotice(locale: locale)
                oauthGrantsAndKillSwitchSection(locale: locale)
                qrPollApprovalSection(locale: locale)
                redactedAuditLogSection(locale: locale)
                remoteServersSection(locale: locale)
                officialCatalogAndLocalRegistrySection(locale: locale, isRegularWidth: isRegularWidth)
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiAgentStudioStrings.text("m4.mcp.title", locale: locale))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { @MainActor in
                        await viewModel.loadAll()
                    }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .fabushiMinimumTouchTarget()
                .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.refresh", locale: locale))
                .accessibilityIdentifier("fabushi.internal.mcp.refreshButton")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("fabushi.internal.mcp.root")
        .task(id: viewModel.coordinator.accountScopeGeneration) {
            await viewModel.loadAll()
        }
        .onChange(of: viewModel.coordinator.accountScopeGeneration) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .onChange(of: viewModel.coordinator.activeAccountId) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .confirmationDialog(
            FabushiAgentStudioStrings.text("m4.mcp.killSwitchButton", locale: locale),
            isPresented: $isKillSwitchConfirmPresented,
            titleVisibility: .visible
        ) {
            Button(
                FabushiAgentStudioStrings.text("m4.mcp.killSwitchButton", locale: locale),
                role: .destructive
            ) {
                Task { @MainActor in
                    _ = await viewModel.triggerEmergencyKillSwitch(locale: locale)
                }
            }
            Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale), role: .cancel) {}
        } message: {
            Text(FabushiAgentStudioStrings.text("m4.mcp.killSwitchConfirm", locale: locale))
        }
        .sheet(isPresented: $isQrScannerPresented) {
            FabushiMcpQrScannerSheet(
                onScannedPayload: { payload in
                    isQrScannerPresented = false
                    viewModel.pairTokenInput = payload
                    Task { @MainActor in
                        _ = await viewModel.approveQrPairLogin(rawPayload: payload, locale: locale)
                    }
                },
                onCancel: {
                    isQrScannerPresented = false
                }
            )
            .environment(theme)
        }
        .sheet(isPresented: $isRegisterServerSheetPresented) {
            registerServerSheet(locale: locale)
                .environment(theme)
        }
    }

    // MARK: - Banners

    @ViewBuilder
    private func errorBanner(text: String, locale: FabushiLocale) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(theme.warningColor)
                .accessibilityHidden(true)
            Text(text)
                .fabushiFont(.caption)
                .foregroundStyle(theme.textPrimary)
            Spacer(minLength: 4)
            Button {
                viewModel.lastErrorMessage = nil
            } label: {
                Image(systemName: "xmark")
                    .foregroundStyle(theme.textSecondary)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(FabushiAgentStudioStrings.text("m4.error.dismissHint", locale: locale))
        }
        .padding(12)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    @ViewBuilder
    private func stdioSandboxNotice(locale: FabushiLocale) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "shield.lefthalf.filled")
                .foregroundStyle(theme.accentColor)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(FabushiAgentStudioStrings.text("m4.common.iosSandboxNotice", locale: locale))
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
                Text(viewModel.lastSandboxNotice ?? FabushiAgentStudioStrings.text("m4.mcp.stdioBlockedNotice", locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
    }

    // MARK: - 1. OAuth 2.1 PKCE Active Grants & Emergency Kill-Switch

    @ViewBuilder
    private func oauthGrantsAndKillSwitchSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.grantsSection", locale: locale))
                    Spacer(minLength: 8)
                    killSwitchButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.grantsSection", locale: locale))
                    killSwitchButton(locale: locale)
                }
            }

            if viewModel.grants.isEmpty {
                Text(FabushiAgentStudioStrings.text("m4.mcp.grantsEmpty", locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            } else {
                ForEach(viewModel.grants) { grant in
                    VStack(alignment: .leading, spacing: 8) {
                        ViewThatFits(in: .horizontal) {
                            HStack(spacing: 8) {
                                grantHeader(grant: grant, locale: locale)
                                Spacer(minLength: 4)
                                revokeGrantButton(grant: grant, locale: locale)
                            }
                            VStack(alignment: .leading, spacing: 8) {
                                grantHeader(grant: grant, locale: locale)
                                revokeGrantButton(grant: grant, locale: locale)
                            }
                        }

                        if !grant.scopes.isEmpty {
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 6) {
                                    ForEach(grant.scopes, id: \.self) { scope in
                                        Text(scope)
                                            .fabushiFont(.monospacedCaption)
                                            .foregroundStyle(theme.textPrimary)
                                            .padding(.horizontal, 8)
                                            .padding(.vertical, 4)
                                            .background(theme.surfaceElevated, in: Capsule())
                                    }
                                }
                            }
                        }
                    }
                    .padding(12)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                }
            }
        }
    }

    @ViewBuilder
    private func grantHeader(grant: FabushiMcpOAuthGrantDTO, locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(grant.clientName)
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
            let deviceScopeText = grant.allowsAllDevices
                ? FabushiAgentStudioStrings.text("m4.mcp.allDevicesScope", locale: locale)
                : grant.allowedNodeIds.joined(separator: ", ")
            Text("\(grant.familyId) · \(deviceScopeText)")
                .fabushiFont(.monospacedCaption)
                .foregroundStyle(theme.textSecondary)
        }
    }

    @ViewBuilder
    private func revokeGrantButton(grant: FabushiMcpOAuthGrantDTO, locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.mcp.revokeGrantButton", locale: locale),
            systemImage: "xmark.shield.fill",
            accessibilityID: "fabushi.internal.mcp.revoke.\(grant.familyId)",
            isProminent: false
        ) {
            Task { @MainActor in
                _ = await viewModel.revokeGrant(familyId: grant.familyId)
            }
        }
    }

    @ViewBuilder
    private func killSwitchButton(locale: FabushiLocale) -> some View {
        Button(role: .destructive) {
            isKillSwitchConfirmPresented = true
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "bolt.slash.fill")
                    .accessibilityHidden(true)
                Text(FabushiAgentStudioStrings.text("m4.mcp.killSwitchButton", locale: locale))
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
            }
            .foregroundStyle(Color.white)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .fabushiMinimumTouchTarget()
            .background(theme.destructiveColor, in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("fabushi.internal.mcp.killSwitchButton")
    }

    // MARK: - 2. QR-Poll Login Approval (`FabushiAccessibilityIDs.Studio.mcpApproveButton`)

    @ViewBuilder
    private func qrPollApprovalSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.qrApproveSection", locale: locale))

            VStack(alignment: .leading, spacing: 10) {
                TextField(
                    FabushiAgentStudioStrings.text("m4.mcp.qrManualPlaceholder", locale: locale),
                    text: $viewModel.pairTokenInput
                )
                .fabushiFont(.monospacedCaption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityIdentifier("fabushi.internal.mcp.qrPairInput")

                #if canImport(UIKit)
                if let extracted = FabushiMcpQrPairParser.extractPairToken(from: viewModel.pairTokenInput),
                   extracted.hasPrefix("pair_"),
                   let qrImage = FabushiQRCodeRenderer.renderQRCodeImage(from: extracted, scale: 6)
                {
                    HStack {
                        Spacer()
                        Image(uiImage: qrImage)
                            .interpolation(.none)
                            .resizable()
                            .scaledToFit()
                            .frame(width: 112, height: 112)
                            .padding(8)
                            .background(Color.white, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                            .accessibilityHidden(true)
                        Spacer()
                    }
                }
                #endif

                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 10) {
                        scanQrButton(locale: locale)
                        approveGrantButton(locale: locale)
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        scanQrButton(locale: locale)
                        approveGrantButton(locale: locale)
                    }
                }
            }
            .padding(12)
            .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        }
    }

    @ViewBuilder
    private func scanQrButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.mcp.qrScanButton", locale: locale),
            systemImage: "qrcode.viewfinder",
            accessibilityID: "fabushi.internal.mcp.qrScanButton",
            isProminent: false
        ) {
            isQrScannerPresented = true
        }
    }

    @ViewBuilder
    private func approveGrantButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.mcp.approveGrantButton", locale: locale),
            systemImage: "checkmark.seal.fill",
            accessibilityID: FabushiAccessibilityIDs.Studio.mcpApproveButton,
            isProminent: true
        ) {
            Task { @MainActor in
                _ = await viewModel.approveQrPairLogin(rawPayload: viewModel.pairTokenInput, locale: locale)
            }
        }
        .disabled(viewModel.pairTokenInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }

    // MARK: - 3. Redacted Tool Audit Log (`FabushiAccessibilityIDs.Studio.mcpAuditLogList`)

    @ViewBuilder
    private func redactedAuditLogSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.auditSection", locale: locale))

            if viewModel.isServerAuditPendingM6 {
                Text(FabushiAgentStudioStrings.text("m4.mcp.auditPendingM6Notice", locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }

            TextField(
                FabushiAgentStudioStrings.text("m4.mcp.auditFilterPlaceholder", locale: locale),
                text: $viewModel.auditSearchQuery
            )
            .fabushiFont(.body)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            Picker("Audit Status", selection: $viewModel.auditStatusFilter) {
                ForEach(FabushiMcpAuditStatusFilter.allCases) { filter in
                    Text(FabushiAgentStudioStrings.text(filter.localizationKey, locale: locale)).tag(filter)
                }
            }
            .pickerStyle(.segmented)

            VStack(alignment: .leading, spacing: 8) {
                let records = viewModel.filteredAuditRecords
                if records.isEmpty {
                    FabushiEmptyStateView(
                        title: FabushiAgentStudioStrings.text("m4.mcp.auditSection", locale: locale),
                        systemImage: "list.bullet.rectangle.portrait",
                        descriptionText: FabushiAgentStudioStrings.text("m4.mcp.auditEmpty", locale: locale),
                        accessibilityID: "fabushi.internal.mcp.auditEmptyState"
                    )
                } else {
                    ForEach(records) { record in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 8) {
                                Image(systemName: record.ok ? "checkmark.circle.fill" : "xmark.octagon.fill")
                                    .foregroundStyle(record.ok ? theme.successColor : theme.destructiveColor)
                                    .accessibilityHidden(true)
                                Text(record.toolName)
                                    .fabushiFont(.subheadline)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(theme.textPrimary)
                                Spacer(minLength: 4)
                                Text(record.inferredScope)
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.accentColor)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(theme.accentColor.opacity(0.14), in: Capsule())
                            }

                            Text(record.argsSummary)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(4)

                            if let errorCode = record.errorCode, !errorCode.isEmpty {
                                Text(errorCode)
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.destructiveColor)
                            }
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
            }
            .padding(10)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Studio.mcpAuditLogList)
        }
    }

    // MARK: - 4. Remote MCP Servers (HTTPS/SSE Only; `stdio` Blocked on iOS)

    @ViewBuilder
    private func remoteServersSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.serversSection", locale: locale))
                    Spacer(minLength: 8)
                    registerServerButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.serversSection", locale: locale))
                    registerServerButton(locale: locale)
                }
            }

            if viewModel.remoteServers.isEmpty {
                Text(FabushiAgentStudioStrings.text("m4.mcp.serversEmpty", locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            } else {
                ForEach(viewModel.remoteServers) { server in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            Text(server.name)
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                            Spacer(minLength: 4)
                            Text(server.transport)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textSecondary)
                        }
                        if let url = server.url, !url.isEmpty {
                            Text(url)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                }
            }
        }
    }

    @ViewBuilder
    private func registerServerButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.mcp.addServerButton", locale: locale),
            systemImage: "plus.circle.fill",
            accessibilityID: "fabushi.internal.mcp.registerServerButton",
            isProminent: false
        ) {
            draftServerName = ""
            draftServerTransport = .streamableHttp
            draftServerUrl = ""
            isRegisterServerSheetPresented = true
        }
    }

    // MARK: - 5. Official 31-Tool / 14-Scope Catalog & Embedded Local Registry Tools

    @ViewBuilder
    private func officialCatalogAndLocalRegistrySection(locale: FabushiLocale, isRegularWidth: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.catalogSection", locale: locale))

            DisclosureGroup(isExpanded: $isOfficialCatalogExpanded) {
                let columns = isRegularWidth
                    ? [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)]
                    : [GridItem(.flexible())]
                LazyVGrid(columns: columns, spacing: 8) {
                    ForEach(FabushiMcpOfficialCatalog.officialTools) { spec in
                        HStack(spacing: 8) {
                            Text(spec.name)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textPrimary)
                            Spacer(minLength: 4)
                            Text(spec.scope)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.accentColor)
                        }
                        .padding(8)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                }
                .padding(.top, 8)
            } label: {
                Text(
                    FabushiAgentStudioStrings.format(
                        "m4.mcp.officialCatalogFormat",
                        locale: locale,
                        Int32(FabushiMcpOfficialCatalog.officialTools.count),
                        Int32(FabushiMcpOfficialCatalog.supportedScopes.count)
                    )
                )
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.textPrimary)
            }
            .padding(12)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))

            if !viewModel.localRegistryTools.isEmpty {
                FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.mcp.localToolsSection", locale: locale))
                ForEach(viewModel.localRegistryTools) { tool in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            Text(tool.name)
                                .fabushiFont(.monospacedCaption)
                                .fontWeight(.semibold)
                                .foregroundStyle(theme.textPrimary)
                            Spacer(minLength: 4)
                            Text(tool.riskLevel)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textSecondary)
                        }
                        if !tool.description.isEmpty {
                            Text(tool.description)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
        }
    }

    // MARK: - Modal Register Server Sheet (Wrapped in `NavigationStack`)

    @ViewBuilder
    private func registerServerSheet(locale: FabushiLocale) -> some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.mcp.serverNamePlaceholder", locale: locale),
                        text: $draftServerName
                    )
                    Picker(
                        FabushiAgentStudioStrings.text("m4.mcp.transportLabel", locale: locale),
                        selection: $draftServerTransport
                    ) {
                        ForEach(FabushiMcpTransportKind.allCases) { kind in
                            Text(kind.displayName).tag(kind)
                        }
                    }
                    if draftServerTransport.isAllowedOnIos {
                        TextField(
                            FabushiAgentStudioStrings.text("m4.mcp.serverUrlPlaceholder", locale: locale),
                            text: $draftServerUrl
                        )
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    } else {
                        Text(FabushiAgentStudioStrings.text("m4.mcp.stdioBlockedNotice", locale: locale))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.warningColor)
                    }
                }
            }
            .navigationTitle(FabushiAgentStudioStrings.text("m4.mcp.addServerButton", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                        isRegisterServerSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(FabushiAgentStudioStrings.text("m4.common.save", locale: locale)) {
                        Task { @MainActor in
                            do {
                                _ = try await viewModel.registerServer(
                                    name: draftServerName,
                                    transport: draftServerTransport.rawValue,
                                    url: draftServerTransport.isAllowedOnIos ? draftServerUrl : nil,
                                    locale: locale
                                )
                                isRegisterServerSheetPresented = false
                            } catch {
                                viewModel.lastErrorMessage = error.localizedDescription
                            }
                        }
                    }
                    .disabled(draftServerName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}

// MARK: - M4 Native Camera QR Scanner + Simulator Manual Fallback Sheet (`FabushiAgentStudioStrings`)

private final class FabushiMcpQRMetadataRelay: NSObject, AVCaptureMetadataOutputObjectsDelegate, Sendable {
    private let didEmit = OSAllocatedUnfairLock(initialState: false)
    private let onDetected: @Sendable (String) -> Void

    init(onDetected: @escaping @Sendable (String) -> Void) {
        self.onDetected = onDetected
    }

    func reset() {
        didEmit.withLock { $0 = false }
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        for object in metadataObjects {
            guard let readable = object as? AVMetadataMachineReadableCodeObject,
                  readable.type == .qr,
                  let payload = readable.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !payload.isEmpty
            else {
                continue
            }
            let shouldEmit = didEmit.withLock { alreadyEmitted -> Bool in
                if alreadyEmitted { return false }
                alreadyEmitted = true
                return true
            }
            if shouldEmit {
                onDetected(payload)
            }
            return
        }
    }
}

#if canImport(UIKit)
@MainActor
private final class FabushiMcpCameraPreviewController: UIViewController {
    private let captureSession = AVCaptureSession()
    private let metadataQueue = DispatchQueue(label: "com.ombhrum.fabushi.m4.mcp.qr.metadata")
    private let sessionQueue = DispatchQueue(label: "com.ombhrum.fabushi.m4.mcp.qr.session")
    private let relay: FabushiMcpQRMetadataRelay
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var isConfigured = false

    init(onCode: @escaping @Sendable (String) -> Void) {
        self.relay = FabushiMcpQRMetadataRelay(onDetected: onCode)
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        configureIfHardwareAvailable()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        relay.reset()
        guard isConfigured else { return }
        let session = captureSession
        sessionQueue.async {
            if !session.isRunning {
                session.startRunning()
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        guard isConfigured else { return }
        let session = captureSession
        sessionQueue.async {
            if session.isRunning {
                session.stopRunning()
            }
        }
    }

    private func configureIfHardwareAvailable() {
        guard FabushiCameraAvailability.isHardwareCameraAvailable,
              let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              captureSession.canAddInput(input)
        else {
            return
        }
        captureSession.beginConfiguration()
        captureSession.addInput(input)
        let output = AVCaptureMetadataOutput()
        if captureSession.canAddOutput(output) {
            captureSession.addOutput(output)
            output.setMetadataObjectsDelegate(relay, queue: metadataQueue)
            if output.availableMetadataObjectTypes.contains(.qr) {
                output.metadataObjectTypes = [.qr]
            }
        }
        captureSession.commitConfiguration()

        let layer = AVCaptureVideoPreviewLayer(session: captureSession)
        layer.videoGravity = .resizeAspectFill
        layer.frame = view.bounds
        view.layer.addSublayer(layer)
        previewLayer = layer
        isConfigured = true
    }
}

@MainActor
private struct FabushiMcpCameraPreviewRepresentable: UIViewControllerRepresentable {
    let onDetectedCode: @MainActor (String) -> Void

    func makeUIViewController(context: Context) -> FabushiMcpCameraPreviewController {
        FabushiMcpCameraPreviewController { payload in
            Task { @MainActor in
                onDetectedCode(payload)
            }
        }
    }

    func updateUIViewController(_ uiViewController: FabushiMcpCameraPreviewController, context: Context) {}
}
#endif

@MainActor
private struct FabushiMcpQrScannerSheet: View {
    let onScannedPayload: (String) -> Void
    let onCancel: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var manualPayload: String = ""
    @State private var cameraAuthStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)

    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        let locale = theme.locale
        NavigationStack {
            ScrollView {
                VStack(spacing: 18) {
                    #if canImport(UIKit)
                    if FabushiCameraAvailability.isHardwareCameraAvailable && cameraAuthStatus == .authorized {
                        FabushiMcpCameraPreviewRepresentable { payload in
                            onScannedPayload(payload)
                        }
                        .frame(height: 240)
                        .clipShape(RoundedRectangle(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, style: .continuous))
                    } else {
                        VStack(spacing: 10) {
                            Image(systemName: "qrcode.viewfinder")
                                .fabushiFont(.largeTitle)
                                .foregroundStyle(theme.accentColor)
                                .accessibilityHidden(true)

                            Text(FabushiAgentStudioStrings.text("m4.mcp.cameraUnavailableNotice", locale: locale))
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textSecondary)
                                .multilineTextAlignment(.center)

                            if FabushiCameraAvailability.isHardwareCameraAvailable && cameraAuthStatus == .notDetermined {
                                FabushiGlassPillButton(
                                    title: FabushiAgentStudioStrings.text("m4.mcp.grantCameraButton", locale: locale),
                                    systemImage: "camera.fill",
                                    accessibilityID: "fabushi.internal.mcp.grantCameraButton",
                                    isProminent: false
                                ) {
                                    Task { @MainActor in
                                        let granted = await AVCaptureDevice.requestAccess(for: .video)
                                        cameraAuthStatus = granted ? .authorized : .denied
                                    }
                                }
                            }
                        }
                        .padding(18)
                        .frame(maxWidth: .infinity)
                        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius)
                    }
                    #endif

                    VStack(alignment: .leading, spacing: 10) {
                        TextField(
                            FabushiAgentStudioStrings.text("m4.mcp.qrManualPlaceholder", locale: locale),
                            text: $manualPayload,
                            axis: .vertical
                        )
                        .fabushiFont(.monospacedCaption)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .lineLimit(2...4)
                        .padding(12)
                        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.common.apply", locale: locale),
                            systemImage: "checkmark.circle.fill",
                            accessibilityID: "fabushi.internal.mcp.applyManualQr",
                            isProminent: true
                        ) {
                            let trimmed = manualPayload.trimmingCharacters(in: .whitespacesAndNewlines)
                            guard !trimmed.isEmpty else { return }
                            onScannedPayload(trimmed)
                        }
                        .disabled(manualPayload.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    .padding(14)
                    .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
                }
                .padding(16)
            }
            .background(theme.surfacePrimary)
            .navigationTitle(FabushiAgentStudioStrings.text("m4.mcp.qrScanButton", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale), action: onCancel)
                        .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}
