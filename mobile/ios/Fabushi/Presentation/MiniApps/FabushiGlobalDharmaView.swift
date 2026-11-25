import SwiftUI

/// Native Apple HIG surface for 全球法布施 (Global Dharma) backed by the real pure-Rust
/// `fabushi_miniapp` state machine & SQLite WAL (`miniapp.dharmaBootstrap`, `dharmaStateSnapshot`,
/// `dharmaDispatch`, `dharmaRecordPractice`, `dharmaLeaderboard`, `dharmaAssetBytes`) and
/// fail-closed `.fabapp` Ed25519 verification (`FabAppPackageVerifier`).
@MainActor
public struct FabushiGlobalDharmaView: View {
    @Bindable public var viewModel: FabushiMiniAppHostViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var selectedSutra: String = "心经"
    @State private var durationMinutes: Int = 15
    @State private var reflectionNotes: String = ""

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    private let availableSutras = ["心经", "金刚经", "普门品", "阿弥陀经", "地藏经"]

    public init(viewModel: FabushiMiniAppHostViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                headerCard
                practiceControlCard
                assetIntegrityCard
                leaderboardCard
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.miniAppGlobalDharmaTitle, locale: locale))
        .task {
            await viewModel.loadGlobalDharma()
        }
    }

    private var headerCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(FabushiPlatformStrings.text(.miniAppGlobalDharmaTitle, locale: locale))
                    .fabushiFont(.title3)
                    .foregroundStyle(theme.textPrimary)
                Spacer()
                Text(
                    viewModel.isGlobalDharmaPackageVerified
                        ? FabushiPlatformStrings.text(.miniAppGlobalDharmaVerifiedBadge, locale: locale)
                        : FabushiPlatformStrings.text(.miniAppGlobalDharmaUnverifiedBadge, locale: locale)
                )
                .fabushiFont(.caption2)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(
                    (viewModel.isGlobalDharmaPackageVerified ? theme.successColor : theme.warningColor).opacity(0.18),
                    in: Capsule()
                )
                .foregroundStyle(viewModel.isGlobalDharmaPackageVerified ? theme.successColor : theme.warningColor)
            }
            Text(FabushiPlatformStrings.text(.miniAppGlobalDharmaSubtitle, locale: locale))
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)

            if let verifyError = viewModel.globalDharmaVerificationError {
                Text(verifyError)
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.destructiveColor)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    private var practiceControlCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            Picker("Sutra", selection: $selectedSutra) {
                ForEach(availableSutras, id: \.self) { sutra in
                    Text(sutra).tag(sutra)
                }
            }
            .pickerStyle(.segmented)
            .onChange(of: selectedSutra) { _, newTitle in
                Task {
                    _ = await viewModel.dispatchDharmaCommand(
                        type: "select_dharma_sutra",
                        extraFields: ["title": .string(newTitle)]
                    )
                }
            }

            HStack(spacing: 12) {
                Button {
                    Task {
                        let cmd = viewModel.isPracticeSessionActive ? "end_practice_session" : "start_practice_session"
                        _ = await viewModel.dispatchDharmaCommand(type: cmd)
                    }
                } label: {
                    Label(
                        viewModel.isPracticeSessionActive
                            ? FabushiPlatformStrings.text(.miniAppGlobalDharmaEndSession, locale: locale)
                            : FabushiPlatformStrings.text(.miniAppGlobalDharmaStartSession, locale: locale),
                        systemImage: viewModel.isPracticeSessionActive ? "stop.circle.fill" : "play.circle.fill"
                    )
                    .fabushiFont(.subheadline)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                    .foregroundStyle(theme.accentForegroundColor)
                }
                .fabushiMinimumTouchTarget()

                Button {
                    Task {
                        _ = await viewModel.dispatchDharmaCommand(type: "increment_chant")
                    }
                } label: {
                    Text("\(FabushiPlatformStrings.text(.miniAppGlobalDharmaChantButton, locale: locale)) (\(viewModel.localChantCount))")
                        .fabushiFont(.subheadline)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 10)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                        .foregroundStyle(theme.textPrimary)
                }
                .fabushiMinimumTouchTarget()
            }

            Stepper(value: $durationMinutes, in: 1...240) {
                Text(FabushiPlatformStrings.text(locale, en: "Duration: \(durationMinutes) min", zhHans: "共修时长：\(durationMinutes) 分钟"))
                    .fabushiFont(.subheadline)
            }

            TextField(
                FabushiPlatformStrings.text(locale, en: "Practice reflection (optional)", zhHans: "回向 / 修行随笔（可选）"),
                text: $reflectionNotes
            )
            .fabushiFont(.body)
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))

            Button {
                let count = max(1, UInt32(viewModel.localChantCount))
                let mins = UInt32(durationMinutes)
                let notes = reflectionNotes.trimmingCharacters(in: .whitespacesAndNewlines)
                Task {
                    _ = await viewModel.recordDharmaPractice(
                        sutra: selectedSutra,
                        durationMinutes: mins,
                        chantCount: count,
                        notes: notes.isEmpty ? nil : notes,
                        locale: locale
                    )
                }
            } label: {
                Label(
                    FabushiPlatformStrings.text(.miniAppGlobalDharmaRecordPractice, locale: locale),
                    systemImage: "checkmark.seal.fill"
                )
                .fabushiFont(.headline)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
                .background(theme.successColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                .foregroundStyle(Color.white)
            }
            .fabushiMinimumTouchTarget()
        }
        .padding(14)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    private var assetIntegrityCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(FabushiPlatformStrings.text(.miniAppGlobalDharmaAssetIntegrity, locale: locale))
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
            if let wasm = viewModel.dharmaWasmAssetInfo {
                Text("\(wasm.name) · \(wasm.mimeType) · \(wasm.sizeBytes) B · WASM=\(wasm.hasWasmMagic ? "OK" : "INVALID")")
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(wasm.hasWasmMagic ? theme.successColor : theme.destructiveColor)
            } else {
                Text(FabushiPlatformStrings.text(.loading, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    private var leaderboardCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(FabushiPlatformStrings.text(.miniAppGlobalDharmaLeaderboardTitle, locale: locale))
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)

            if let board = viewModel.dharmaLeaderboard {
                let entries = board["entries"]?.arrayValue ?? board.arrayValue ?? []
                if entries.isEmpty {
                    Text(FabushiPlatformStrings.text(locale, en: "Record a practice session to appear on the leaderboard.", zhHans: "完成一次共修记录后即可显示于全球修行榜。"))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(Array(entries.prefix(10).enumerated()), id: \.offset) { idx, entry in
                        let name = entry["display_name"]?.stringValue
                            ?? entry["username"]?.stringValue
                            ?? "#\(idx + 1)"
                        let total = entry["total_merit"]?.int64Value
                            ?? entry["total_chants"]?.int64Value
                            ?? entry["practice_count"]?.int64Value
                            ?? 0
                        HStack {
                            Text("#\(idx + 1) \(name)")
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textPrimary)
                            Spacer()
                            Text("\(total)")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.accentColor)
                        }
                        .padding(.vertical, 4)
                    }
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }
}
