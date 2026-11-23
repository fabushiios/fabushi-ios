//
//  SearchIndexingBridge.swift
//  Fabushi
//
//  SQLite WAL & Tantivy full-text search bridge backed strictly by canonical Rust host
//  `search.indexMessage`, `search.localMessages`, `search.rebuildLocalIndex`, and
//  `settings.checkpointWal` RPCs (zero in-memory fake indices).
//

import Foundation

// MARK: - Models

public struct SearchDocument: Identifiable, Sendable, Equatable {
    public let id: Int64
    public let chatId: Int64
    public let senderId: Int64
    public let text: String
    public let timestamp: Date

    public init(id: Int64, chatId: Int64, senderId: Int64 = 1, text: String, timestamp: Date = Date()) {
        self.id = id
        self.chatId = chatId
        self.senderId = senderId
        self.text = text
        self.timestamp = timestamp
    }
}

public struct SearchQueryParams: Sendable, Equatable {
    public let query: String
    public let chatId: Int64?
    public let limit: Int

    public init(query: String, chatId: Int64? = nil, limit: Int = 20) {
        self.query = query
        self.chatId = chatId
        self.limit = limit
    }
}

public struct SearchResultItem: Identifiable, Sendable, Equatable {
    public let id: Int64
    public let chatId: Int64
    public let snippetWithHighlights: String
    public let score: Float
    public let engine: String

    public init(
        id: Int64,
        chatId: Int64,
        snippetWithHighlights: String,
        score: Float = 1.0,
        engine: String = "tantivy"
    ) {
        self.id = id
        self.chatId = chatId
        self.snippetWithHighlights = snippetWithHighlights
        self.score = score
        self.engine = engine
    }
}

public struct SearchIndexReceipt: Sendable, Equatable {
    public let indexed: Bool
    public let messageId: Int64
    public let tantivyDocs: UInt64
    public let fts5Enabled: Bool
}

public enum SearchIndexError: Error, Equatable, LocalizedError, Sendable {
    case schemaCorrupted(missingTables: [String])
    case journalModeNotWal
    case transactionRolledBack(String)
    case diskFull
    case hostNotInitialized

    public var errorDescription: String? {
        switch self {
        case .schemaCorrupted(let tables):
            return "Missing tables: \(tables.joined(separator: ", "))"
        case .journalModeNotWal:
            return "SQLite WAL checkpoint reported non-WAL or busy state"
        case .transactionRolledBack(let msg):
            return "Transaction rolled back: \(msg)"
        case .diskFull:
            return "Disk space exhausted (ENOSPC)"
        case .hostNotInitialized:
            return "Fabushi Rust host is not initialized"
        }
    }
}

// MARK: - Internal Rust RPC DTOs (Exact source/host/src/handlers_messaging.rs:1615-1669)

private struct IndexMessageParams: Encodable, Sendable {
    let message_id: Int64
    let chat_id: Int64
    let sender_id: UInt64
    let timestamp_ms: Int64
    let body: String
}

private struct IndexMessageResponse: Decodable, Sendable {
    let indexed: Bool
    let message_id: Int64
    let tantivy_docs: UInt64
    let fts5_enabled: Bool
}

private struct LocalSearchParams: Encodable, Sendable {
    let query: String
    let chat_id: Int64?
    let limit: UInt64
}

private struct LocalSearchHitDTO: Decodable, Sendable {
    let message_id: Int64
    let chat_id: Int64
    let sender_id: UInt64?
    let timestamp_ms: Int64?
    let body: String
    let score: Double
    let engine: String
}

private struct LocalSearchResponse: Decodable, Sendable {
    let engine: String
    let query: String
    let total: Int
    let hits: [LocalSearchHitDTO]
}

private struct RebuildIndexResponse: Decodable, Sendable {
    let rebuilt: Bool
    let indexed_docs: Int
}

// MARK: - Production Search Indexing Bridge Actor

public actor SearchIndexingBridge {
    public static let shared = SearchIndexingBridge(host: .shared)

    private let host: FabushiRustHostActor
    private var pendingBatch: [SearchDocument] = []

    public init(host: FabushiRustHostActor = .shared) {
        self.host = host
    }

    /// Verifies live SQLite WAL checkpoint health (`settings.checkpointWal`) and
    /// Tantivy/FTS5 index rebuildability (`search.rebuildLocalIndex`) on the real Rust host.
    public func verifySchemaIntegrity() async throws -> Bool {
        do {
            let wal = try await host.checkpointWAL()
            guard wal.checkpointed && wal.busy == 0 else {
                throw SearchIndexError.journalModeNotWal
            }
            let rebuild: RebuildIndexResponse = try await host.dispatch(
                method: FabushiRPCNamespace.search.method("rebuildLocalIndex")
            )
            guard rebuild.rebuilt else {
                throw SearchIndexError.transactionRolledBack("search.rebuildLocalIndex returned false")
            }
            return true
        } catch FabushiHostError.notInitialized {
            throw SearchIndexError.hostNotInitialized
        } catch let searchErr as SearchIndexError {
            throw searchErr
        } catch {
            throw SearchIndexError.transactionRolledBack(error.localizedDescription)
        }
    }

    @discardableResult
    public func indexMessage(doc: SearchDocument) async throws -> SearchIndexReceipt {
        let params = IndexMessageParams(
            message_id: doc.id,
            chat_id: doc.chatId,
            sender_id: UInt64(max(0, doc.senderId)),
            timestamp_ms: Int64(doc.timestamp.timeIntervalSince1970 * 1000),
            body: doc.text
        )
        do {
            let resp: IndexMessageResponse = try await host.dispatch(
                method: FabushiRPCNamespace.search.method("indexMessage"),
                params: params
            )
            return SearchIndexReceipt(
                indexed: resp.indexed,
                messageId: resp.message_id,
                tantivyDocs: resp.tantivy_docs,
                fts5Enabled: resp.fts5_enabled
            )
        } catch FabushiHostError.notInitialized {
            throw SearchIndexError.hostNotInitialized
        } catch {
            throw SearchIndexError.transactionRolledBack(error.localizedDescription)
        }
    }

    public func batchIndex(docs: [SearchDocument]) async throws {
        pendingBatch.append(contentsOf: docs)
    }

    public func commit() async throws {
        let batch = pendingBatch
        pendingBatch.removeAll()
        for doc in batch {
            _ = try await indexMessage(doc: doc)
        }
    }

    public func rollbackPending() {
        pendingBatch.removeAll()
    }

    @discardableResult
    public func rebuildIndex() async throws -> Int {
        do {
            let resp: RebuildIndexResponse = try await host.dispatch(
                method: FabushiRPCNamespace.search.method("rebuildLocalIndex")
            )
            return resp.indexed_docs
        } catch FabushiHostError.notInitialized {
            throw SearchIndexError.hostNotInitialized
        } catch {
            throw SearchIndexError.transactionRolledBack(error.localizedDescription)
        }
    }

    public func search(query: SearchQueryParams) async throws -> [SearchResultItem] {
        let trimmed = query.query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return [] }

        let params = LocalSearchParams(
            query: trimmed,
            chat_id: query.chatId,
            limit: UInt64(max(1, query.limit))
        )

        do {
            let resp: LocalSearchResponse = try await host.dispatch(
                method: FabushiRPCNamespace.search.method("localMessages"),
                params: params
            )
            return resp.hits.map { hit in
                SearchResultItem(
                    id: hit.message_id,
                    chatId: hit.chat_id,
                    snippetWithHighlights: Self.highlightSnippet(text: hit.body, query: trimmed),
                    score: Float(hit.score),
                    engine: hit.engine
                )
            }
        } catch FabushiHostError.notInitialized {
            throw SearchIndexError.hostNotInitialized
        } catch {
            throw SearchIndexError.transactionRolledBack(error.localizedDescription)
        }
    }

    public func totalIndexedCount() async throws -> Int {
        try await rebuildIndex()
    }

    // MARK: - Highlight Formatting Over Real Rust Search Hits

    private static func highlightSnippet(text: String, query: String) -> String {
        let tokens = query
            .components(separatedBy: CharacterSet.whitespacesAndNewlines)
            .filter { !$0.isEmpty }
        guard !tokens.isEmpty else { return text }

        var snippet = text
        for token in tokens {
            let escaped = NSRegularExpression.escapedPattern(for: token)
            let isAsciiWord = token.unicodeScalars.allSatisfy { CharacterSet.alphanumerics.contains($0) && $0.isASCII }
            let pattern = isAsciiWord ? "(?i)\\b(\(escaped))\\b" : "(?i)(\(escaped))"
            if let regex = try? NSRegularExpression(pattern: pattern, options: []) {
                let range = NSRange(location: 0, length: snippet.utf16.count)
                snippet = regex.stringByReplacingMatches(
                    in: snippet,
                    options: [],
                    range: range,
                    withTemplate: "<mark>$1</mark>"
                )
            }
        }
        return snippet
    }
}
