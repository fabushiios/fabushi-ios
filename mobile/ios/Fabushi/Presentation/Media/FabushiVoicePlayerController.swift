import AVFoundation
import Foundation
import Observation
import SwiftUI

// MARK: - Swift 6 5-Bit Waveform Voice Player Controller (`#20`)

/// Swift 6 `@MainActor @Observable` real `AVAudioPlayer`-backed voice playback & on-device
/// transcription controller completely independent of legacy `MessagingModel` / `VoicePlaybackController`.
@MainActor
@Observable
public final class FabushiVoicePlayerController: NSObject, AVAudioPlayerDelegate {
    public static let supportedRates: [Float] = [1.0, 1.5, 2.0]

    public private(set) var activeFileId: String? = nil
    public private(set) var isPlaying: Bool = false
    public private(set) var isLoading: Bool = false
    public private(set) var currentTime: TimeInterval = 0
    public private(set) var duration: TimeInterval = 0
    public private(set) var playbackRate: Float = 1.0
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var transcriptsByFileId: [String: String] = [:]
    public private(set) var transcribingFileId: String? = nil

    @ObservationIgnored private var player: AVAudioPlayer? = nil
    @ObservationIgnored private var progressTask: Task<Void, Never>? = nil
    @ObservationIgnored private var localTempURLsByFileId: [String: URL] = [:]
    @ObservationIgnored private let transcriber = OfflineSpeechTranscriber()

    public override init() {
        super.init()
    }

    public var progress: Double {
        guard duration > 0 else { return 0 }
        return min(1.0, max(0.0, currentTime / duration))
    }

    /// Decodes Telegram 63-byte 5-bit packed waveforms (or pre-decoded 0..31 amplitude bars)
    /// into normalized `[0.0...1.0]` bar heights for display.
    public static func normalizedWaveformBars(from rawBytes: [UInt8], targetCount: Int = 44) -> [Double] {
        let clampedTarget = max(8, targetCount)
        if rawBytes.count == WaveformCodec5Bit.packedByteCount,
           let decoded = try? WaveformCodec5Bit.decode(packed: rawBytes) {
            let doubles = decoded.map { Double($0) / Double(WaveformCodec5Bit.maximumLevel) }
            return WaveformCodec5Bit.resample(samples: doubles, targetCount: clampedTarget)
        }
        if !rawBytes.isEmpty {
            let maxVal = max(Double(rawBytes.max() ?? 1), 1.0)
            let doubles = rawBytes.map { Double($0) / maxVal }
            return WaveformCodec5Bit.resample(samples: doubles, targetCount: clampedTarget)
        }
        // Deterministic calm fallback silhouette when a remote audio payload omitted waveform bytes.
        return (0..<clampedTarget).map { idx in
            let wave = sin(Double(idx) * 0.45) * 0.35 + 0.50
            return min(0.95, max(0.18, wave))
        }
    }

    public func togglePlay(
        fileId: String,
        fallbackDurationSeconds: UInt32,
        chatId: Int64,
        store: FabushiMessagingStore,
        router: ShellNavigationRouter,
        isCompact: Bool,
        locale: FabushiLocale
    ) async {
        if activeFileId == fileId, let player {
            if isPlaying {
                player.pause()
                isPlaying = false
                stopProgressTask()
            } else {
                player.rate = playbackRate
                player.play()
                isPlaying = true
                startProgressTask()
            }
            return
        }

        stop()
        activeFileId = fileId
        isLoading = true
        lastErrorMessage = nil
        duration = TimeInterval(fallbackDurationSeconds)

        guard let audioURL = await resolveAudioFileURL(
            fileId: fileId,
            chatId: chatId,
            store: store,
            router: router,
            isCompact: isCompact,
            locale: locale
        ) else {
            isLoading = false
            activeFileId = nil
            return
        }

        do {
            #if os(iOS)
            let session = AVAudioSession.sharedInstance()
            try? session.setCategory(.playback, mode: .spokenAudio, options: [.duckOthers])
            try? session.setActive(true)
            #endif
            let audioPlayer = try AVAudioPlayer(contentsOf: audioURL)
            audioPlayer.delegate = self
            audioPlayer.enableRate = true
            audioPlayer.rate = playbackRate
            audioPlayer.prepareToPlay()
            self.player = audioPlayer
            self.duration = audioPlayer.duration > 0 ? audioPlayer.duration : TimeInterval(fallbackDurationSeconds)
            self.currentTime = 0
            self.isLoading = false
            if audioPlayer.play() {
                self.isPlaying = true
                startProgressTask()
            }
        } catch {
            self.isLoading = false
            self.isPlaying = false
            self.lastErrorMessage = error.localizedDescription
        }
    }

    /// Plays already-materialized audio data directly (usable from previews & real-audio XCTests).
    public func loadAudioData(_ data: Data, fileId: String) throws {
        stop()
        let audioPlayer = try AVAudioPlayer(data: data)
        audioPlayer.delegate = self
        audioPlayer.enableRate = true
        audioPlayer.rate = playbackRate
        audioPlayer.prepareToPlay()
        self.player = audioPlayer
        self.activeFileId = fileId
        self.duration = audioPlayer.duration
        self.currentTime = 0
        self.isPlaying = false
        self.lastErrorMessage = nil
    }

    public func seek(toProgress fraction: Double) {
        let clamped = min(1.0, max(0.0, fraction))
        guard let player, duration > 0 else {
            currentTime = duration * clamped
            return
        }
        let target = duration * clamped
        player.currentTime = target
        currentTime = target
    }

    public func cyclePlaybackRate() {
        let nextIndex: Int
        if let idx = Self.supportedRates.firstIndex(of: playbackRate) {
            nextIndex = (idx + 1) % Self.supportedRates.count
        } else {
            nextIndex = 0
        }
        playbackRate = Self.supportedRates[nextIndex]
        player?.rate = playbackRate
    }

    public func stop() {
        stopProgressTask()
        player?.stop()
        player = nil
        isPlaying = false
        isLoading = false
        currentTime = 0
    }

    /// Runs fail-closed on-device `SFSpeechRecognizer` transcription via compiled `OfflineSpeechTranscriber`.
    public func transcribeVoiceNote(
        fileId: String,
        chatId: Int64,
        store: FabushiMessagingStore,
        router: ShellNavigationRouter,
        isCompact: Bool,
        locale: FabushiLocale
    ) async {
        guard transcribingFileId == nil else { return }
        transcribingFileId = fileId
        lastErrorMessage = nil
        defer { transcribingFileId = nil }

        guard let audioURL = await resolveAudioFileURL(
            fileId: fileId,
            chatId: chatId,
            store: store,
            router: router,
            isCompact: isCompact,
            locale: locale
        ) else {
            return
        }

        do {
            let text = try await transcriber.transcribe(fileURL: audioURL, locale: locale.foundationLocale)
            transcriptsByFileId[fileId] = text
        } catch {
            lastErrorMessage = error.localizedDescription
        }
    }

    private func resolveAudioFileURL(
        fileId: String,
        chatId: Int64,
        store: FabushiMessagingStore,
        router: ShellNavigationRouter,
        isCompact: Bool,
        locale: FabushiLocale
    ) async -> URL? {
        if let existing = localTempURLsByFileId[fileId],
           FileManager.default.fileExists(atPath: existing.path) {
            return existing
        }
        if let casPath = try? await store.client.localCasPath(fileId: fileId),
           !casPath.isEmpty,
           FileManager.default.fileExists(atPath: casPath) {
            let url = URL(fileURLWithPath: casPath)
            localTempURLsByFileId[fileId] = url
            return url
        }
        guard let bytes = await store.downloadAttachment(
            fileId: fileId,
            fileName: "\(fileId).m4a",
            chatId: chatId,
            router: router,
            isCompact: isCompact,
            locale: locale
        ) else {
            return nil
        }
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi-voice-cache", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let safeName = fileId.replacingOccurrences(of: "/", with: "_")
        let targetURL = dir.appendingPathComponent("\(safeName).m4a")
        do {
            try bytes.write(to: targetURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            localTempURLsByFileId[fileId] = targetURL
            return targetURL
        } catch {
            return nil
        }
    }

    private func startProgressTask() {
        stopProgressTask()
        progressTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 80_000_000)
                guard !Task.isCancelled, let self, let player = self.player else { break }
                self.currentTime = player.currentTime
                if !player.isPlaying {
                    self.isPlaying = false
                    break
                }
            }
        }
    }

    private func stopProgressTask() {
        progressTask?.cancel()
        progressTask = nil
    }

    public nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.stopProgressTask()
            self.isPlaying = false
            self.currentTime = 0
        }
    }
}

// MARK: - 5-Bit Waveform Bars View (`FabushiAccessibilityIDs.Stage.waveformView`)

@MainActor
public struct FabushiWaveformBarsView: View {
    public let bars: [Double]
    public let progress: Double
    public let activeTint: Color
    public let inactiveTint: Color
    public let onSeek: ((Double) -> Void)?

    public init(
        bars: [Double],
        progress: Double,
        activeTint: Color,
        inactiveTint: Color,
        onSeek: ((Double) -> Void)? = nil
    ) {
        self.bars = bars
        self.progress = progress
        self.activeTint = activeTint
        self.inactiveTint = inactiveTint
        self.onSeek = onSeek
    }

    public var body: some View {
        let clampedProgress = min(1.0, max(0.0, progress))
        let activeBarIndex = Int(round(clampedProgress * Double(max(0, bars.count - 1))))

        GeometryReader { proxy in
            HStack(alignment: .center, spacing: 2) {
                ForEach(Array(bars.enumerated()), id: \.offset) { index, level in
                    let height = max(4.0, min(26.0, CGFloat(level) * 26.0))
                    Capsule(style: .continuous)
                        .fill(index <= activeBarIndex && clampedProgress > 0 ? activeTint : inactiveTint)
                        .frame(maxWidth: .infinity)
                        .frame(height: height)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        guard proxy.size.width > 0 else { return }
                        let fraction = min(1.0, max(0.0, Double(value.location.x / proxy.size.width)))
                        onSeek?(fraction)
                    }
            )
        }
        .frame(height: 28)
        .accessibilityElement(children: .ignore)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.waveformView)
        .accessibilityValue("\(Int(clampedProgress * 100))%")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment:
                onSeek?(min(1.0, clampedProgress + 0.1))
            case .decrement:
                onSeek?(max(0.0, clampedProgress - 0.1))
            @unknown default:
                break
            }
        }
    }
}
