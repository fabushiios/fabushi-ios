import Foundation

// MARK: - Honest Upstream Capability Boundaries (`BLK-08` / `D15-03`)

/// Documents verified upstream baseline capability boundaries for M3 (`Fabushi v1.6.15`, `2ca3fffd32c9`).
public enum FabushiMessagingCapabilityBoundaries: Sendable {
    public enum CommunityFoundationStatus: String, Sendable, Equatable {
        /// Upstream `v1.6.15` ships pure-Rust `gpui_telegram_community` foundation types only —
        /// zero server HTTP/WS routes, zero `TelegramClient` RPCs, and zero desktop Community UI.
        /// M3 strictly omits fabricated Community tabs, results, or `community.*` RPCs (`BLK-08`).
        case upstreamFoundation = "UPSTREAM-FOUNDATION"
    }

    public static let communityFoundationStatus: CommunityFoundationStatus = .upstreamFoundation
    public static let communitySearchSupported: Bool = false
    public static let e2eeServerPrekeyBoundaryPartial: Bool = true
}

// MARK: - Module-Local Bilingual Catalog (`en` + `zh-Hans`)

/// Module-local bilingual localization table for Milestone M3 (Messaging, Composer, Inspector,
/// Media & Contacts). `Localizable.xcstrings` is frozen during M3 and consolidated in M6.
///
/// Invariants enforced by `FabushiMessagingM3aStateAndSyncTests`:
/// - Every key defines distinct non-empty `en` and `zh-Hans` values.
/// - Every `en` value contains ASCII letters and zero Han characters.
/// - Every `zh-Hans` value contains Han characters.
/// - Zero raw key prefixes (`FabushiAccessibilityIDs.forbiddenRawLocalizationKeyPrefixes`).
/// - Zero synthetic placeholder chat titles.
public enum FabushiMessagingStrings: Sendable {
    public enum Key: String, CaseIterable, Sendable {
        // Chat List & Folders
        case chatsTitle
        case personalNotes
        case allFolder
        case unreadFolder
        case botsFolder
        case newFolder
        case editFolder
        case deleteFolder
        case folderNamePlaceholder
        case includeGroups
        case includeBots
        case includeChannels
        case excludeMuted
        case excludeRead
        case excludeArchived
        case archivedChats
        case archiveAction
        case unarchiveAction
        case pinAction
        case unpinAction
        case muteAction
        case unmuteAction
        case markReadAction
        case deleteChatAction
        case clearHistoryAction
        case leaveGroupAction
        case forumTopicsTitle
        case newTopicAction
        case topicTitlePlaceholder
        case topicClosedBadge
        case noTopicsYet
        case draftPrefix
        case unreadCountFormat
        case pinnedBadge
        case mutedBadge
        case channelBadge
        case groupBadge
        case secretChatBadge
        case typingIndicator
        case botThinkingIndicator
        case botRespondingIndicator
        case syncingMessagesPill
        case updatingConversations
        case offlinePreservedBanner
        case loadingChatsAccessibility
        case loadingMessagesAccessibility

        // Outbox & Sync
        case outboxPendingBannerFormat
        case outboxFailedBannerFormat
        case outboxRetryAllAction
        case outboxRetryItemAction
        case outboxDismissItemAction
        case outboxPendingStatus
        case outboxSendingStatus
        case outboxDeliveredStatus
        case outboxFailedStatus
        case syncGapRecovering
        case syncUpToDate

        // Multi-Select Compose (`#9`, `D10-06`)
        case composeTitle
        case composeToLabel
        case composeSearchPlaceholder
        case composeGroupTitlePlaceholder
        case composeCreateDirectAction
        case composeCreateGroupAction
        case composeCreateFailed
        case composeHumansSection
        case composeBotsSection
        case composeUsernameLookupSection
        case composeNoCandidates
        case composeResolveUsernameAction
        case composeRemoveRecipientHint
        case composeSelectedCountFormat

        // Contacts, Blocklist, Send-As & Prestart Bot (`#11`, `D10-08`)
        case contactsTitle
        case contactsSearchPlaceholder
        case addContactTitle
        case contactFirstNamePlaceholder
        case contactLastNamePlaceholder
        case contactUsernamePlaceholder
        case contactPhonePlaceholder
        case contactSaveAction
        case contactDeleteAction
        case contactAddedToast
        case contactAddFailedToast
        case contactDeletedToast
        case contactDeleteFailedToast
        case resolveUsernameTitle
        case resolveUsernameNotFound
        case startChatAction
        case blockedUsersTitle
        case blockedUsersEmpty
        case blockUserAction
        case unblockUserAction
        case userBlockedToast
        case userUnblockedToast
        case blockActionFailedToast
        case sendAsTitle
        case sendAsDefaultBadge
        case prestartBotHeadline
        case prestartBotFallbackDescription
        case prestartBotStartAction
        case prestartBotOpenMiniAppAction
        case prestartBotStartedToast
        case prestartBotStartFailedToast

        // Universal Search (`#18`, `BLK-08`)
        case searchTitle
        case searchPlaceholder
        case searchInChatPlaceholder
        case searchScopeAll
        case searchScopeChats
        case searchScopeMessages
        case searchScopeMedia
        case searchScopeLinks
        case searchScopeFiles
        case searchScopeBots
        case searchScopeMiniApps
        case searchOfflineIndexBanner
        case searchEmptyPromptTitle
        case searchEmptyPromptDescription
        case searchNoResultsTitle
        case searchNoResultsDescription
        case searchResultsCountFormat
        case searchJumpToMessage

        // Transcript, Stage Actions & Shared M3b Keys (`#13`..`#20`)
        case messageSendFailedToast
        case messageEditedBadge
        case messageDeletedTombstone
        case messageReplyAction
        case messageEditAction
        case messageDeleteAction
        case messageDeleteForEveryone
        case messageDeleteForMe
        case messageForwardAction
        case messagePinAction
        case messageUnpinAction
        case messageCopyAction
        case messageCopiedPill
        case messagePinnedPill
        case messageUnpinnedPill
        case messageSelectAction
        case multiSelectCountFormat
        case multiSelectForwardAction
        case multiSelectDeleteAction
        case composerPlaceholder
        case composerReplyPrefixFormat
        case composerEditingLabel
        case composerSendAction
        case composerSendSilently
        case composerScheduleMessage
        case composerAttachAction
        case composerVoiceRecordHint
        case composerSlideToCancel
        case composerRecordingLocked
        case composerDraftSaved
        case slashCommandsTitle
        case slashDistillCardTitle
        case contextBadgesTitle
        case addContextBadgeAction
        case ttlTitle
        case ttlOff
        case ttlOneDay
        case ttlOneWeek
        case ttlOneMonth
        case ttlUpdatedToast
        case ttlUpdateFailedToast
        case downloadFailedToastFormat
        case inspectorMembersTab
        case inspectorMediaTab
        case inspectorFilesTab
        case inspectorLinksTab
        case inspectorVoiceTab
        case inspectorBotConfigTitle
        case inspectorBotComputersTitle
        case inspectorFollowSettingsOption
        case inspectorModelSettingsAction
        case inspectorModelMismatchWarning
        case inspectorNoProviderWarning
        case inspectorCopyUsernameAction
        case inspectorUsernameCopiedPill
        case inspectorLeaveConfirmTitle
        case inspectorLeaveConfirmMessage
        case inspectorExportChatAction
        case inspectorStatisticsAction
        case e2eeTitle
        case e2eeVerifiedStatus
        case e2eeUnverifiedStatus
        case e2eeVerifyAction
        case e2eeSafetyNumbersTitle
        case e2eeSafetyNumbersHint
        case e2eePeerNotEnrolledNotice
        case storiesTitle
        case storiesCreateAction
        case storiesCaptionPlaceholder
        case storiesEmptyTitle
        case mediaLightboxTitle
        case mediaSaveAction
        case mediaShareAction
        case photoEditorTitle
        case stickerPickerTitle
        case gifSearchPlaceholder
        case instantViewTitle
        case instantViewShareAction
        case browserTakeoverAction
        case browserResumeAction
        case browserCancelAction
        case browserTakeoverUnavailableToast
        case inlineCallbackPendingNotice
        case floorInterruptAction
        case toolApproveAction
        case toolDenyAction

        // Additional M3b Stage, Cards, Composer, Inspector, E2EE & Media Copy
        case jumpToBottomAction
        case pinnedMessagesSheetTitle
        case forwardPickerTitle
        case forwardSentToastFormat
        case clearHistoryConfirmTitle
        case clearHistoryConfirmMessage
        case clearHistoryDoneToast
        case exportHistoryDoneToast
        case exportHistoryFailedToast
        case chatStatisticsTitle
        case statsMembersLabel
        case statsTotalMessagesLabel
        case statsLast7DaysLabel
        case statsReactionsLabel
        case statsActivePostersLabel
        case statsGrowthRateLabel
        case formatBoldAction
        case formatItalicAction
        case formatCodeAction
        case formatStrikethroughAction
        case formatSpoilerAction
        case formatLinkAction
        case spoilerTapToReveal
        case copyCodeAction
        case voiceTranscribeAction
        case voiceTranscriptTitle
        case voiceSpeedFormat
        case attachPhotoVideoAction
        case attachDocumentAction
        case attachCameraAction
        case attachLocationAction
        case attachPollAction
        case attachTodoAction
        case cameraUnavailableToast
        case locationCardTitle
        case openInMapsAction
        case locationShareAction
        case pollCreatorTitle
        case pollQuestionPlaceholder
        case pollOptionPlaceholderFormat
        case pollAddOptionAction
        case pollAnonymousToggle
        case pollMultipleChoiceToggle
        case pollQuizToggle
        case pollCreateAction
        case pollVotesCountFormat
        case pollClosedBadge
        case pollCloseAction
        case pollQuizCorrectLabel
        case todoCreatorTitle
        case todoTitlePlaceholder
        case todoItemPlaceholderFormat
        case todoAddItemAction
        case todoCreateAction
        case callIncomingLabel
        case callOutgoingLabel
        case callMissedLabel
        case callBackAction
        case invoicePaidBadge
        case invoicePendingBadge
        case storyReplyBadge
        case computerCardOpenAction
        case agentReasoningTitle
        case scheduleSendTitle
        case scheduleSendAction
        case sendAsBannerFormat
        case slashCategorySession
        case slashCategoryModel
        case slashCategoryTools
        case slashCategoryMemory
        case slashCategoryWorkflow
        case slashCategorySafety
        case slashCategoryInspection
        case slashCategoryModes
        case slashNoMatches
        case contextBadgeValuePlaceholder
        case inspectorActionMute
        case inspectorActionUnmute
        case inspectorActionSearch
        case inspectorActionCall
        case inspectorActionMore
        case inspectorProfileBioLabel
        case inspectorEditInfoTitle
        case inspectorChatTitlePlaceholder
        case inspectorChatAboutPlaceholder
        case inspectorAddMemberAction
        case inspectorAddMemberPlaceholder
        case inspectorInviteLinksTitle
        case inspectorCreateInviteAction
        case inspectorRevokeInviteAction
        case inspectorNoSharedMedia
        case inspectorNoMembers
        case inspectorActiveModelCaptionFormat
        case inspectorOverrideModelLabel
        case inspectorThinkingLevelLabel
        case inspectorApprovalModeLabel
        case inspectorPersonaPromptLabel
        case inspectorBindComputerLabel
        case inspectorUnboundComputerOption
        case inspectorSaveChangesAction
        case inspectorSavedChangesToast
        case inspectorSaveFailedToast
        case e2eeStartEncryptedNotice
        case e2eeCopyFingerprintAction
        case e2eeLocalIdentityLabel
        case e2eeRemoteIdentityLabel
        case e2eeEncryptedMessageLabel
        case photoEditorRotateAction
        case photoEditorCropAction
        case photoEditorClearMarkupAction
        case photoEditorApplyAction
        case mediaSavedToast
        case stickerTabEmoji
        case stickerTabStickers
        case stickerTabGifs
        case stickerInstallAction
        case stickerInstalledBadge
        case storiesComposerTitle
        case storiesPrivacyLabel
        case storiesPublishAction
        case storiesReplyPlaceholder
        case storiesViewsCountFormat
        case instantViewOpenInSafariAction
        case instantViewTextSizeLabel
    }

    public static func localized(_ key: Key, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return englishTable(for: key)
        case .zhHans:
            return simplifiedChineseTable(for: key)
        }
    }

    public static func format(
        _ key: Key,
        locale: FabushiLocale,
        _ arguments: CVarArg...
    ) -> String {
        let template = localized(key, locale: locale)
        return String(format: template, locale: locale.foundationLocale, arguments: arguments)
    }

    private static func englishTable(for key: Key) -> String {
        switch key {
        // Chat List & Folders
        case .chatsTitle: return "Chats"
        case .personalNotes: return "Personal Notes"
        case .allFolder: return "All"
        case .unreadFolder: return "Unread"
        case .botsFolder: return "Bots"
        case .newFolder: return "New Folder"
        case .editFolder: return "Edit Folder"
        case .deleteFolder: return "Delete Folder"
        case .folderNamePlaceholder: return "Folder Name"
        case .includeGroups: return "Include Groups"
        case .includeBots: return "Include Bots"
        case .includeChannels: return "Include Channels"
        case .excludeMuted: return "Exclude Muted"
        case .excludeRead: return "Exclude Read"
        case .excludeArchived: return "Exclude Archived"
        case .archivedChats: return "Archived Chats"
        case .archiveAction: return "Archive"
        case .unarchiveAction: return "Unarchive"
        case .pinAction: return "Pin"
        case .unpinAction: return "Unpin"
        case .muteAction: return "Mute"
        case .unmuteAction: return "Unmute"
        case .markReadAction: return "Mark Read"
        case .deleteChatAction: return "Delete"
        case .clearHistoryAction: return "Clear History"
        case .leaveGroupAction: return "Leave Group"
        case .forumTopicsTitle: return "Topics"
        case .newTopicAction: return "New Topic"
        case .topicTitlePlaceholder: return "Topic Title"
        case .topicClosedBadge: return "Closed"
        case .noTopicsYet: return "No topics yet"
        case .draftPrefix: return "Draft:"
        case .unreadCountFormat: return "%d unread"
        case .pinnedBadge: return "Pinned"
        case .mutedBadge: return "Muted"
        case .channelBadge: return "Channel"
        case .groupBadge: return "Group"
        case .secretChatBadge: return "Encrypted"
        case .typingIndicator: return "typing..."
        case .botThinkingIndicator: return "Thinking..."
        case .botRespondingIndicator: return "Responding..."
        case .syncingMessagesPill: return "Updating messages..."
        case .updatingConversations: return "Updating conversations..."
        case .offlinePreservedBanner: return "Offline mode: showing cached conversations"
        case .loadingChatsAccessibility: return "Loading conversations"
        case .loadingMessagesAccessibility: return "Loading messages"

        // Outbox & Sync
        case .outboxPendingBannerFormat: return "%d queued for delivery"
        case .outboxFailedBannerFormat: return "%d failed to send"
        case .outboxRetryAllAction: return "Retry All"
        case .outboxRetryItemAction: return "Retry"
        case .outboxDismissItemAction: return "Dismiss"
        case .outboxPendingStatus: return "Queued"
        case .outboxSendingStatus: return "Sending"
        case .outboxDeliveredStatus: return "Sent"
        case .outboxFailedStatus: return "Failed"
        case .syncGapRecovering: return "Catching up missed updates..."
        case .syncUpToDate: return "Synced"

        // Multi-Select Compose
        case .composeTitle: return "New Message"
        case .composeToLabel: return "To:"
        case .composeSearchPlaceholder: return "People, bots, or @username"
        case .composeGroupTitlePlaceholder: return "Group Name (optional)"
        case .composeCreateDirectAction: return "Start Chat"
        case .composeCreateGroupAction: return "Create Group"
        case .composeCreateFailed: return "Could not create conversation"
        case .composeHumansSection: return "People"
        case .composeBotsSection: return "Bots"
        case .composeUsernameLookupSection: return "Username Lookup"
        case .composeNoCandidates: return "No matching people or bots"
        case .composeResolveUsernameAction: return "Look up username"
        case .composeRemoveRecipientHint: return "Double tap to remove recipient"
        case .composeSelectedCountFormat: return "%d selected"

        // Contacts, Blocklist, Send-As & Prestart Bot
        case .contactsTitle: return "Contacts"
        case .contactsSearchPlaceholder: return "Search contacts or @username"
        case .addContactTitle: return "New Contact"
        case .contactFirstNamePlaceholder: return "First Name"
        case .contactLastNamePlaceholder: return "Last Name (optional)"
        case .contactUsernamePlaceholder: return "Username (optional)"
        case .contactPhonePlaceholder: return "Phone (optional)"
        case .contactSaveAction: return "Save Contact"
        case .contactDeleteAction: return "Remove Contact"
        case .contactAddedToast: return "Contact added"
        case .contactAddFailedToast: return "Failed to add contact"
        case .contactDeletedToast: return "Contact removed"
        case .contactDeleteFailedToast: return "Failed to remove contact"
        case .resolveUsernameTitle: return "Global Lookup"
        case .resolveUsernameNotFound: return "No user found for that username"
        case .startChatAction: return "Message"
        case .blockedUsersTitle: return "Blocked Users"
        case .blockedUsersEmpty: return "No blocked users"
        case .blockUserAction: return "Block User"
        case .unblockUserAction: return "Unblock"
        case .userBlockedToast: return "User blocked"
        case .userUnblockedToast: return "User unblocked"
        case .blockActionFailedToast: return "Failed to update block status"
        case .sendAsTitle: return "Send As"
        case .sendAsDefaultBadge: return "Default"
        case .prestartBotHeadline: return "What can this bot do?"
        case .prestartBotFallbackDescription: return "Start a conversation to interact with this bot and its tools."
        case .prestartBotStartAction: return "Start"
        case .prestartBotOpenMiniAppAction: return "Open App"
        case .prestartBotStartedToast: return "Bot conversation started"
        case .prestartBotStartFailedToast: return "Failed to start bot"

        // Universal Search
        case .searchTitle: return "Search"
        case .searchPlaceholder: return "Search chats, messages, bots, mini apps"
        case .searchInChatPlaceholder: return "Search in this chat"
        case .searchScopeAll: return "All"
        case .searchScopeChats: return "Chats"
        case .searchScopeMessages: return "Messages"
        case .searchScopeMedia: return "Media"
        case .searchScopeLinks: return "Links"
        case .searchScopeFiles: return "Files"
        case .searchScopeBots: return "Bots"
        case .searchScopeMiniApps: return "Mini Apps"
        case .searchOfflineIndexBanner: return "Searching local offline index"
        case .searchEmptyPromptTitle: return "Search Fabushi"
        case .searchEmptyPromptDescription: return "Find conversations, messages, media, files, bots, and mini apps."
        case .searchNoResultsTitle: return "No Results"
        case .searchNoResultsDescription: return "Try another keyword or switch search category."
        case .searchResultsCountFormat: return "%d results"
        case .searchJumpToMessage: return "Jump to Message"

        // Transcript, Stage Actions & Shared M3b Keys
        case .messageSendFailedToast: return "Message send failed"
        case .messageEditedBadge: return "edited"
        case .messageDeletedTombstone: return "This message was deleted"
        case .messageReplyAction: return "Reply"
        case .messageEditAction: return "Edit"
        case .messageDeleteAction: return "Delete"
        case .messageDeleteForEveryone: return "Delete for Everyone"
        case .messageDeleteForMe: return "Delete for Me"
        case .messageForwardAction: return "Forward"
        case .messagePinAction: return "Pin Message"
        case .messageUnpinAction: return "Unpin Message"
        case .messageCopyAction: return "Copy"
        case .messageCopiedPill: return "Copied"
        case .messagePinnedPill: return "Pinned"
        case .messageUnpinnedPill: return "Unpinned"
        case .messageSelectAction: return "Select"
        case .multiSelectCountFormat: return "%d selected"
        case .multiSelectForwardAction: return "Forward"
        case .multiSelectDeleteAction: return "Delete"
        case .composerPlaceholder: return "Message"
        case .composerReplyPrefixFormat: return "Reply to %@"
        case .composerEditingLabel: return "Editing message"
        case .composerSendAction: return "Send"
        case .composerSendSilently: return "Send Without Sound"
        case .composerScheduleMessage: return "Schedule Message"
        case .composerAttachAction: return "Attach"
        case .composerVoiceRecordHint: return "Hold to record voice message"
        case .composerSlideToCancel: return "Slide left to cancel"
        case .composerRecordingLocked: return "Recording locked"
        case .composerDraftSaved: return "Draft"
        case .slashCommandsTitle: return "Commands"
        case .slashDistillCardTitle: return "Distilled Browser Script"
        case .contextBadgesTitle: return "Context"
        case .addContextBadgeAction: return "Add Context"
        case .ttlTitle: return "Auto-Delete Messages"
        case .ttlOff: return "Off"
        case .ttlOneDay: return "1 Day"
        case .ttlOneWeek: return "1 Week"
        case .ttlOneMonth: return "1 Month"
        case .ttlUpdatedToast: return "Auto-delete timer updated"
        case .ttlUpdateFailedToast: return "Failed to update auto-delete timer"
        case .downloadFailedToastFormat: return "Download failed: %@"
        case .inspectorMembersTab: return "Members"
        case .inspectorMediaTab: return "Media"
        case .inspectorFilesTab: return "Files"
        case .inspectorLinksTab: return "Links"
        case .inspectorVoiceTab: return "Voice"
        case .inspectorBotConfigTitle: return "Bot Configuration"
        case .inspectorBotComputersTitle: return "Bound Computers"
        case .inspectorFollowSettingsOption: return "Follow Settings"
        case .inspectorModelSettingsAction: return "Model Settings"
        case .inspectorModelMismatchWarning: return "Selected model differs from active configuration"
        case .inspectorNoProviderWarning: return "No AI provider configured in Model Settings"
        case .inspectorCopyUsernameAction: return "Copy Username"
        case .inspectorUsernameCopiedPill: return "Username copied"
        case .inspectorLeaveConfirmTitle: return "Leave Group?"
        case .inspectorLeaveConfirmMessage: return "You will no longer receive messages from this group."
        case .inspectorExportChatAction: return "Export Chat History"
        case .inspectorStatisticsAction: return "Statistics"
        case .e2eeTitle: return "Encryption"
        case .e2eeVerifiedStatus: return "Verified"
        case .e2eeUnverifiedStatus: return "Not Verified"
        case .e2eeVerifyAction: return "Mark as Verified"
        case .e2eeSafetyNumbersTitle: return "Safety Number"
        case .e2eeSafetyNumbersHint: return "Compare these 4 emojis or scan the QR code with your contact to verify end-to-end encryption."
        case .e2eePeerNotEnrolledNotice: return "This peer has not published E2EE prekeys on the server yet. Message was not sent in plaintext."
        case .storiesTitle: return "Stories"
        case .storiesCreateAction: return "Add Story"
        case .storiesCaptionPlaceholder: return "Add a caption"
        case .storiesEmptyTitle: return "No active stories"
        case .mediaLightboxTitle: return "Media Preview"
        case .mediaSaveAction: return "Save"
        case .mediaShareAction: return "Share"
        case .photoEditorTitle: return "Edit Photo"
        case .stickerPickerTitle: return "Stickers & GIFs"
        case .gifSearchPlaceholder: return "Search GIFs"
        case .instantViewTitle: return "Instant View"
        case .instantViewShareAction: return "Share to Chat"
        case .browserTakeoverAction: return "Take Over Browser"
        case .browserResumeAction: return "Resume Automation"
        case .browserCancelAction: return "Cancel Task"
        case .browserTakeoverUnavailableToast: return "No active browser session attached"
        case .inlineCallbackPendingNotice: return "Interactive bot callback requires live server relay"
        case .floorInterruptAction: return "Interrupt"
        case .toolApproveAction: return "Approve"
        case .toolDenyAction: return "Deny"

        // Additional M3b Stage, Cards, Composer, Inspector, E2EE & Media Copy
        case .jumpToBottomAction: return "Jump to Latest"
        case .pinnedMessagesSheetTitle: return "Pinned Messages"
        case .forwardPickerTitle: return "Forward To"
        case .forwardSentToastFormat: return "Forwarded %d messages"
        case .clearHistoryConfirmTitle: return "Clear Chat History?"
        case .clearHistoryConfirmMessage: return "Messages in this conversation will be cleared."
        case .clearHistoryDoneToast: return "Chat history cleared"
        case .exportHistoryDoneToast: return "Chat history exported"
        case .exportHistoryFailedToast: return "Failed to export chat history"
        case .chatStatisticsTitle: return "Chat Statistics"
        case .statsMembersLabel: return "Members"
        case .statsTotalMessagesLabel: return "Total Messages"
        case .statsLast7DaysLabel: return "Last 7 Days"
        case .statsReactionsLabel: return "Total Reactions"
        case .statsActivePostersLabel: return "Active Posters"
        case .statsGrowthRateLabel: return "Growth Rate"
        case .formatBoldAction: return "Bold"
        case .formatItalicAction: return "Italic"
        case .formatCodeAction: return "Code"
        case .formatStrikethroughAction: return "Strike"
        case .formatSpoilerAction: return "Spoiler"
        case .formatLinkAction: return "Link"
        case .spoilerTapToReveal: return "Tap to reveal spoiler"
        case .copyCodeAction: return "Copy Code"
        case .voiceTranscribeAction: return "Transcribe"
        case .voiceTranscriptTitle: return "On-Device Transcript"
        case .voiceSpeedFormat: return "%.1fx"
        case .attachPhotoVideoAction: return "Photo or Video"
        case .attachDocumentAction: return "File"
        case .attachCameraAction: return "Camera"
        case .attachLocationAction: return "Location"
        case .attachPollAction: return "Poll"
        case .attachTodoAction: return "Checklist"
        case .cameraUnavailableToast: return "Camera is unavailable on this device"
        case .locationCardTitle: return "Shared Location"
        case .openInMapsAction: return "Open in Maps"
        case .locationShareAction: return "Send Current Location"
        case .pollCreatorTitle: return "Create Poll"
        case .pollQuestionPlaceholder: return "Ask a question"
        case .pollOptionPlaceholderFormat: return "Option %d"
        case .pollAddOptionAction: return "Add Option"
        case .pollAnonymousToggle: return "Anonymous Voting"
        case .pollMultipleChoiceToggle: return "Multiple Answers"
        case .pollQuizToggle: return "Quiz Mode"
        case .pollCreateAction: return "Send Poll"
        case .pollVotesCountFormat: return "%d votes"
        case .pollClosedBadge: return "Final Results"
        case .pollCloseAction: return "Stop Poll"
        case .pollQuizCorrectLabel: return "Correct Answer"
        case .todoCreatorTitle: return "Create Checklist"
        case .todoTitlePlaceholder: return "Checklist title"
        case .todoItemPlaceholderFormat: return "Task %d"
        case .todoAddItemAction: return "Add Task"
        case .todoCreateAction: return "Send Checklist"
        case .callIncomingLabel: return "Incoming Call"
        case .callOutgoingLabel: return "Outgoing Call"
        case .callMissedLabel: return "Missed Call"
        case .callBackAction: return "Call Back"
        case .invoicePaidBadge: return "Paid"
        case .invoicePendingBadge: return "Pending"
        case .storyReplyBadge: return "Story Mention"
        case .computerCardOpenAction: return "Open Computer"
        case .agentReasoningTitle: return "Agent Activity"
        case .scheduleSendTitle: return "Schedule Message"
        case .scheduleSendAction: return "Schedule"
        case .sendAsBannerFormat: return "Sending as %@"
        case .slashCategorySession: return "Session"
        case .slashCategoryModel: return "Model & Provider"
        case .slashCategoryTools: return "Tools & Skills"
        case .slashCategoryMemory: return "Memory & Persona"
        case .slashCategoryWorkflow: return "Workflow & Automation"
        case .slashCategorySafety: return "Safety & Control"
        case .slashCategoryInspection: return "Inspection & Export"
        case .slashCategoryModes: return "Modes & UI"
        case .slashNoMatches: return "No matching commands"
        case .contextBadgeValuePlaceholder: return "Path, ref, or URL"
        case .inspectorActionMute: return "Mute"
        case .inspectorActionUnmute: return "Unmute"
        case .inspectorActionSearch: return "Search"
        case .inspectorActionCall: return "Call"
        case .inspectorActionMore: return "More"
        case .inspectorProfileBioLabel: return "About"
        case .inspectorEditInfoTitle: return "Edit Info"
        case .inspectorChatTitlePlaceholder: return "Conversation Name"
        case .inspectorChatAboutPlaceholder: return "Description"
        case .inspectorAddMemberAction: return "Add Member"
        case .inspectorAddMemberPlaceholder: return "User ID"
        case .inspectorInviteLinksTitle: return "Invite Links"
        case .inspectorCreateInviteAction: return "New Invite Link"
        case .inspectorRevokeInviteAction: return "Revoke"
        case .inspectorNoSharedMedia: return "Nothing shared here yet"
        case .inspectorNoMembers: return "No members loaded"
        case .inspectorActiveModelCaptionFormat: return "Active in Settings: %@"
        case .inspectorOverrideModelLabel: return "Model Override"
        case .inspectorThinkingLevelLabel: return "Thinking Level"
        case .inspectorApprovalModeLabel: return "Approval Mode"
        case .inspectorPersonaPromptLabel: return "Persona Prompt"
        case .inspectorBindComputerLabel: return "Computer Node"
        case .inspectorUnboundComputerOption: return "None (Cloud Only)"
        case .inspectorSaveChangesAction: return "Save Changes"
        case .inspectorSavedChangesToast: return "Configuration saved"
        case .inspectorSaveFailedToast: return "Failed to save configuration"
        case .e2eeStartEncryptedNotice: return "End-to-end encryption keys verified locally"
        case .e2eeCopyFingerprintAction: return "Copy Key Fingerprint"
        case .e2eeLocalIdentityLabel: return "This Device Key"
        case .e2eeRemoteIdentityLabel: return "Contact Key"
        case .e2eeEncryptedMessageLabel: return "End-to-End Encrypted Envelope"
        case .photoEditorRotateAction: return "Rotate"
        case .photoEditorCropAction: return "Crop"
        case .photoEditorClearMarkupAction: return "Clear"
        case .photoEditorApplyAction: return "Use Photo"
        case .mediaSavedToast: return "Saved to Photos"
        case .stickerTabEmoji: return "Emoji"
        case .stickerTabStickers: return "Stickers"
        case .stickerTabGifs: return "GIFs"
        case .stickerInstallAction: return "Add"
        case .stickerInstalledBadge: return "Added"
        case .storiesComposerTitle: return "New Story"
        case .storiesPrivacyLabel: return "Visibility"
        case .storiesPublishAction: return "Post Story"
        case .storiesReplyPlaceholder: return "Reply to story"
        case .storiesViewsCountFormat: return "%d views"
        case .instantViewOpenInSafariAction: return "Open in Safari"
        case .instantViewTextSizeLabel: return "Text Size"
        }
    }

    private static func simplifiedChineseTable(for key: Key) -> String {
        switch key {
        // Chat List & Folders
        case .chatsTitle: return "聊天"
        case .personalNotes: return "收藏夹"
        case .allFolder: return "全部"
        case .unreadFolder: return "未读"
        case .botsFolder: return "机器人"
        case .newFolder: return "新建分组"
        case .editFolder: return "编辑分组"
        case .deleteFolder: return "删除分组"
        case .folderNamePlaceholder: return "分组名称"
        case .includeGroups: return "包含群组"
        case .includeBots: return "包含机器人"
        case .includeChannels: return "包含频道"
        case .excludeMuted: return "排除静音对话"
        case .excludeRead: return "排除已读对话"
        case .excludeArchived: return "排除归档对话"
        case .archivedChats: return "已归档对话"
        case .archiveAction: return "归档"
        case .unarchiveAction: return "取消归档"
        case .pinAction: return "置顶"
        case .unpinAction: return "取消置顶"
        case .muteAction: return "静音"
        case .unmuteAction: return "取消静音"
        case .markReadAction: return "标记已读"
        case .deleteChatAction: return "删除"
        case .clearHistoryAction: return "清空记录"
        case .leaveGroupAction: return "退出群组"
        case .forumTopicsTitle: return "话题"
        case .newTopicAction: return "新建话题"
        case .topicTitlePlaceholder: return "话题标题"
        case .topicClosedBadge: return "已关闭"
        case .noTopicsYet: return "暂无话题"
        case .draftPrefix: return "草稿："
        case .unreadCountFormat: return "%d 条未读"
        case .pinnedBadge: return "已置顶"
        case .mutedBadge: return "已静音"
        case .channelBadge: return "频道"
        case .groupBadge: return "群组"
        case .secretChatBadge: return "已加密"
        case .typingIndicator: return "正在输入…"
        case .botThinkingIndicator: return "正在思考…"
        case .botRespondingIndicator: return "正在回复…"
        case .syncingMessagesPill: return "正在同步消息…"
        case .updatingConversations: return "正在更新对话…"
        case .offlinePreservedBanner: return "离线模式：正在显示本地缓存对话"
        case .loadingChatsAccessibility: return "正在加载对话列表"
        case .loadingMessagesAccessibility: return "正在加载消息记录"

        // Outbox & Sync
        case .outboxPendingBannerFormat: return "%d 条消息待发送"
        case .outboxFailedBannerFormat: return "%d 条消息发送失败"
        case .outboxRetryAllAction: return "全部重试"
        case .outboxRetryItemAction: return "重试"
        case .outboxDismissItemAction: return "移除"
        case .outboxPendingStatus: return "排队中"
        case .outboxSendingStatus: return "发送中"
        case .outboxDeliveredStatus: return "已发送"
        case .outboxFailedStatus: return "发送失败"
        case .syncGapRecovering: return "正在补齐漏收消息…"
        case .syncUpToDate: return "已同步"

        // Multi-Select Compose
        case .composeTitle: return "新建消息"
        case .composeToLabel: return "收件人："
        case .composeSearchPlaceholder: return "搜索联系人、机器人或 @用户名"
        case .composeGroupTitlePlaceholder: return "群组名称（可选）"
        case .composeCreateDirectAction: return "开始聊天"
        case .composeCreateGroupAction: return "创建群组"
        case .composeCreateFailed: return "创建对话失败"
        case .composeHumansSection: return "联系人"
        case .composeBotsSection: return "机器人"
        case .composeUsernameLookupSection: return "用户名查找"
        case .composeNoCandidates: return "未找到匹配的联系人或机器人"
        case .composeResolveUsernameAction: return "查找用户名"
        case .composeRemoveRecipientHint: return "轻点两下移除该成员"
        case .composeSelectedCountFormat: return "已选择 %d 位"

        // Contacts, Blocklist, Send-As & Prestart Bot
        case .contactsTitle: return "联系人"
        case .contactsSearchPlaceholder: return "搜索联系人或 @用户名"
        case .addContactTitle: return "添加联系人"
        case .contactFirstNamePlaceholder: return "名字"
        case .contactLastNamePlaceholder: return "姓氏（可选）"
        case .contactUsernamePlaceholder: return "用户名（可选）"
        case .contactPhonePlaceholder: return "手机号（可选）"
        case .contactSaveAction: return "保存联系人"
        case .contactDeleteAction: return "删除联系人"
        case .contactAddedToast: return "联系人已添加"
        case .contactAddFailedToast: return "添加联系人失败"
        case .contactDeletedToast: return "联系人已删除"
        case .contactDeleteFailedToast: return "删除联系人失败"
        case .resolveUsernameTitle: return "全局查找"
        case .resolveUsernameNotFound: return "未找到该用户名对应的用户"
        case .startChatAction: return "发消息"
        case .blockedUsersTitle: return "黑名单"
        case .blockedUsersEmpty: return "暂无已拉黑用户"
        case .blockUserAction: return "拉黑用户"
        case .unblockUserAction: return "取消拉黑"
        case .userBlockedToast: return "已拉黑该用户"
        case .userUnblockedToast: return "已取消拉黑"
        case .blockActionFailedToast: return "更新拉黑状态失败"
        case .sendAsTitle: return "发送身份"
        case .sendAsDefaultBadge: return "默认"
        case .prestartBotHeadline: return "这个机器人能做什么？"
        case .prestartBotFallbackDescription: return "点击下方“开始”按钮即可与该机器人对话并使用其能力。"
        case .prestartBotStartAction: return "开始"
        case .prestartBotOpenMiniAppAction: return "打开小程序"
        case .prestartBotStartedToast: return "已启用机器人对话"
        case .prestartBotStartFailedToast: return "启动机器人失败"

        // Universal Search
        case .searchTitle: return "搜索"
        case .searchPlaceholder: return "搜索聊天、消息、机器人与小程序"
        case .searchInChatPlaceholder: return "在当前对话中搜索"
        case .searchScopeAll: return "全部"
        case .searchScopeChats: return "聊天"
        case .searchScopeMessages: return "消息"
        case .searchScopeMedia: return "媒体"
        case .searchScopeLinks: return "链接"
        case .searchScopeFiles: return "文件"
        case .searchScopeBots: return "机器人"
        case .searchScopeMiniApps: return "小程序"
        case .searchOfflineIndexBanner: return "正在使用本地离线索引搜索"
        case .searchEmptyPromptTitle: return "搜索法布施"
        case .searchEmptyPromptDescription: return "查找对话、消息记录、媒体、文件、机器人与小程序。"
        case .searchNoResultsTitle: return "无搜索结果"
        case .searchNoResultsDescription: return "请尝试其他关键词或切换分类筛选。"
        case .searchResultsCountFormat: return "共 %d 条结果"
        case .searchJumpToMessage: return "定位到消息"

        // Transcript, Stage Actions & Shared M3b Keys
        case .messageSendFailedToast: return "消息发送失败"
        case .messageEditedBadge: return "已编辑"
        case .messageDeletedTombstone: return "此消息已撤回"
        case .messageReplyAction: return "回复"
        case .messageEditAction: return "编辑"
        case .messageDeleteAction: return "删除"
        case .messageDeleteForEveryone: return "为所有人撤回"
        case .messageDeleteForMe: return "仅对我删除"
        case .messageForwardAction: return "转发"
        case .messagePinAction: return "置顶消息"
        case .messageUnpinAction: return "取消置顶"
        case .messageCopyAction: return "复制"
        case .messageCopiedPill: return "已复制"
        case .messagePinnedPill: return "已置顶"
        case .messageUnpinnedPill: return "已取消置顶"
        case .messageSelectAction: return "多选"
        case .multiSelectCountFormat: return "已选 %d 条"
        case .multiSelectForwardAction: return "转发"
        case .multiSelectDeleteAction: return "删除"
        case .composerPlaceholder: return "输入消息"
        case .composerReplyPrefixFormat: return "回复 %@"
        case .composerEditingLabel: return "正在编辑消息"
        case .composerSendAction: return "发送"
        case .composerSendSilently: return "静音发送"
        case .composerScheduleMessage: return "定时发送"
        case .composerAttachAction: return "附件"
        case .composerVoiceRecordHint: return "按住录制语音消息"
        case .composerSlideToCancel: return "向左滑动取消"
        case .composerRecordingLocked: return "已锁定录音"
        case .composerDraftSaved: return "草稿"
        case .slashCommandsTitle: return "斜杠指令"
        case .slashDistillCardTitle: return "已提炼浏览器脚本"
        case .contextBadgesTitle: return "上下文引用"
        case .addContextBadgeAction: return "添加上下文"
        case .ttlTitle: return "自动销毁消息"
        case .ttlOff: return "关闭"
        case .ttlOneDay: return "1 天"
        case .ttlOneWeek: return "1 周"
        case .ttlOneMonth: return "1 个月"
        case .ttlUpdatedToast: return "已更新自动销毁时长"
        case .ttlUpdateFailedToast: return "更新自动销毁时长失败"
        case .downloadFailedToastFormat: return "下载失败：%@"
        case .inspectorMembersTab: return "成员"
        case .inspectorMediaTab: return "媒体"
        case .inspectorFilesTab: return "文件"
        case .inspectorLinksTab: return "链接"
        case .inspectorVoiceTab: return "语音"
        case .inspectorBotConfigTitle: return "机器人配置"
        case .inspectorBotComputersTitle: return "绑定电脑"
        case .inspectorFollowSettingsOption: return "跟随设置"
        case .inspectorModelSettingsAction: return "模型设置"
        case .inspectorModelMismatchWarning: return "当前模型与设置中的生效模型不一致"
        case .inspectorNoProviderWarning: return "尚未在模型设置中配置可用的 AI 供应商"
        case .inspectorCopyUsernameAction: return "复制用户名"
        case .inspectorUsernameCopiedPill: return "已复制用户名"
        case .inspectorLeaveConfirmTitle: return "确认退出群组？"
        case .inspectorLeaveConfirmMessage: return "退出后将不再接收此群组的新消息。"
        case .inspectorExportChatAction: return "导出聊天记录"
        case .inspectorStatisticsAction: return "数据统计"
        case .e2eeTitle: return "端到端加密"
        case .e2eeVerifiedStatus: return "已验证"
        case .e2eeUnverifiedStatus: return "未验证"
        case .e2eeVerifyAction: return "标记为已验证"
        case .e2eeSafetyNumbersTitle: return "安全码"
        case .e2eeSafetyNumbersHint: return "请与对方核对这 4 个表情符号或扫描二维码，以确认端到端加密密钥一致。"
        case .e2eePeerNotEnrolledNotice: return "对方尚未在服务端发布端到端加密预密钥，消息未降级为明文发送。"
        case .storiesTitle: return "动态"
        case .storiesCreateAction: return "发布动态"
        case .storiesCaptionPlaceholder: return "添加说明文字"
        case .storiesEmptyTitle: return "暂无动态"
        case .mediaLightboxTitle: return "媒体预览"
        case .mediaSaveAction: return "保存"
        case .mediaShareAction: return "分享"
        case .photoEditorTitle: return "编辑图片"
        case .stickerPickerTitle: return "贴纸与动图"
        case .gifSearchPlaceholder: return "搜索动图"
        case .instantViewTitle: return "即时阅读"
        case .instantViewShareAction: return "分享到聊天"
        case .browserTakeoverAction: return "接管浏览器"
        case .browserResumeAction: return "恢复自动化"
        case .browserCancelAction: return "取消任务"
        case .browserTakeoverUnavailableToast: return "当前没有已连接的浏览器会话"
        case .inlineCallbackPendingNotice: return "该机器人内联回调需要服务端实时中继支持"
        case .floorInterruptAction: return "打断发言"
        case .toolApproveAction: return "批准"
        case .toolDenyAction: return "拒绝"

        // Additional M3b Stage, Cards, Composer, Inspector, E2EE & Media Copy
        case .jumpToBottomAction: return "回到底部"
        case .pinnedMessagesSheetTitle: return "置顶消息"
        case .forwardPickerTitle: return "转发到"
        case .forwardSentToastFormat: return "已转发 %d 条消息"
        case .clearHistoryConfirmTitle: return "确认清空聊天记录？"
        case .clearHistoryConfirmMessage: return "此对话中的消息记录将被清空。"
        case .clearHistoryDoneToast: return "已清空聊天记录"
        case .exportHistoryDoneToast: return "聊天记录已导出"
        case .exportHistoryFailedToast: return "导出聊天记录失败"
        case .chatStatisticsTitle: return "对话统计"
        case .statsMembersLabel: return "成员数"
        case .statsTotalMessagesLabel: return "消息总数"
        case .statsLast7DaysLabel: return "近 7 天消息"
        case .statsReactionsLabel: return "表情回应数"
        case .statsActivePostersLabel: return "活跃发言者"
        case .statsGrowthRateLabel: return "增长率"
        case .formatBoldAction: return "加粗"
        case .formatItalicAction: return "斜体"
        case .formatCodeAction: return "代码"
        case .formatStrikethroughAction: return "删除线"
        case .formatSpoilerAction: return "隐藏剧透"
        case .formatLinkAction: return "链接"
        case .spoilerTapToReveal: return "轻点查看隐藏内容"
        case .copyCodeAction: return "复制代码"
        case .voiceTranscribeAction: return "转文字"
        case .voiceTranscriptTitle: return "本地语音转写"
        case .voiceSpeedFormat: return "%.1f倍"
        case .attachPhotoVideoAction: return "照片或视频"
        case .attachDocumentAction: return "文件"
        case .attachCameraAction: return "相机"
        case .attachLocationAction: return "位置"
        case .attachPollAction: return "投票"
        case .attachTodoAction: return "清单"
        case .cameraUnavailableToast: return "当前设备不支持相机"
        case .locationCardTitle: return "位置分享"
        case .openInMapsAction: return "在地图中打开"
        case .locationShareAction: return "发送当前位置"
        case .pollCreatorTitle: return "发起投票"
        case .pollQuestionPlaceholder: return "输入投票问题"
        case .pollOptionPlaceholderFormat: return "选项 %d"
        case .pollAddOptionAction: return "添加选项"
        case .pollAnonymousToggle: return "匿名投票"
        case .pollMultipleChoiceToggle: return "允许多选"
        case .pollQuizToggle: return "测验模式"
        case .pollCreateAction: return "发送投票"
        case .pollVotesCountFormat: return "%d 票"
        case .pollClosedBadge: return "最终结果"
        case .pollCloseAction: return "结束投票"
        case .pollQuizCorrectLabel: return "正确答案"
        case .todoCreatorTitle: return "创建清单"
        case .todoTitlePlaceholder: return "清单标题"
        case .todoItemPlaceholderFormat: return "事项 %d"
        case .todoAddItemAction: return "添加事项"
        case .todoCreateAction: return "发送清单"
        case .callIncomingLabel: return "呼入通话"
        case .callOutgoingLabel: return "呼出通话"
        case .callMissedLabel: return "未接通话"
        case .callBackAction: return "回拨"
        case .invoicePaidBadge: return "已支付"
        case .invoicePendingBadge: return "待支付"
        case .storyReplyBadge: return "动态提及"
        case .computerCardOpenAction: return "打开远程电脑"
        case .agentReasoningTitle: return "智能体动态"
        case .scheduleSendTitle: return "定时发送"
        case .scheduleSendAction: return "确认定时"
        case .sendAsBannerFormat: return "将以 %@ 身份发送"
        case .slashCategorySession: return "会话"
        case .slashCategoryModel: return "模型与供应商"
        case .slashCategoryTools: return "工具与技能"
        case .slashCategoryMemory: return "记忆与人设"
        case .slashCategoryWorkflow: return "工作流与自动化"
        case .slashCategorySafety: return "安全与审批"
        case .slashCategoryInspection: return "检查与导出"
        case .slashCategoryModes: return "模式与界面"
        case .slashNoMatches: return "未找到匹配的指令"
        case .contextBadgeValuePlaceholder: return "输入路径、提交号或网址"
        case .inspectorActionMute: return "静音"
        case .inspectorActionUnmute: return "取消静音"
        case .inspectorActionSearch: return "搜索"
        case .inspectorActionCall: return "通话"
        case .inspectorActionMore: return "更多"
        case .inspectorProfileBioLabel: return "简介"
        case .inspectorEditInfoTitle: return "编辑资料"
        case .inspectorChatTitlePlaceholder: return "对话名称"
        case .inspectorChatAboutPlaceholder: return "描述说明"
        case .inspectorAddMemberAction: return "添加成员"
        case .inspectorAddMemberPlaceholder: return "用户编号"
        case .inspectorInviteLinksTitle: return "邀请链接"
        case .inspectorCreateInviteAction: return "创建邀请链接"
        case .inspectorRevokeInviteAction: return "撤销"
        case .inspectorNoSharedMedia: return "暂无共享内容"
        case .inspectorNoMembers: return "暂无成员信息"
        case .inspectorActiveModelCaptionFormat: return "当前设置生效模型：%@"
        case .inspectorOverrideModelLabel: return "模型覆盖"
        case .inspectorThinkingLevelLabel: return "思考深度"
        case .inspectorApprovalModeLabel: return "审批模式"
        case .inspectorPersonaPromptLabel: return "人设提示词"
        case .inspectorBindComputerLabel: return "绑定电脑节点"
        case .inspectorUnboundComputerOption: return "不绑定（仅云端）"
        case .inspectorSaveChangesAction: return "保存更改"
        case .inspectorSavedChangesToast: return "配置已保存"
        case .inspectorSaveFailedToast: return "保存配置失败"
        case .e2eeStartEncryptedNotice: return "本地端到端加密密钥已校验"
        case .e2eeCopyFingerprintAction: return "复制密钥指纹"
        case .e2eeLocalIdentityLabel: return "本设备密钥"
        case .e2eeRemoteIdentityLabel: return "对方密钥"
        case .e2eeEncryptedMessageLabel: return "端到端加密消息信封"
        case .photoEditorRotateAction: return "旋转"
        case .photoEditorCropAction: return "裁剪"
        case .photoEditorClearMarkupAction: return "清除标注"
        case .photoEditorApplyAction: return "使用图片"
        case .mediaSavedToast: return "已保存到相册"
        case .stickerTabEmoji: return "表情"
        case .stickerTabStickers: return "贴纸"
        case .stickerTabGifs: return "动图"
        case .stickerInstallAction: return "添加"
        case .stickerInstalledBadge: return "已添加"
        case .storiesComposerTitle: return "发布新动态"
        case .storiesPrivacyLabel: return "可见范围"
        case .storiesPublishAction: return "发布"
        case .storiesReplyPlaceholder: return "回复这条动态"
        case .storiesViewsCountFormat: return "%d 次查看"
        case .instantViewOpenInSafariAction: return "在浏览器中打开"
        case .instantViewTextSizeLabel: return "字号"
        }
    }
}

public extension FabushiTheme {
    func messagingString(_ key: FabushiMessagingStrings.Key) -> String {
        FabushiMessagingStrings.localized(key, locale: locale)
    }

    func messagingFormat(_ key: FabushiMessagingStrings.Key, _ arguments: CVarArg...) -> String {
        let template = FabushiMessagingStrings.localized(key, locale: locale)
        return String(format: template, locale: locale.foundationLocale, arguments: arguments)
    }
}
