//
//  MediaTransferManager.swift
//  Fabushi
//
//  Production 512KB chunked media transfer manager backed strictly by canonical M1b Rust
//  `media.chunkedSessionCreate` / `media.chunkedSessionReceivePart` / `media.chunkedSessionFinalize`
//  and `media.casGet` / `media.download` RPCs with SHA-256 verification, atomic disk CAS staging,
//  and enforced maxConcurrentTransfers = 4 concurrency throttling (zero mock paths).
//

import CryptoKit
import Foundation

public enum MediaTransferError: Error, Sendable, Equatable {
    case fileNotFound(URL)
    case emptyFile
    case invalidChunkSize(expected: Int, actual: Int)
    case chunkIndexOutOfBounds(index: Int, totalChunks: Int)
    case sha256Mismatch(expected: String, actual: String)
    case transferCancelled(String)
    case maxRetriesExceeded(String)
    case cacheWriteFailed(String)
    case serverError(String)
}

// MARK: - Exact M1b Rust Media RPC DTOs (source/host/src/handlers_messaging.rs:1790-1952)

private struct ChunkedSessionCreateParams: Encodable, Sendable {
    let file_id: String
    let total_size: UInt64
    let part_size: UInt32
    let expected_sha256: String
}

private struct ChunkedSessionCreateResponse: Decodable, Sendable {
    let file_id: String
    let total_size: UInt64
    let part_size: UInt32
    let total_parts: UInt32
}

private struct ChunkedSessionReceivePartParams: Encodable, Sendable {
    let file_id: String
    let part_index: UInt32
    let data_b64: String
}

private struct ChunkedSessionReceivePartResponse: Decodable, Sendable {
    let file_id: String
    let part_index: UInt32
    let is_complete: Bool
    let missing_parts: [UInt32]
}

private struct ChunkedSessionFinalizeParams: Encodable, Sendable {
    let file_id: String
    let mime_type: String
}

private struct ChunkedSessionFinalizeResponse: Decodable, Sendable {
    let file_id: String
    let verified: Bool
    let sha256: String
    let size_bytes: UInt64
    let local_path: String?
}

private struct MediaFileIdParams: Encodable, Sendable {
    let file_id: String
}

private struct MediaDownloadPayloadResponse: Decodable, Sendable {
    let size_bytes: UInt64
    let data_b64: String
}

// MARK: - Production Media Transfer Actor

public actor MediaTransferManager {
    public static let shared = MediaTransferManager(host: .shared)
    public static let chunkSize: Int = 512 * 1024 // 512 KB
    public static let maxConcurrentTransfers: Int = 4
    public static let maxRetries: Int = 5

    public enum TransferProgress: Sendable, Equatable {
        case progress(transferId: String, fractionCompleted: Double, bytesTransferred: Int64, totalBytes: Int64)
        case completed(transferId: String, mediaId: String, fileURL: URL)
        case failed(transferId: String, error: String)
    }

    private let host: FabushiRustHostActor
    private var activeTransfers: Set<String> = []
    private var cancelledTransfers: Set<String> = []
    private var completedChunksByTransfer: [String: Set<Int>] = [:]

    // Enforced concurrency throttle (maxConcurrentTransfers = 4)
    private var activeSlotCount: Int = 0
    private var peakObservedConcurrency: Int = 0
    private var slotWaiters: [CheckedContinuation<Void, Never>] = []

    public init(host: FabushiRustHostActor = .shared) {
        self.host = host
    }

    public func currentActiveTransferCount() -> Int {
        activeSlotCount
    }

    public func maxObservedConcurrentTransfers() -> Int {
        peakObservedConcurrency
    }

    // MARK: - SHA-256 CAS Calculation

    public static func computeSHA256(data: Data) -> String {
        let digest = SHA256.hash(data: data)
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    // MARK: - Chunk Partitioning Helper

    public static func partition(data: Data, chunkSize: Int = 512 * 1024) throws -> [Data] {
        guard !data.isEmpty else {
            throw MediaTransferError.emptyFile
        }
        guard chunkSize > 0 else {
            throw MediaTransferError.invalidChunkSize(expected: Self.chunkSize, actual: chunkSize)
        }

        var chunks: [Data] = []
        var offset = 0
        while offset < data.count {
            let length = min(chunkSize, data.count - offset)
            let sub = data.subdata(in: offset..<(offset + length))
            chunks.append(sub)
            offset += length
        }
        return chunks
    }

    // MARK: - Chunk Index Validation & Access

    public static func validateChunkIndex(index: Int, totalChunks: Int) throws {
        guard index >= 0 && index < totalChunks else {
            throw MediaTransferError.chunkIndexOutOfBounds(index: index, totalChunks: totalChunks)
        }
    }

    public static func chunk(at index: Int, from data: Data, chunkSize: Int = 512 * 1024) throws -> Data {
        let chunks = try partition(data: data, chunkSize: chunkSize)
        guard index >= 0 && index < chunks.count else {
            throw MediaTransferError.chunkIndexOutOfBounds(index: index, totalChunks: chunks.count)
        }
        return chunks[index]
    }

    // MARK: - Cache Directory Management

    public static func mediaCacheDirectory() throws -> URL {
        let fileManager = FileManager.default
        let caches = fileManager.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        let dir = caches.appendingPathComponent("Fabushi/Media", isDirectory: true)
        if !fileManager.fileExists(atPath: dir.path) {
            try fileManager.createDirectory(at: dir, withIntermediateDirectories: true)
        }
        return dir
    }

    public func cachedMediaURL(forSha256 sha256: String, ext: String = "dat") -> URL? {
        guard let dir = try? Self.mediaCacheDirectory() else { return nil }
        let cleanExt = ext.isEmpty ? "dat" : ext
        let fileURL = dir.appendingPathComponent("\(sha256).\(cleanExt)")
        if FileManager.default.fileExists(atPath: fileURL.path) {
            return fileURL
        }
        return nil
    }

    // MARK: - Cancellation

    public func cancelTransfer(transferId: String) {
        cancelledTransfers.insert(transferId)
        activeTransfers.remove(transferId)
    }

    public func isCancelled(transferId: String) -> Bool {
        cancelledTransfers.contains(transferId)
    }

    // MARK: - Real Rust Chunked Upload (`chunkedSessionCreate` -> `ReceivePart` -> `Finalize`)

    public func uploadMedia(
        fileURL: URL,
        mimeType: String,
        fileName: String,
        fileId: String? = nil,
        resumedFromChunk: Int = 0,
        progressStream: AsyncStream<TransferProgress>.Continuation? = nil
    ) async throws -> String {
        await acquireTransferSlot()
        defer { releaseTransferSlot() }

        let transferId = fileId ?? UUID().uuidString
        activeTransfers.insert(transferId)
        defer {
            activeTransfers.remove(transferId)
            cancelledTransfers.remove(transferId)
            completedChunksByTransfer.removeValue(forKey: transferId)
        }

        guard !Task.isCancelled && !isCancelled(transferId: transferId) else {
            throw MediaTransferError.transferCancelled(transferId)
        }

        guard FileManager.default.fileExists(atPath: fileURL.path) else {
            throw MediaTransferError.fileNotFound(fileURL)
        }

        let data = try Data(contentsOf: fileURL)
        guard !data.isEmpty else {
            throw MediaTransferError.emptyFile
        }

        let chunks = try Self.partition(data: data, chunkSize: Self.chunkSize)
        let totalChunks = chunks.count
        let totalBytes = Int64(data.count)
        let fullFileSha256 = Self.computeSHA256(data: data)

        // Step 1: Create Rust chunked upload session with expected SHA-256 verification.
        if resumedFromChunk == 0 || fileId == nil {
            let createParams = ChunkedSessionCreateParams(
                file_id: transferId,
                total_size: UInt64(data.count),
                part_size: UInt32(Self.chunkSize),
                expected_sha256: fullFileSha256
            )
            do {
                let _: ChunkedSessionCreateResponse = try await host.dispatch(
                    method: FabushiRPCNamespace.media.method("chunkedSessionCreate"),
                    params: createParams
                )
            } catch is CancellationError {
                throw MediaTransferError.transferCancelled(transferId)
            } catch let hostError as FabushiHostError {
                if hostError.isCancelled || isCancelled(transferId: transferId) || Task.isCancelled {
                    throw MediaTransferError.transferCancelled(transferId)
                }
                progressStream?.yield(.failed(transferId: transferId, error: hostError.localizedDescription))
                throw MediaTransferError.serverError(hostError.localizedDescription)
            } catch {
                if isCancelled(transferId: transferId) || Task.isCancelled {
                    throw MediaTransferError.transferCancelled(transferId)
                }
                progressStream?.yield(.failed(transferId: transferId, error: error.localizedDescription))
                throw MediaTransferError.serverError(error.localizedDescription)
            }
        }

        var transferredBytes: Int64 = 0
        var completed = completedChunksByTransfer[transferId] ?? Set<Int>()
        if resumedFromChunk > 0 {
            for i in 0..<min(resumedFromChunk, totalChunks) {
                completed.insert(i)
                transferredBytes += Int64(chunks[i].count)
            }
        }

        // Step 2: Upload each 512KB chunk via `media.chunkedSessionReceivePart`.
        for (index, chunkData) in chunks.enumerated() {
            guard !isCancelled(transferId: transferId) && !Task.isCancelled else {
                throw MediaTransferError.transferCancelled(transferId)
            }

            if completed.contains(index) {
                continue
            }

            let partParams = ChunkedSessionReceivePartParams(
                file_id: transferId,
                part_index: UInt32(index),
                data_b64: chunkData.base64EncodedString()
            )

            var retries = 0
            var succeeded = false
            while retries < Self.maxRetries && !succeeded {
                guard !isCancelled(transferId: transferId) && !Task.isCancelled else {
                    throw MediaTransferError.transferCancelled(transferId)
                }

                do {
                    let _: ChunkedSessionReceivePartResponse = try await host.dispatch(
                        method: FabushiRPCNamespace.media.method("chunkedSessionReceivePart"),
                        params: partParams
                    )
                    succeeded = true
                } catch is CancellationError {
                    throw MediaTransferError.transferCancelled(transferId)
                } catch let hostError as FabushiHostError {
                    if hostError.isCancelled || isCancelled(transferId: transferId) || Task.isCancelled {
                        throw MediaTransferError.transferCancelled(transferId)
                    }
                    if isNonRetryableHostError(hostError) {
                        progressStream?.yield(
                            .failed(transferId: transferId, error: "Chunk \(index) failed: \(hostError.localizedDescription)")
                        )
                        throw MediaTransferError.serverError(hostError.localizedDescription)
                    }
                    retries += 1
                    if retries >= Self.maxRetries {
                        progressStream?.yield(
                            .failed(transferId: transferId, error: "Chunk \(index) failed: \(hostError.localizedDescription)")
                        )
                        throw MediaTransferError.maxRetriesExceeded("Chunk \(index) failed after \(retries) retries")
                    }
                    let backoffNs = UInt64(pow(2.0, Double(retries)) * 50_000_000)
                    try? await Task.sleep(nanoseconds: backoffNs)
                } catch {
                    if isCancelled(transferId: transferId) || Task.isCancelled {
                        throw MediaTransferError.transferCancelled(transferId)
                    }
                    retries += 1
                    if retries >= Self.maxRetries {
                        progressStream?.yield(
                            .failed(transferId: transferId, error: "Chunk \(index) failed: \(error.localizedDescription)")
                        )
                        throw MediaTransferError.maxRetriesExceeded("Chunk \(index) failed after \(retries) retries")
                    }
                    let backoffNs = UInt64(pow(2.0, Double(retries)) * 50_000_000)
                    try? await Task.sleep(nanoseconds: backoffNs)
                }
            }

            completed.insert(index)
            completedChunksByTransfer[transferId] = completed
            transferredBytes += Int64(chunkData.count)

            let fraction = Double(transferredBytes) / Double(totalBytes)
            progressStream?.yield(
                .progress(
                    transferId: transferId,
                    fractionCompleted: min(1.0, fraction),
                    bytesTransferred: transferredBytes,
                    totalBytes: totalBytes
                )
            )
        }

        // Step 3: Finalize upload & verify assembled SHA-256 in Rust CAS.
        let finalizeParams = ChunkedSessionFinalizeParams(
            file_id: transferId,
            mime_type: mimeType
        )
        let finalizeResp: ChunkedSessionFinalizeResponse
        do {
            finalizeResp = try await host.dispatch(
                method: FabushiRPCNamespace.media.method("chunkedSessionFinalize"),
                params: finalizeParams
            )
        } catch is CancellationError {
            throw MediaTransferError.transferCancelled(transferId)
        } catch let hostError as FabushiHostError {
            if hostError.isCancelled || isCancelled(transferId: transferId) || Task.isCancelled {
                throw MediaTransferError.transferCancelled(transferId)
            }
            progressStream?.yield(.failed(transferId: transferId, error: hostError.localizedDescription))
            throw MediaTransferError.serverError(hostError.localizedDescription)
        } catch {
            if isCancelled(transferId: transferId) || Task.isCancelled {
                throw MediaTransferError.transferCancelled(transferId)
            }
            progressStream?.yield(.failed(transferId: transferId, error: error.localizedDescription))
            throw MediaTransferError.serverError(error.localizedDescription)
        }

        guard finalizeResp.verified,
              finalizeResp.sha256.lowercased() == fullFileSha256.lowercased()
        else {
            throw MediaTransferError.sha256Mismatch(expected: fullFileSha256, actual: finalizeResp.sha256)
        }

        progressStream?.yield(.completed(transferId: transferId, mediaId: finalizeResp.file_id, fileURL: fileURL))
        return finalizeResp.file_id
    }

    // MARK: - Real Rust CAS / Remote Download (`media.casGet` -> `media.download`)

    public func downloadMedia(
        mediaId: String,
        fileSha256: String,
        totalBytes: Int64,
        fileName: String,
        progressStream: AsyncStream<TransferProgress>.Continuation? = nil
    ) async throws -> URL {
        let ext = URL(fileURLWithPath: fileName).pathExtension
        if !fileSha256.isEmpty, let cached = cachedMediaURL(forSha256: fileSha256, ext: ext) {
            progressStream?.yield(.completed(transferId: mediaId, mediaId: mediaId, fileURL: cached))
            return cached
        }

        await acquireTransferSlot()
        defer { releaseTransferSlot() }

        let payloadResp: MediaDownloadPayloadResponse
        do {
            payloadResp = try await host.dispatch(
                method: FabushiRPCNamespace.media.method("casGet"),
                params: MediaFileIdParams(file_id: mediaId)
            )
        } catch FabushiHostError.notFound {
            do {
                payloadResp = try await host.dispatch(
                    method: FabushiRPCNamespace.media.method("download"),
                    params: MediaFileIdParams(file_id: mediaId)
                )
            } catch {
                progressStream?.yield(.failed(transferId: mediaId, error: error.localizedDescription))
                throw MediaTransferError.serverError(error.localizedDescription)
            }
        } catch {
            progressStream?.yield(.failed(transferId: mediaId, error: error.localizedDescription))
            throw MediaTransferError.serverError(error.localizedDescription)
        }

        guard let assembledData = Data(base64Encoded: payloadResp.data_b64) else {
            throw MediaTransferError.serverError("Invalid base64 payload returned from Rust media host")
        }

        let reportedTotal = totalBytes > 0 ? totalBytes : Int64(assembledData.count)
        progressStream?.yield(
            .progress(
                transferId: mediaId,
                fractionCompleted: 1.0,
                bytesTransferred: Int64(assembledData.count),
                totalBytes: reportedTotal
            )
        )

        let calculatedSha256 = Self.computeSHA256(data: assembledData)
        if !fileSha256.isEmpty && calculatedSha256.lowercased() != fileSha256.lowercased() {
            throw MediaTransferError.sha256Mismatch(expected: fileSha256, actual: calculatedSha256)
        }

        let cacheKey = fileSha256.isEmpty ? calculatedSha256 : fileSha256
        let cacheDir = try Self.mediaCacheDirectory()
        let stagingURL = cacheDir.appendingPathComponent(".tmp_\(UUID().uuidString)")
        let targetURL = cacheDir.appendingPathComponent("\(cacheKey).\(ext.isEmpty ? "dat" : ext)")

        do {
            try assembledData.write(to: stagingURL, options: .atomic)
            if FileManager.default.fileExists(atPath: targetURL.path) {
                try FileManager.default.removeItem(at: targetURL)
            }
            try FileManager.default.moveItem(at: stagingURL, to: targetURL)
        } catch {
            try? FileManager.default.removeItem(at: stagingURL)
            throw MediaTransferError.cacheWriteFailed(error.localizedDescription)
        }

        progressStream?.yield(.completed(transferId: mediaId, mediaId: mediaId, fileURL: targetURL))
        return targetURL
    }

    // MARK: - Enforced Concurrency Semaphore

    private func acquireTransferSlot() async {
        while activeSlotCount >= Self.maxConcurrentTransfers {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                slotWaiters.append(continuation)
            }
        }
        activeSlotCount += 1
        if activeSlotCount > peakObservedConcurrency {
            peakObservedConcurrency = activeSlotCount
        }
    }

    private func releaseTransferSlot() {
        activeSlotCount = max(0, activeSlotCount - 1)
        if !slotWaiters.isEmpty {
            let next = slotWaiters.removeFirst()
            next.resume()
        }
    }

    private func isNonRetryableHostError(_ error: FabushiHostError) -> Bool {
        switch error {
        case .notFound,
             .invalidArguments,
             .hostClosed,
             .notInitialized,
             .cryptoError,
             .unknownCommand,
             .capabilityUnavailable,
             .upstreamNotImplemented,
             .unauthorized,
             .permissionDenied,
             .rustPanic:
            return true
        default:
            return false
        }
    }
}
