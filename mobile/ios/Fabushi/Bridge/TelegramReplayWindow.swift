//
//  TelegramReplayWindow.swift
//  Fabushi
//
//  Production 128-bit sliding replay window bridging iOS native client with Rust core C-ABI.
//

import Foundation
import os

public enum TelegramReplayWindowError: Error, Sendable, Equatable {
    case allocationFailed
}

private struct ReplayWindowPointerBox: @unchecked Sendable {
    var pointer: UnsafeMutableRawPointer?
}

public final class TelegramReplayWindow: Sendable {
    private let lock: OSAllocatedUnfairLock<ReplayWindowPointerBox>

    /// Authoritative protocol maximum packet size exported by Rust `telegram_proto_max_packet_size()`.
    public static var maxPacketSize: Int {
        Int(telegram_proto_max_packet_size())
    }

    public init() {
        guard let ptr = telegram_replay_window_create() else {
            fatalError("Failed to allocate telegram_replay_window via Rust C-ABI")
        }
        self.lock = OSAllocatedUnfairLock(initialState: ReplayWindowPointerBox(pointer: ptr))
    }

    public init(safe: Void) throws {
        guard let ptr = telegram_replay_window_create() else {
            throw TelegramReplayWindowError.allocationFailed
        }
        self.lock = OSAllocatedUnfairLock(initialState: ReplayWindowPointerBox(pointer: ptr))
    }

    deinit {
        let taken = lock.withLock { box -> ReplayWindowPointerBox in
            let p = box.pointer
            box.pointer = nil
            return ReplayWindowPointerBox(pointer: p)
        }
        if let raw = taken.pointer {
            telegram_replay_window_destroy(raw)
        }
    }

    @discardableResult
    public func checkAndAdd(seq: UInt64) -> Bool {
        lock.withLock { box in
            guard let raw = box.pointer else { return false }
            return telegram_replay_window_check_and_add(raw, seq)
        }
    }
}
