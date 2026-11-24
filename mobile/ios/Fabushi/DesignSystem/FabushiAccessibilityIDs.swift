import Foundation

/// Canonical production accessibility identifier catalog for Fabushi iOS (`v1.6.9`).
/// Matches the opaque-box E2E accessibility identifier contract across all 13 required
/// `fabushi.*` UI namespaces character-for-character (281 identifiers + 13 namespace literals).
public enum FabushiAccessibilityIDs: Sendable {
    public static let allRequiredNamespaces: [String] = [
        Auth.namespace,
        Shell.namespace,
        ChatList.namespace,
        Compose.namespace,
        Stage.namespace,
        Composer.namespace,
        Inspector.namespace,
        Studio.namespace,
        Magpie.namespace,
        Remote.namespace,
        MiniApp.namespace,
        Call.namespace,
        Settings.namespace,
    ]

    public static let forbiddenRawLocalizationKeyPrefixes: [String] = [
        "fabushi.",
        "auth.",
        "shell.",
        "chatlist.",
        "compose.",
        "stage.",
        "composer.",
        "inspector.",
        "studio.",
        "magpie.",
        "remote.",
        "miniapp.",
        "call.",
        "settings.",
    ]

    // MARK: - 1. fabushi.auth (28 IDs)
    public enum Auth: Sendable {
        public static let namespace = "fabushi.auth"
        public static let root = "fabushi.auth.root"
        public static let serverURLField = "fabushi.auth.serverURLField"
        public static let serverURLError = "fabushi.auth.serverURLError"
        public static let usernameField = "fabushi.auth.usernameField"
        public static let passwordField = "fabushi.auth.passwordField"
        public static let loginButton = "fabushi.auth.loginButton"
        public static let registerTabButton = "fabushi.auth.registerTabButton"
        public static let registerSubmitButton = "fabushi.auth.registerSubmitButton"
        public static let emailField = "fabushi.auth.emailField"
        public static let phoneField = "fabushi.auth.phoneField"
        public static let sendOTPButton = "fabushi.auth.sendOTPButton"
        public static let otpCodeField = "fabushi.auth.otpCodeField"
        public static let verifyOTPButton = "fabushi.auth.verifyOTPButton"
        public static let alipayLoginButton = "fabushi.auth.alipayLoginButton"
        public static let alipayQRView = "fabushi.auth.alipayQRView"
        public static let alipayStatusLabel = "fabushi.auth.alipayStatusLabel"
        public static let passkeyLoginButton = "fabushi.auth.passkeyLoginButton"
        public static let qrPairButton = "fabushi.auth.qrPairButton"
        public static let qrPairCanvas = "fabushi.auth.qrPairCanvas"
        public static let twoFactorPasswordField = "fabushi.auth.twoFactorPasswordField"
        public static let twoFactorVerifyButton = "fabushi.auth.twoFactorVerifyButton"
        public static let passcodeOverlay = "fabushi.auth.passcodeOverlay"
        public static let passcodeField = "fabushi.auth.passcodeField"
        public static let passcodeSubmitButton = "fabushi.auth.passcodeSubmitButton"
        public static let passcodeErrorBanner = "fabushi.auth.passcodeErrorBanner"
        public static let privacyBlurOverlay = "fabushi.auth.privacyBlurOverlay"
        public static let errorBanner = "fabushi.auth.errorBanner"
        public static let splitKeyBadge = "fabushi.auth.splitKeyBadge"

        public static let allIdentifiers: [String] = [
            root, serverURLField, serverURLError, usernameField, passwordField,
            loginButton, registerTabButton, registerSubmitButton, emailField,
            phoneField, sendOTPButton, otpCodeField, verifyOTPButton,
            alipayLoginButton, alipayQRView, alipayStatusLabel, passkeyLoginButton,
            qrPairButton, qrPairCanvas, twoFactorPasswordField, twoFactorVerifyButton,
            passcodeOverlay, passcodeField, passcodeSubmitButton, passcodeErrorBanner,
            privacyBlurOverlay, errorBanner, splitKeyBadge,
        ]
    }

    // MARK: - 2. fabushi.shell (28 IDs)
    public enum Shell: Sendable {
        public static let namespace = "fabushi.shell"
        public static let root = "fabushi.shell.root"
        public static let splitView = "fabushi.shell.splitView"
        public static let tabBar = "fabushi.shell.tabBar"
        public static let tabChats = "fabushi.shell.tab.chats"
        public static let tabContacts = "fabushi.shell.tab.contacts"
        public static let tabStudio = "fabushi.shell.tab.studio"
        public static let tabRemote = "fabushi.shell.tab.remote"
        public static let tabSettings = "fabushi.shell.tab.settings"
        public static let sidebarCollapsedRail = "fabushi.shell.sidebar.collapsedRail"
        public static let sidebarCollapseToggle = "fabushi.shell.sidebar.collapseToggle"
        public static let stageSlot = "fabushi.shell.stageSlot"
        public static let accountSwitcherButton = "fabushi.shell.accountSwitcherButton"
        public static let accountSwitcherSheet = "fabushi.shell.accountSwitcherSheet"
        public static let accountSwitcherCloseButton = "fabushi.shell.accountSwitcherCloseButton"
        public static let accountSwitcherFirstRow = "fabushi.shell.accountSwitcher.row.0"
        public static let accountSwitcherSecondRow = "fabushi.shell.accountSwitcher.row.1"
        public static let addAccountButton = "fabushi.shell.addAccountButton"
        public static let connectionStatusPill = "fabushi.shell.connectionStatusPill"
        public static let offlineUnavailableView = "fabushi.shell.offlineUnavailableView"
        public static let offlineRetryButton = "fabushi.shell.offlineRetryButton"
        public static let toastWindowBanner = "fabushi.shell.toast.windowBanner"
        public static let toastWindowBannerClose = "fabushi.shell.toast.windowBanner.close"
        public static let toastStagePill = "fabushi.shell.toast.stagePill"
        public static let providerErrorCard = "fabushi.shell.providerErrorCard"
        public static let providerErrorOpenAIProviders = "fabushi.shell.providerErrorCard.openAiProviders"
        public static let actionBusDeepLinkBanner = "fabushi.shell.actionBusDeepLinkBanner"
        public static let capabilityLedgerBanner = "fabushi.shell.capabilityLedgerBanner"
        public static let backButton = "fabushi.shell.backButton"

        public static let allIdentifiers: [String] = [
            root, splitView, tabBar, tabChats, tabContacts, tabStudio, tabRemote,
            tabSettings, sidebarCollapsedRail, sidebarCollapseToggle, stageSlot,
            accountSwitcherButton, accountSwitcherSheet, accountSwitcherCloseButton,
            accountSwitcherFirstRow, accountSwitcherSecondRow, addAccountButton,
            connectionStatusPill, offlineUnavailableView, offlineRetryButton,
            toastWindowBanner, toastWindowBannerClose, toastStagePill,
            providerErrorCard, providerErrorOpenAIProviders, actionBusDeepLinkBanner,
            capabilityLedgerBanner, backButton,
        ]
    }

    // MARK: - 3. fabushi.chatlist (26 IDs)
    public enum ChatList: Sendable {
        public static let namespace = "fabushi.chatlist"
        public static let root = "fabushi.chatlist.root"
        public static let list = "fabushi.chatlist.list"
        public static let emptyState = "fabushi.chatlist.emptyState"
        public static let composeButton = "fabushi.chatlist.composeButton"
        public static let searchField = "fabushi.chatlist.searchField"
        public static let searchResultsList = "fabushi.chatlist.searchResultsList"
        public static let folderBar = "fabushi.chatlist.folderBar"
        public static let folderAllTab = "fabushi.chatlist.folder.all"
        public static let folderUnreadTab = "fabushi.chatlist.folder.unread"
        public static let folderBotsTab = "fabushi.chatlist.folder.bots"
        public static let pinnedGrid = "fabushi.chatlist.pinnedGrid"
        public static let railList = "fabushi.chatlist.railList"
        public static let archiveRow = "fabushi.chatlist.archiveRow"
        public static let topicsList = "fabushi.chatlist.topicsList"
        public static let unreadBadge = "fabushi.chatlist.unreadBadge"
        public static let unreadMarkDot = "fabushi.chatlist.unreadMarkDot"
        public static let botAvatarBadge = "fabushi.chatlist.botAvatarBadge"
        public static let contactsSheetButton = "fabushi.chatlist.contactsSheetButton"
        public static let contactsList = "fabushi.chatlist.contactsList"
        public static let addContactButton = "fabushi.chatlist.addContactButton"
        public static let contactUsernameField = "fabushi.chatlist.contactUsernameField"
        public static let contactSaveButton = "fabushi.chatlist.contactSaveButton"
        public static let storiesStrip = "fabushi.chatlist.storiesStrip"
        public static let firstRow = "fabushi.chatlist.row.0"
        public static let secondRow = "fabushi.chatlist.row.1"
        public static let thirdRow = "fabushi.chatlist.row.2"

        public static let allIdentifiers: [String] = [
            root, list, emptyState, composeButton, searchField, searchResultsList,
            folderBar, folderAllTab, folderUnreadTab, folderBotsTab, pinnedGrid,
            railList, archiveRow, topicsList, unreadBadge, unreadMarkDot,
            botAvatarBadge, contactsSheetButton, contactsList, addContactButton,
            contactUsernameField, contactSaveButton, storiesStrip, firstRow,
            secondRow, thirdRow,
        ]
    }

    // MARK: - 4. fabushi.compose (10 IDs)
    public enum Compose: Sendable {
        public static let namespace = "fabushi.compose"
        public static let stage = "fabushi.compose.stage"
        public static let toField = "fabushi.compose.toField"
        public static let candidateList = "fabushi.compose.candidateList"
        public static let firstCandidate = "fabushi.compose.candidate.0"
        public static let secondCandidate = "fabushi.compose.candidate.1"
        public static let chipList = "fabushi.compose.chipList"
        public static let chipRemove = "fabushi.compose.chipRemove"
        public static let groupTitleField = "fabushi.compose.groupTitleField"
        public static let cancelButton = "fabushi.compose.cancelButton"
        public static let createButton = "fabushi.compose.createButton"

        public static let allIdentifiers: [String] = [
            stage, toField, candidateList, firstCandidate, secondCandidate,
            chipList, chipRemove, groupTitleField, cancelButton, createButton,
        ]
    }

    // MARK: - 5. fabushi.stage (36 IDs)
    public enum Stage: Sendable {
        public static let namespace = "fabushi.stage"
        public static let root = "fabushi.stage.root"
        public static let headerTitle = "fabushi.stage.headerTitle"
        public static let inspectorToggleButton = "fabushi.stage.inspectorToggleButton"
        public static let transcriptList = "fabushi.stage.transcriptList"
        public static let messageBubbleLatest = "fabushi.stage.messageBubble.latest"
        public static let messageStatusBadge = "fabushi.stage.messageStatusBadge"
        public static let messageContextMenu = "fabushi.stage.messageContextMenu"
        public static let editMessageAction = "fabushi.stage.action.edit"
        public static let deleteMessageAction = "fabushi.stage.action.delete"
        public static let forwardMessageAction = "fabushi.stage.action.forward"
        public static let reactMessageAction = "fabushi.stage.action.react"
        public static let pinMessageAction = "fabushi.stage.action.pin"
        public static let multiSelectToolbar = "fabushi.stage.multiSelectToolbar"
        public static let multiSelectForwardButton = "fabushi.stage.multiSelect.forward"
        public static let multiSelectDeleteButton = "fabushi.stage.multiSelect.delete"
        public static let pinnedBanner = "fabushi.stage.pinnedBanner"
        public static let botDescriptionCard = "fabushi.stage.botDescriptionCard"
        public static let botStartBar = "fabushi.stage.botStartBar"
        public static let botStartButton = "fabushi.stage.botStartButton"
        public static let toolApprovalDock = "fabushi.stage.toolApprovalDock"
        public static let toolApproveButton = "fabushi.stage.toolApproveButton"
        public static let toolDenyButton = "fabushi.stage.toolDenyButton"
        public static let streamingIndicator = "fabushi.stage.streamingIndicator"
        public static let floorControlBanner = "fabushi.stage.floorControlBanner"
        public static let floorInterruptButton = "fabushi.stage.floorInterruptButton"
        public static let codingDiffCard = "fabushi.stage.codingDiffCard"
        public static let codingCheckpointButton = "fabushi.stage.codingCheckpointButton"
        public static let e2eeVerifiedBadge = "fabushi.stage.e2eeVerifiedBadge"
        public static let instantViewCard = "fabushi.stage.instantViewCard"
        public static let mediaPlayerCard = "fabushi.stage.mediaPlayerCard"
        public static let waveformView = "fabushi.stage.waveformView"
        public static let searchBar = "fabushi.stage.searchBar"
        public static let callStartButton = "fabushi.stage.callStartButton"
        public static let talkModeButton = "fabushi.stage.talkModeButton"
        public static let errorUnavailableView = "fabushi.stage.errorUnavailableView"
        public static let errorRetryButton = "fabushi.stage.errorRetryButton"

        public static let allIdentifiers: [String] = [
            root, headerTitle, inspectorToggleButton, transcriptList,
            messageBubbleLatest, messageStatusBadge, messageContextMenu,
            editMessageAction, deleteMessageAction, forwardMessageAction,
            reactMessageAction, pinMessageAction, multiSelectToolbar,
            multiSelectForwardButton, multiSelectDeleteButton, pinnedBanner,
            botDescriptionCard, botStartBar, botStartButton, toolApprovalDock,
            toolApproveButton, toolDenyButton, streamingIndicator,
            floorControlBanner, floorInterruptButton, codingDiffCard,
            codingCheckpointButton, e2eeVerifiedBadge, instantViewCard,
            mediaPlayerCard, waveformView, searchBar, callStartButton,
            talkModeButton, errorUnavailableView, errorRetryButton,
        ]
    }

    // MARK: - 6. fabushi.composer (18 IDs)
    public enum Composer: Sendable {
        public static let namespace = "fabushi.composer"
        public static let root = "fabushi.composer.root"
        public static let inputWrap = "fabushi.composer.inputWrap"
        public static let textField = "fabushi.composer.textField"
        public static let sendButton = "fabushi.composer.sendButton"
        public static let attachButton = "fabushi.composer.attachButton"
        public static let voiceRecordButton = "fabushi.composer.voiceRecordButton"
        public static let stickerPickerButton = "fabushi.composer.stickerPickerButton"
        public static let stickerGrid = "fabushi.composer.stickerGrid"
        public static let sendAsButton = "fabushi.composer.sendAsButton"
        public static let botMenuButton = "fabushi.composer.botMenuButton"
        public static let botSlashButton = "fabushi.composer.botSlashButton"
        public static let botKeyboardToggle = "fabushi.composer.botKeyboardToggle"
        public static let botReplyKeyboard = "fabushi.composer.botReplyKeyboard"
        public static let slashCommandPopover = "fabushi.composer.slashCommandPopover"
        public static let slashCommandFirstItem = "fabushi.composer.slashCommand.0"
        public static let contextBadgeStrip = "fabushi.composer.contextBadgeStrip"
        public static let validationErrorBanner = "fabushi.composer.validationErrorBanner"
        public static let draftIndicator = "fabushi.composer.draftIndicator"

        public static let allIdentifiers: [String] = [
            root, inputWrap, textField, sendButton, attachButton,
            voiceRecordButton, stickerPickerButton, stickerGrid, sendAsButton,
            botMenuButton, botSlashButton, botKeyboardToggle, botReplyKeyboard,
            slashCommandPopover, slashCommandFirstItem, contextBadgeStrip,
            validationErrorBanner, draftIndicator,
        ]
    }

    // MARK: - 7. fabushi.inspector (22 IDs)
    public enum Inspector: Sendable {
        public static let namespace = "fabushi.inspector"
        public static let drawer = "fabushi.inspector.drawer"
        public static let closeButton = "fabushi.inspector.closeButton"
        public static let profileBotSection = "fabushi.inspector.profile.botSection"
        public static let profileGroupSection = "fabushi.inspector.profile.groupSection"
        public static let profileUserSection = "fabushi.inspector.profile.userSection"
        public static let openAppButton = "fabushi.inspector.profile.openAppButton"
        public static let startSecretChatButton = "fabushi.inspector.profile.startSecretChatButton"
        public static let blockContactButton = "fabushi.inspector.profile.blockContactButton"
        public static let leaveGroupButton = "fabushi.inspector.profile.leaveGroupButton"
        public static let leaveConfirmModal = "fabushi.inspector.profile.leaveConfirmModal"
        public static let leaveConfirmCancel = "fabushi.inspector.profile.leaveConfirmModal.cancel"
        public static let copyUsernameButton = "fabushi.inspector.profile.copyUsernameButton"
        public static let copiedFeedback = "fabushi.inspector.profile.copiedFeedback"
        public static let tabMembers = "fabushi.inspector.tab.members"
        public static let tabMedia = "fabushi.inspector.tab.media"
        public static let tabFiles = "fabushi.inspector.tab.files"
        public static let tabLinks = "fabushi.inspector.tab.links"
        public static let tabVoice = "fabushi.inspector.tab.voice"
        public static let e2eeSafetyNumberButton = "fabushi.inspector.e2eeSafetyNumberButton"
        public static let e2eeSafetyEmojiGrid = "fabushi.inspector.e2eeSafetyEmojiGrid"
        public static let codingHostProbeButton = "fabushi.inspector.codingHostProbeButton"
        public static let multiAgentWorkspaceBadge = "fabushi.inspector.multiAgentWorkspaceBadge"

        public static let allIdentifiers: [String] = [
            drawer, closeButton, profileBotSection, profileGroupSection,
            profileUserSection, openAppButton, startSecretChatButton,
            blockContactButton, leaveGroupButton, leaveConfirmModal,
            leaveConfirmCancel, copyUsernameButton, copiedFeedback, tabMembers,
            tabMedia, tabFiles, tabLinks, tabVoice, e2eeSafetyNumberButton,
            e2eeSafetyEmojiGrid, codingHostProbeButton, multiAgentWorkspaceBadge,
        ]
    }

    // MARK: - 8. fabushi.studio (17 IDs)
    public enum Studio: Sendable {
        public static let namespace = "fabushi.studio"
        public static let root = "fabushi.studio.root"
        public static let tabBar = "fabushi.studio.tabBar"
        public static let tabOverview = "fabushi.studio.tab.overview"
        public static let tabKanban = "fabushi.studio.tab.kanban"
        public static let tabSkills = "fabushi.studio.tab.skills"
        public static let tabMemory = "fabushi.studio.tab.memory"
        public static let tabCron = "fabushi.studio.tab.cron"
        public static let tabApprovals = "fabushi.studio.tab.approvals"
        public static let tabSessions = "fabushi.studio.tab.sessions"
        public static let toolsetList = "fabushi.studio.toolsetList"
        public static let toolsetToggleFirst = "fabushi.studio.toolset.toggle.0"
        public static let kanbanBoard = "fabushi.studio.kanbanBoard"
        public static let mcpGrantsButton = "fabushi.studio.mcpGrantsButton"
        public static let mcpAuditLogList = "fabushi.studio.mcpAuditLogList"
        public static let mcpApproveButton = "fabushi.studio.mcpApproveButton"
        public static let codingAgentsPanel = "fabushi.studio.codingAgentsPanel"
        public static let codingAgentCreatePRButton = "fabushi.studio.codingAgent.createPR"

        public static let allIdentifiers: [String] = [
            root, tabBar, tabOverview, tabKanban, tabSkills, tabMemory, tabCron,
            tabApprovals, tabSessions, toolsetList, toolsetToggleFirst,
            kanbanBoard, mcpGrantsButton, mcpAuditLogList, mcpApproveButton,
            codingAgentsPanel, codingAgentCreatePRButton,
        ]
    }

    // MARK: - 9. fabushi.magpie (9 IDs)
    public enum Magpie: Sendable {
        public static let namespace = "fabushi.magpie"
        public static let root = "fabushi.magpie.root"
        public static let providerList = "fabushi.magpie.providerList"
        public static let addProviderButton = "fabushi.magpie.addProviderButton"
        public static let routingRuleToggle = "fabushi.magpie.routingRuleToggle"
        public static let piiMaskingToggle = "fabushi.magpie.piiMaskingToggle"
        public static let circuitBreakerBadge = "fabushi.magpie.circuitBreakerBadge"
        public static let vaultOAuthSyncButton = "fabushi.magpie.vaultOAuthSyncButton"
        public static let probeHealthButton = "fabushi.magpie.probeHealthButton"
        public static let probeStatusLabel = "fabushi.magpie.probeStatusLabel"

        public static let allIdentifiers: [String] = [
            root, providerList, addProviderButton, routingRuleToggle,
            piiMaskingToggle, circuitBreakerBadge, vaultOAuthSyncButton,
            probeHealthButton, probeStatusLabel,
        ]
    }

    // MARK: - 10. fabushi.remote (16 IDs)
    public enum Remote: Sendable {
        public static let namespace = "fabushi.remote"
        public static let workbench = "fabushi.remote.workbench"
        public static let nodeList = "fabushi.remote.nodeList"
        public static let firstNodeRow = "fabushi.remote.node.0"
        public static let streamCanvas = "fabushi.remote.streamCanvas"
        public static let controlledBanner = "fabushi.remote.controlledBanner"
        public static let emergencyTakeoverButton = "fabushi.remote.emergencyTakeoverButton"
        public static let ptyTerminal = "fabushi.remote.ptyTerminal"
        public static let ptyInputField = "fabushi.remote.ptyInputField"
        public static let tailnetStatusBadge = "fabushi.remote.tailnetStatusBadge"
        public static let permissionModal = "fabushi.remote.permissionModal"
        public static let permissionModalScreenStatus = "fabushi.remote.permissionModal.screenStatus"
        public static let permissionModalAccessibilityStatus = "fabushi.remote.permissionModal.accessibilityStatus"
        public static let permissionModalRetryButton = "fabushi.remote.permissionModal.retryButton"
        public static let permissionModalDismissButton = "fabushi.remote.permissionModal.dismissButton"
        public static let localHostUnsupportedBanner = "fabushi.remote.localHostUnsupportedBanner"
        public static let macosDragHudForbidden = "fabushi.remote.macosDragHud"

        public static let allIdentifiers: [String] = [
            workbench, nodeList, firstNodeRow, streamCanvas, controlledBanner,
            emergencyTakeoverButton, ptyTerminal, ptyInputField,
            tailnetStatusBadge, permissionModal, permissionModalScreenStatus,
            permissionModalAccessibilityStatus, permissionModalRetryButton,
            permissionModalDismissButton, localHostUnsupportedBanner,
            macosDragHudForbidden,
        ]
    }

    // MARK: - 11. fabushi.miniapp (14 IDs)
    public enum MiniApp: Sendable {
        public static let namespace = "fabushi.miniapp"
        public static let root = "fabushi.miniapp.root"
        public static let launcherButton = "fabushi.miniapp.launcherButton"
        public static let catalogList = "fabushi.miniapp.catalogList"
        public static let globalDharmaCard = "fabushi.miniapp.card.globalDharma"
        public static let tosModal = "fabushi.miniapp.tosModal"
        public static let tosAcceptButton = "fabushi.miniapp.tosAcceptButton"
        public static let tosDeclineButton = "fabushi.miniapp.tosDeclineButton"
        public static let tosBlockedBanner = "fabushi.miniapp.tosBlockedBanner"
        public static let webView = "fabushi.miniapp.webView"
        public static let webMcpStatusBadge = "fabushi.miniapp.webMcpStatusBadge"
        public static let webMcpErrorBanner = "fabushi.miniapp.webMcpErrorBanner"
        public static let minimizePiPButton = "fabushi.miniapp.minimizePiPButton"
        public static let floatingPiPPill = "fabushi.miniapp.floatingPiPPill"
        public static let closeButton = "fabushi.miniapp.closeButton"

        public static let allIdentifiers: [String] = [
            root, launcherButton, catalogList, globalDharmaCard, tosModal,
            tosAcceptButton, tosDeclineButton, tosBlockedBanner, webView,
            webMcpStatusBadge, webMcpErrorBanner, minimizePiPButton,
            floatingPiPPill, closeButton,
        ]
    }

    // MARK: - 12. fabushi.call (14 IDs)
    public enum Call: Sendable {
        public static let namespace = "fabushi.call"
        public static let overlay = "fabushi.call.overlay"
        public static let muteButton = "fabushi.call.muteButton"
        public static let cameraButton = "fabushi.call.cameraButton"
        public static let screenShareButton = "fabushi.call.screenShareButton"
        public static let screenSharePicker = "fabushi.call.screenSharePicker"
        public static let hangupButton = "fabushi.call.hangupButton"
        public static let fingerprintBadge = "fabushi.call.fingerprintBadge"
        public static let permissionDeniedBanner = "fabushi.call.permissionDeniedBanner"
        public static let permissionDeniedOpenSettings = "fabushi.call.permissionDeniedBanner.openSettingsButton"
        public static let screenShareUnavailableBanner = "fabushi.call.screenShareUnavailableBanner"
        public static let sfuDisclosureBanner = "fabushi.call.sfuDisclosureBanner"
        public static let talkOrb = "fabushi.call.talkOrb"
        public static let talkBargeInButton = "fabushi.call.talkBargeInButton"
        public static let talkStopButton = "fabushi.call.talkStopButton"

        public static let allIdentifiers: [String] = [
            overlay, muteButton, cameraButton, screenShareButton,
            screenSharePicker, hangupButton, fingerprintBadge,
            permissionDeniedBanner, permissionDeniedOpenSettings,
            screenShareUnavailableBanner, sfuDisclosureBanner, talkOrb,
            talkBargeInButton, talkStopButton,
        ]
    }

    // MARK: - 13. fabushi.settings (43 IDs)
    public enum Settings: Sendable {
        public static let namespace = "fabushi.settings"
        public static let root = "fabushi.settings.root"
        public static let searchField = "fabushi.settings.searchField"
        public static let sectionList = "fabushi.settings.sectionList"
        public static let sectionAppearance = "fabushi.settings.section.appearance"
        public static let sectionLanguage = "fabushi.settings.section.language"
        public static let sectionPrivacySecurity = "fabushi.settings.section.privacySecurity"
        public static let sectionDevices = "fabushi.settings.section.devices"
        public static let sectionVault = "fabushi.settings.section.vault"
        public static let sectionDataStorage = "fabushi.settings.section.dataStorage"
        public static let sectionNetworkProxy = "fabushi.settings.section.networkProxy"
        public static let sectionAIProviders = "fabushi.settings.section.aiProviders"
        public static let sectionMCP = "fabushi.settings.section.mcp"
        public static let sectionWalletStars = "fabushi.settings.section.walletStars"
        public static let sectionNotifications = "fabushi.settings.section.notifications"
        public static let sectionUpdates = "fabushi.settings.section.updates"
        public static let sectionCapabilities = "fabushi.settings.section.capabilities"
        public static let themeNightButton = "fabushi.settings.theme.night"
        public static let themeTintedButton = "fabushi.settings.theme.tinted"
        public static let themeDayButton = "fabushi.settings.theme.day"
        public static let themeClassicButton = "fabushi.settings.theme.classic"
        public static let accentHexField = "fabushi.settings.accentHexField"
        public static let accentContrastBadge = "fabushi.settings.accentContrastBadge"
        public static let localeZhHansButton = "fabushi.settings.locale.zhHans"
        public static let localeEnButton = "fabushi.settings.locale.en"
        public static let reduceMotionToggle = "fabushi.settings.reduceMotionToggle"
        public static let highContrastToggle = "fabushi.settings.highContrastToggle"
        public static let storageMetricsLabel = "fabushi.settings.storageMetricsLabel"
        public static let storagePurgeCacheButton = "fabushi.settings.storagePurgeCacheButton"
        public static let proxyHostField = "fabushi.settings.proxyHostField"
        public static let proxyPortField = "fabushi.settings.proxyPortField"
        public static let proxyProbeButton = "fabushi.settings.proxyProbeButton"
        public static let proxyStatusLabel = "fabushi.settings.proxyStatusLabel"
        public static let starsBalanceLabel = "fabushi.settings.starsBalanceLabel"
        public static let storeKitRestoreButton = "fabushi.settings.storeKitRestoreButton"
        public static let storeKitDisclosureBanner = "fabushi.settings.storeKitDisclosureBanner"
        public static let vaultUnlockButton = "fabushi.settings.vaultUnlockButton"
        public static let vaultTotpCodeLabel = "fabushi.settings.vaultTotpCodeLabel"
        public static let devicesList = "fabushi.settings.devicesList"
        public static let revokeOtherDevicesButton = "fabushi.settings.revokeOtherDevicesButton"
        public static let capabilityLedgerList = "fabushi.settings.capabilityLedgerList"
        public static let updateCheckButton = "fabushi.settings.updateCheckButton"
        public static let updateStatusLabel = "fabushi.settings.updateStatusLabel"
        public static let apnsStatusLabel = "fabushi.settings.apnsStatusLabel"

        public static let allIdentifiers: [String] = [
            root, searchField, sectionList, sectionAppearance, sectionLanguage,
            sectionPrivacySecurity, sectionDevices, sectionVault,
            sectionDataStorage, sectionNetworkProxy, sectionAIProviders,
            sectionMCP, sectionWalletStars, sectionNotifications, sectionUpdates,
            sectionCapabilities, themeNightButton, themeTintedButton,
            themeDayButton, themeClassicButton, accentHexField,
            accentContrastBadge, localeZhHansButton, localeEnButton,
            reduceMotionToggle, highContrastToggle, storageMetricsLabel,
            storagePurgeCacheButton, proxyHostField, proxyPortField,
            proxyProbeButton, proxyStatusLabel, starsBalanceLabel,
            storeKitRestoreButton, storeKitDisclosureBanner, vaultUnlockButton,
            vaultTotpCodeLabel, devicesList, revokeOtherDevicesButton,
            capabilityLedgerList, updateCheckButton, updateStatusLabel,
            apnsStatusLabel,
        ]
    }

    public static let allIdentifiersByNamespace: [String: [String]] = [
        Auth.namespace: Auth.allIdentifiers,
        Shell.namespace: Shell.allIdentifiers,
        ChatList.namespace: ChatList.allIdentifiers,
        Compose.namespace: Compose.allIdentifiers,
        Stage.namespace: Stage.allIdentifiers,
        Composer.namespace: Composer.allIdentifiers,
        Inspector.namespace: Inspector.allIdentifiers,
        Studio.namespace: Studio.allIdentifiers,
        Magpie.namespace: Magpie.allIdentifiers,
        Remote.namespace: Remote.allIdentifiers,
        MiniApp.namespace: MiniApp.allIdentifiers,
        Call.namespace: Call.allIdentifiers,
        Settings.namespace: Settings.allIdentifiers,
    ]
}
