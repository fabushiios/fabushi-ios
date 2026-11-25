import Foundation

/// Module-local bilingual (`en` + `zh-Hans`) string catalog for M5 Platform surfaces
/// (Remote Computer, Mini Apps & 全球法布施, Built-in Browser & Automation App Store,
/// plus extensible helpers for M5b Calls/Settings/Economy/Proxy/Intents).
///
/// Kept outside frozen `Localizable.xcstrings` during parallel M3/M4/M5 execution;
/// M6 consolidates into the shared string catalog.
public enum FabushiPlatformStrings: Sendable {
    /// Inline bilingual selector for M5 extensions and parameterized messages.
    public static func text(_ locale: FabushiLocale, en: String, zhHans: String) -> String {
        switch locale {
        case .en:
            return en
        case .zhHans:
            return zhHans
        }
    }

    public enum Key: String, CaseIterable, Sendable {
        // MARK: - Shared Platform Actions
        case refresh
        case cancel
        case close
        case done
        case save
        case delete
        case copy
        case copied
        case retry
        case back
        case open
        case run
        case stop
        case install
        case uninstall
        case installed
        case publish
        case searchPlaceholder
        case loading
        case online
        case offline
        case enabled
        case disabled
        case builtInBrowserTitle
        case builtInBrowserSubtitle

        // MARK: - Remote Computer Hub & Workbench (Feature #27 + D10-11)
        case remoteHubTitle
        case remoteTabMyDevices
        case remoteTabRemoteAssist
        case remoteTabCloudPCs
        case remoteTabAiMcp
        case remoteAllowToggle
        case remoteAllowSubtitle
        case remoteDisconnectAllAction
        case remoteEmergencyTakeover
        case remoteControlledBannerTitle
        case remoteTailnetTitle
        case remoteTailnetDirectP2P
        case remoteTailnetDerpRelay
        case remoteTailnetHttpFallback
        case remoteTailnetOffline
        case remoteTailnetForceDerp
        case remoteTailnetStart
        case remoteTailnetStop
        case remoteTailnetRegister
        case remoteLocalHostUnsupportedTitle
        case remoteLocalHostUnsupportedBody
        case remoteCapabilityDiagnostics
        case remotePermissionModalTitle
        case remotePermissionScreenStatusTitle
        case remotePermissionScreenStatusValue
        case remotePermissionAccessibilityTitle
        case remotePermissionAccessibilityValue
        case remotePermissionRetry
        case remotePermissionDismiss
        case remoteThisPCBadge
        case remoteCloudBadge
        case remoteActionEnterDesktop
        case remoteActionShell
        case remoteActionSettings
        case remoteAddCloudPC
        case remoteAddCloudPCSubtitle
        case remoteQuickAssistTitle
        case remoteAssistIdLabel
        case remoteAccessCodeLabel
        case remoteResetCode
        case remoteSetCustomCode
        case remoteCopyInvite
        case remoteAssistOthersAction
        case remoteAssistTargetIdPlaceholder
        case remoteAssistAccessCodePlaceholder
        case remoteAssistConnectAction
        case remoteEmptyDevicesTitle
        case remoteEmptyDevicesSubtitle
        case remoteWorkbenchDesktopTab
        case remoteWorkbenchTerminalTab
        case remoteWorkbenchFilesTab
        case remoteWorkbenchSettingsTab
        case remoteWorkbenchCaptureFrame
        case remoteWorkbenchSendKey
        case remoteWorkbenchPrivacyScreen
        case remoteWorkbenchClipboardPull
        case remoteWorkbenchClipboardPush
        case remoteWorkbenchDisplaySwitch
        case remoteWorkbenchPowerLock
        case remoteWorkbenchPowerSleep
        case remoteWorkbenchPowerReboot
        case remoteWorkbenchWakeOnLan
        case remoteWorkbenchTestConnection
        case remotePtyNewSession
        case remotePtyInputPlaceholder
        case remotePtySendAction
        case remotePtyOutputPlaceholder
        case remotePendingApprovalsTitle
        case remoteNoPendingApprovals
        case remoteApprove
        case remoteDeny
        case remoteBotProfileTitle
        case remoteCloudFormTitle
        case remoteCloudNamePlaceholder
        case remoteCloudHostPlaceholder
        case remoteCloudPortPlaceholder
        case remoteCloudUsernamePlaceholder
        case remoteCloudSecretPlaceholder
        case remoteCloudWorkdirPlaceholder
        case remotePolicyTitle
        case remotePolicyAskEveryTime
        case remotePolicyAllowReadOnly
        case remotePolicyAlwaysAllow
        case remotePolicyDisabled
        case remotePolicySavedToast
        case remoteAssistAllowFailedPrefix
        case remoteDisconnectAllSuccess
        case remoteEmergencyTakeoverSuccess
        case remoteControlledPillTakeover

        // MARK: - Mini Apps, WebMCP, Floating PiP & 全球法布施 (Feature #28 + D10-12 + D12-01)
        case miniAppHubTitle
        case miniAppCatalogSearchPlaceholder
        case miniAppFilterAll
        case miniAppFilterInstalled
        case miniAppFilterRecent
        case miniAppCreateManagedBot
        case miniAppCreateBotTitle
        case miniAppCreateBotPromptLabel
        case miniAppCreateBotPromptPlaceholder
        case miniAppCreateBotSubmit
        case miniAppCreateBotSuccess
        case miniAppCreateBotBackgroundSuccess
        case miniAppCreateBotFailedPrefix
        case miniAppGlobalDharmaTitle
        case miniAppGlobalDharmaSubtitle
        case miniAppGlobalDharmaOpen
        case miniAppGlobalDharmaChantButton
        case miniAppGlobalDharmaStartSession
        case miniAppGlobalDharmaEndSession
        case miniAppGlobalDharmaRecordPractice
        case miniAppGlobalDharmaLeaderboardTitle
        case miniAppGlobalDharmaAssetIntegrity
        case miniAppGlobalDharmaVerifiedBadge
        case miniAppGlobalDharmaUnverifiedBadge
        case miniAppTosTitle
        case miniAppTosMessage
        case miniAppTosAccept
        case miniAppTosDecline
        case miniAppTosBlockedMessage
        case miniAppMinimizePiP
        case miniAppRestorePiP
        case miniAppCloseSession
        case miniAppOpenBotProfile
        case miniAppOpenCompanionChat
        case miniAppWebMcpIdle
        case miniAppWebMcpAiControlling
        case miniAppWebMcpWaitingTakeover
        case miniAppWebMcpUserTakeover
        case miniAppWebMcpTakeOverButton
        case miniAppWebMcpResumeAiButton
        case miniAppWebMcpErrorPrefix
        case miniAppEmptyCatalogTitle
        case miniAppEmptyCatalogSubtitle
        case miniAppInvalidUrlBlocked

        // MARK: - Built-in Interactive Browser & Automation App Store (D11-06..D11-09 + D16-01)
        case browserTitle
        case browserUrlPlaceholder
        case browserBack
        case browserForward
        case browserReload
        case browserStopLoading
        case browserInspectorToggle
        case browserTabCopilot
        case browserTabRecorder
        case browserTabSession
        case browserTabStore
        case browserControlIdle
        case browserControlAiRunning
        case browserControlWaitingTakeover
        case browserControlUserTakeover
        case browserControlCancelled
        case browserTakeOverAction
        case browserResumeAiAction
        case browserCancelAiAction
        case browserInvalidUrlError
        case browserStartRecording
        case browserStopRecording
        case browserClearTrace
        case browserDistillScript
        case browserRecordedActionsTitle
        case browserNoRecordedActions
        case browserSecretRedactedBadge
        case browserSessionProfilesTitle
        case browserNewProfile
        case browserEphemeralProfile
        case browserImportCookiesTitle
        case browserImportCookiesPlaceholder
        case browserImportCookiesPasteAction
        case browserImportCookiesFileAction
        case browserExportCookiesAction
        case browserClearCookiesAction
        case browserVaultBackupAction
        case browserVaultRestoreAction
        case browserVaultLockedError
        case browserVaultBundleTooLargeError
        case browserSandboxCaption
        case browserStoreSearchPlaceholder
        case browserStoreInstalledSection
        case browserStoreCatalogSection
        case browserStorePublishDraft
        case browserStoreRunScript
        case browserStoreZeroLlmBadge
        case browserStoreNoAppsTitle
        case browserStoreNoAppsSubtitle
        case browserStoreSecretLeakBlocked
        case browserStoreParametersTitle
        case browserStoreSecretParamHint
        case browserStorePublishSuccess
        case browserStoreReplayComplete
    }

    public static func text(_ key: Key, locale: FabushiLocale) -> String {
        switch key {
        // MARK: Shared Platform Actions
        case .refresh:
            return text(locale, en: "Refresh", zhHans: "刷新")
        case .cancel:
            return text(locale, en: "Cancel", zhHans: "取消")
        case .close:
            return text(locale, en: "Close", zhHans: "关闭")
        case .done:
            return text(locale, en: "Done", zhHans: "完成")
        case .save:
            return text(locale, en: "Save", zhHans: "保存")
        case .delete:
            return text(locale, en: "Delete", zhHans: "删除")
        case .copy:
            return text(locale, en: "Copy", zhHans: "复制")
        case .copied:
            return text(locale, en: "Copied", zhHans: "已复制")
        case .retry:
            return text(locale, en: "Retry", zhHans: "重试")
        case .back:
            return text(locale, en: "Back", zhHans: "返回")
        case .open:
            return text(locale, en: "Open", zhHans: "打开")
        case .run:
            return text(locale, en: "Run", zhHans: "运行")
        case .stop:
            return text(locale, en: "Stop", zhHans: "停止")
        case .install:
            return text(locale, en: "Install", zhHans: "安装")
        case .uninstall:
            return text(locale, en: "Uninstall", zhHans: "卸载")
        case .installed:
            return text(locale, en: "Installed", zhHans: "已安装")
        case .publish:
            return text(locale, en: "Publish", zhHans: "发布")
        case .searchPlaceholder:
            return text(locale, en: "Search", zhHans: "搜索")
        case .loading:
            return text(locale, en: "Loading…", zhHans: "正在加载…")
        case .online:
            return text(locale, en: "Online", zhHans: "在线")
        case .offline:
            return text(locale, en: "Offline", zhHans: "离线")
        case .enabled:
            return text(locale, en: "Enabled", zhHans: "已开启")
        case .disabled:
            return text(locale, en: "Disabled", zhHans: "已停用")
        case .builtInBrowserTitle:
            return text(locale, en: "Built-in Browser & Apps", zhHans: "内置浏览器 · 自动化应用")
        case .builtInBrowserSubtitle:
            return text(
                locale,
                en: "Interactive WKWebView, DOM Recorder, Vault Session Sync & Zero-LLM Automation Store",
                zhHans: "内置交互浏览器、DOM 录制器、加密凭证库会话同步与零 Token 自动化商店"
            )

        // MARK: Remote Computer Hub & Workbench
        case .remoteHubTitle:
            return text(locale, en: "Remote Computers", zhHans: "远程电脑")
        case .remoteTabMyDevices:
            return text(locale, en: "My Devices", zhHans: "我的设备")
        case .remoteTabRemoteAssist:
            return text(locale, en: "Remote Assist", zhHans: "远程协助")
        case .remoteTabCloudPCs:
            return text(locale, en: "Cloud PCs", zhHans: "云电脑")
        case .remoteTabAiMcp:
            return text(locale, en: "AI & MCP", zhHans: "AI 与 MCP")
        case .remoteAllowToggle:
            return text(locale, en: "Allow Remote", zhHans: "允许远程协助")
        case .remoteAllowSubtitle:
            return text(
                locale,
                en: "Allow authorized peers to connect using your assist ID and access code",
                zhHans: "允许已授权设备通过您的协助 ID 与验证码连接"
            )
        case .remoteDisconnectAllAction:
            return text(locale, en: "Disconnect All", zhHans: "一键断开全部")
        case .remoteEmergencyTakeover:
            return text(locale, en: "Emergency Takeover", zhHans: "紧急接管")
        case .remoteControlledBannerTitle:
            return text(locale, en: "Active Remote Control", zhHans: "远程控制进行中")
        case .remoteTailnetTitle:
            return text(locale, en: "Tailnet Mesh", zhHans: "Tailnet 组网")
        case .remoteTailnetDirectP2P:
            return text(locale, en: "Tailnet P2P", zhHans: "P2P 直连")
        case .remoteTailnetDerpRelay:
            return text(locale, en: "DERP Relay", zhHans: "DERP 加密中继")
        case .remoteTailnetHttpFallback:
            return text(locale, en: "HTTP Keepalive", zhHans: "HTTP 保活")
        case .remoteTailnetOffline:
            return text(locale, en: "Mesh Offline", zhHans: "组网离线")
        case .remoteTailnetForceDerp:
            return text(locale, en: "Force DERP Relay", zhHans: "强制 DERP 中继")
        case .remoteTailnetStart:
            return text(locale, en: "Start Mesh", zhHans: "启动组网")
        case .remoteTailnetStop:
            return text(locale, en: "Stop Mesh", zhHans: "停止组网")
        case .remoteTailnetRegister:
            return text(locale, en: "Register Node", zhHans: "注册节点")
        case .remoteLocalHostUnsupportedTitle:
            return text(locale, en: "iOS Mobile Controller Mode", zhHans: "iOS 移动控制端模式")
        case .remoteLocalHostUnsupportedBody:
            return text(
                locale,
                en: "Unattended local host daemon hosting is desktop-only. This iPhone/iPad controls remote desktop & cloud nodes in user-space; opt-in screen sharing uses Apple ReplayKit.",
                zhHans: "无人值守本地受控宿主机仅支持桌面端。当前 iPhone/iPad 以用户态远程控制桌面与云电脑；如需主动共享屏幕，将通过 Apple ReplayKit 发起。"
            )
        case .remoteCapabilityDiagnostics:
            return text(locale, en: "iOS Capabilities", zhHans: "iOS 能力诊断")
        case .remotePermissionModalTitle:
            return text(locale, en: "iOS Remote Capability Diagnostics", zhHans: "iOS 远程控制能力诊断")
        case .remotePermissionScreenStatusTitle:
            return text(locale, en: "Screen Broadcast (ReplayKit)", zhHans: "屏幕广播 (ReplayKit)")
        case .remotePermissionScreenStatusValue:
            return text(
                locale,
                en: "Opt-in via FabushiBroadcastUpload only; background unattended screen capture is blocked by iOS sandbox.",
                zhHans: "仅支持通过 FabushiBroadcastUpload 手动开启；iOS 沙盒禁止后台静默截屏。"
            )
        case .remotePermissionAccessibilityTitle:
            return text(locale, en: "Unattended Local Input & PTY Host", zhHans: "本地无人值守键鼠与终端宿主")
        case .remotePermissionAccessibilityValue:
            return text(
                locale,
                en: "Fail-closed on iOS (capability_unavailable). All remote input/PTY commands execute on remote desktop or cloud nodes.",
                zhHans: "iOS 端按 fail-closed 返回 capability_unavailable，所有键鼠与终端指令均在远端桌面或云电脑执行。"
            )
        case .remotePermissionRetry:
            return text(locale, en: "Re-check Status", zhHans: "重新检测")
        case .remotePermissionDismiss:
            return text(locale, en: "Got It", zhHans: "知道了")
        case .remoteThisPCBadge:
            return text(locale, en: "This PC", zhHans: "本机")
        case .remoteCloudBadge:
            return text(locale, en: "Cloud", zhHans: "云电脑")
        case .remoteActionEnterDesktop:
            return text(locale, en: "Enter Desktop", zhHans: "进入桌面")
        case .remoteActionShell:
            return text(locale, en: "Shell", zhHans: "终端")
        case .remoteActionSettings:
            return text(locale, en: "Settings", zhHans: "设置")
        case .remoteAddCloudPC:
            return text(locale, en: "Add Cloud PC", zhHans: "添加云电脑")
        case .remoteAddCloudPCSubtitle:
            return text(
                locale,
                en: "Connect an SSH server or Fabushi Cloud Sandbox",
                zhHans: "连接 SSH 云服务器或 Fabushi 云沙盒"
            )
        case .remoteQuickAssistTitle:
            return text(locale, en: "Quick Assist", zhHans: "快速远程协助")
        case .remoteAssistIdLabel:
            return text(locale, en: "My Assist ID", zhHans: "我的协助 ID")
        case .remoteAccessCodeLabel:
            return text(locale, en: "Access Code", zhHans: "验证码")
        case .remoteResetCode:
            return text(locale, en: "Reset Code", zhHans: "重置验证码")
        case .remoteSetCustomCode:
            return text(locale, en: "Custom Code", zhHans: "自定义验证码")
        case .remoteCopyInvite:
            return text(locale, en: "Copy Invite", zhHans: "复制邀请")
        case .remoteAssistOthersAction:
            return text(locale, en: "Assist Others →", zhHans: "协助他人 →")
        case .remoteAssistTargetIdPlaceholder:
            return text(locale, en: "9-digit Assist ID", zhHans: "9 位协助 ID")
        case .remoteAssistAccessCodePlaceholder:
            return text(locale, en: "Access Code", zhHans: "访问验证码")
        case .remoteAssistConnectAction:
            return text(locale, en: "Start Assist Session", zhHans: "发起远程协助")
        case .remoteEmptyDevicesTitle:
            return text(locale, en: "No Remote Computers Yet", zhHans: "暂无远程电脑")
        case .remoteEmptyDevicesSubtitle:
            return text(
                locale,
                en: "Sign in on Fabushi Desktop or add a Cloud PC to control it remotely.",
                zhHans: "在桌面端登录 Fabushi 或添加一台云电脑即可远程连接。"
            )
        case .remoteWorkbenchDesktopTab:
            return text(locale, en: "Desktop", zhHans: "桌面")
        case .remoteWorkbenchTerminalTab:
            return text(locale, en: "Terminal", zhHans: "终端")
        case .remoteWorkbenchFilesTab:
            return text(locale, en: "Files", zhHans: "文件")
        case .remoteWorkbenchSettingsTab:
            return text(locale, en: "Device", zhHans: "设备")
        case .remoteWorkbenchCaptureFrame:
            return text(locale, en: "Refresh Frame", zhHans: "刷新画面")
        case .remoteWorkbenchSendKey:
            return text(locale, en: "Hotkeys", zhHans: "快捷键")
        case .remoteWorkbenchPrivacyScreen:
            return text(locale, en: "Privacy Screen", zhHans: "隐私屏")
        case .remoteWorkbenchClipboardPull:
            return text(locale, en: "Pull Clipboard", zhHans: "拉取剪贴板")
        case .remoteWorkbenchClipboardPush:
            return text(locale, en: "Push Clipboard", zhHans: "发送剪贴板")
        case .remoteWorkbenchDisplaySwitch:
            return text(locale, en: "Switch Display", zhHans: "切换显示器")
        case .remoteWorkbenchPowerLock:
            return text(locale, en: "Lock Screen", zhHans: "锁屏")
        case .remoteWorkbenchPowerSleep:
            return text(locale, en: "Sleep", zhHans: "休眠")
        case .remoteWorkbenchPowerReboot:
            return text(locale, en: "Reboot", zhHans: "重启")
        case .remoteWorkbenchWakeOnLan:
            return text(locale, en: "Wake-on-LAN", zhHans: "网络唤醒")
        case .remoteWorkbenchTestConnection:
            return text(locale, en: "Test Connection", zhHans: "测试连接")
        case .remotePtyNewSession:
            return text(locale, en: "New PTY", zhHans: "新建终端")
        case .remotePtyInputPlaceholder:
            return text(locale, en: "Enter shell command…", zhHans: "输入终端命令…")
        case .remotePtySendAction:
            return text(locale, en: "Run", zhHans: "执行")
        case .remotePtyOutputPlaceholder:
            return text(locale, en: "Terminal output will appear here.", zhHans: "终端输出将显示在此处。")
        case .remotePendingApprovalsTitle:
            return text(locale, en: "Pending Command Approvals", zhHans: "待审批远程指令")
        case .remoteNoPendingApprovals:
            return text(locale, en: "No pending command approvals.", zhHans: "当前没有待审批指令。")
        case .remoteApprove:
            return text(locale, en: "Approve", zhHans: "批准")
        case .remoteDeny:
            return text(locale, en: "Deny", zhHans: "拒绝")
        case .remoteBotProfileTitle:
            return text(locale, en: "AI Bot Computer Binding", zhHans: "AI 机器人电脑绑定")
        case .remoteCloudFormTitle:
            return text(locale, en: "Add Cloud PC", zhHans: "添加云电脑")
        case .remoteCloudNamePlaceholder:
            return text(locale, en: "Display Name", zhHans: "设备名称")
        case .remoteCloudHostPlaceholder:
            return text(locale, en: "Host or IP Address", zhHans: "主机名或 IP 地址")
        case .remoteCloudPortPlaceholder:
            return text(locale, en: "SSH Port (22)", zhHans: "SSH 端口 (22)")
        case .remoteCloudUsernamePlaceholder:
            return text(locale, en: "Username", zhHans: "用户名")
        case .remoteCloudSecretPlaceholder:
            return text(locale, en: "Password, Private Key or Token", zhHans: "密码、私钥或令牌")
        case .remoteCloudWorkdirPlaceholder:
            return text(locale, en: "Working Directory (optional)", zhHans: "工作目录（可选）")
        case .remotePolicyTitle:
            return text(locale, en: "Execution Policy", zhHans: "执行策略")
        case .remotePolicyAskEveryTime:
            return text(locale, en: "Ask Every Time", zhHans: "每次询问")
        case .remotePolicyAllowReadOnly:
            return text(locale, en: "Allow Read-Only", zhHans: "自动允许只读")
        case .remotePolicyAlwaysAllow:
            return text(locale, en: "Always Allow Authorized", zhHans: "始终允许已授权")
        case .remotePolicyDisabled:
            return text(locale, en: "Disabled", zhHans: "禁止运行")
        case .remotePolicySavedToast:
            return text(locale, en: "Device policy updated", zhHans: "设备策略已更新")
        case .remoteAssistAllowFailedPrefix:
            return text(locale, en: "Failed to update Allow Remote", zhHans: "更新远程协助开关失败")
        case .remoteDisconnectAllSuccess:
            return text(locale, en: "Disconnected all remote control sessions", zhHans: "已断开全部远程控制会话")
        case .remoteEmergencyTakeoverSuccess:
            return text(locale, en: "Emergency takeover completed", zhHans: "紧急接管已完成")
        case .remoteControlledPillTakeover:
            return text(locale, en: "Take Over", zhHans: "接管")

        // MARK: Mini Apps, WebMCP, Floating PiP & 全球法布施
        case .miniAppHubTitle:
            return text(locale, en: "Mini Apps", zhHans: "小程序")
        case .miniAppCatalogSearchPlaceholder:
            return text(locale, en: "Search Mini Apps…", zhHans: "搜索小程序…")
        case .miniAppFilterAll:
            return text(locale, en: "All", zhHans: "全部")
        case .miniAppFilterInstalled:
            return text(locale, en: "Installed", zhHans: "已安装")
        case .miniAppFilterRecent:
            return text(locale, en: "Recent", zhHans: "最近使用")
        case .miniAppCreateManagedBot:
            return text(locale, en: "Create Bot / Mini App", zhHans: "创建机器人 / 小程序")
        case .miniAppCreateBotTitle:
            return text(locale, en: "Create Managed Bot", zhHans: "创建托管机器人")
        case .miniAppCreateBotPromptLabel:
            return text(
                locale,
                en: "Describe your Bot or Mini App in natural language",
                zhHans: "使用自然语言描述您想创建的机器人或小程序"
            )
        case .miniAppCreateBotPromptPlaceholder:
            return text(
                locale,
                en: "e.g., Daily reading tracker with habit reminders…",
                zhHans: "例如：带每日打卡提醒的阅读记录助手…"
            )
        case .miniAppCreateBotSubmit:
            return text(locale, en: "Create", zhHans: "创建")
        case .miniAppCreateBotSuccess:
            return text(locale, en: "Managed bot created", zhHans: "托管机器人已创建")
        case .miniAppCreateBotBackgroundSuccess:
            return text(
                locale,
                en: "Managed bot created in background; catalog refreshed",
                zhHans: "托管机器人已在后台创建完成，目录已刷新"
            )
        case .miniAppCreateBotFailedPrefix:
            return text(locale, en: "Failed to create bot", zhHans: "创建机器人失败")
        case .miniAppGlobalDharmaTitle:
            return text(locale, en: "Global Dharma (全球法布施)", zhHans: "全球法布施")
        case .miniAppGlobalDharmaSubtitle:
            return text(
                locale,
                en: "Offline-first verified .fabapp runtime, chanting practice, SQLite records & global leaderboard",
                zhHans: "离线优先的签名 .fabapp 运行时、诵经修行记录与全球榜单"
            )
        case .miniAppGlobalDharmaOpen:
            return text(locale, en: "Open Global Dharma", zhHans: "进入全球法布施")
        case .miniAppGlobalDharmaChantButton:
            return text(locale, en: "+1 Chant", zhHans: "计数 +1")
        case .miniAppGlobalDharmaStartSession:
            return text(locale, en: "Start Practice", zhHans: "开始共修")
        case .miniAppGlobalDharmaEndSession:
            return text(locale, en: "End Practice", zhHans: "结束共修")
        case .miniAppGlobalDharmaRecordPractice:
            return text(locale, en: "Save Practice Record", zhHans: "保存修行记录")
        case .miniAppGlobalDharmaLeaderboardTitle:
            return text(locale, en: "Global Practice Leaderboard", zhHans: "全球修行榜")
        case .miniAppGlobalDharmaAssetIntegrity:
            return text(locale, en: "Embedded WASM & Asset Integrity", zhHans: "内置 WASM 与资源完整性")
        case .miniAppGlobalDharmaVerifiedBadge:
            return text(locale, en: "Ed25519 Verified .fabapp", zhHans: "Ed25519 已验签 .fabapp")
        case .miniAppGlobalDharmaUnverifiedBadge:
            return text(locale, en: "Verification Required (Fail-Closed)", zhHans: "待验签（严格拦截）")
        case .miniAppTosTitle:
            return text(locale, en: "Mini App Terms of Service", zhHans: "小程序服务条款")
        case .miniAppTosMessage:
            return text(
                locale,
                en: "Before launching this third-party Mini App, review and accept its Terms of Service. Declining blocks execution.",
                zhHans: "启动此第三方小程序前请确认并同意其服务条款；拒绝将阻止加载运行。"
            )
        case .miniAppTosAccept:
            return text(locale, en: "Accept & Launch", zhHans: "同意并启动")
        case .miniAppTosDecline:
            return text(locale, en: "Decline", zhHans: "拒绝")
        case .miniAppTosBlockedMessage:
            return text(
                locale,
                en: "Launch blocked because the Mini App Terms of Service were declined.",
                zhHans: "您已拒绝小程序服务条款，已阻止启动。"
            )
        case .miniAppMinimizePiP:
            return text(locale, en: "Minimize to PiP", zhHans: "收起至浮窗")
        case .miniAppRestorePiP:
            return text(locale, en: "Restore", zhHans: "恢复")
        case .miniAppCloseSession:
            return text(locale, en: "Close Mini App", zhHans: "关闭小程序")
        case .miniAppOpenBotProfile:
            return text(locale, en: "Bot Profile", zhHans: "机器人资料")
        case .miniAppOpenCompanionChat:
            return text(locale, en: "Companion Chat", zhHans: "伴随对话")
        case .miniAppWebMcpIdle:
            return text(locale, en: "WebMCP Ready", zhHans: "WebMCP 就绪")
        case .miniAppWebMcpAiControlling:
            return text(locale, en: "AI Controlling", zhHans: "AI 操作中")
        case .miniAppWebMcpWaitingTakeover:
            return text(locale, en: "Takeover Requested", zhHans: "等待人工接管")
        case .miniAppWebMcpUserTakeover:
            return text(locale, en: "User Takeover", zhHans: "用户已接管")
        case .miniAppWebMcpTakeOverButton:
            return text(locale, en: "Take Over", zhHans: "人工接管")
        case .miniAppWebMcpResumeAiButton:
            return text(locale, en: "Resume AI", zhHans: "恢复 AI")
        case .miniAppWebMcpErrorPrefix:
            return text(locale, en: "WebMCP Error", zhHans: "WebMCP 错误")
        case .miniAppEmptyCatalogTitle:
            return text(locale, en: "No Mini Apps Found", zhHans: "暂无匹配小程序")
        case .miniAppEmptyCatalogSubtitle:
            return text(
                locale,
                en: "Try a different filter, launch Global Dharma, or create a managed bot.",
                zhHans: "请更换筛选条件、进入全球法布施或创建一个新的托管机器人。"
            )
        case .miniAppInvalidUrlBlocked:
            return text(
                locale,
                en: "Blocked unsafe Mini App URL scheme.",
                zhHans: "已拦截不安全的小程序 URL 协议。"
            )

        // MARK: Built-in Interactive Browser & Automation App Store
        case .browserTitle:
            return text(locale, en: "Built-in Browser", zhHans: "内置浏览器")
        case .browserUrlPlaceholder:
            return text(locale, en: "Enter https:// address…", zhHans: "输入 https:// 网址…")
        case .browserBack:
            return text(locale, en: "Back", zhHans: "后退")
        case .browserForward:
            return text(locale, en: "Forward", zhHans: "前进")
        case .browserReload:
            return text(locale, en: "Reload", zhHans: "刷新")
        case .browserStopLoading:
            return text(locale, en: "Stop", zhHans: "停止加载")
        case .browserInspectorToggle:
            return text(locale, en: "Inspector", zhHans: "检查器")
        case .browserTabCopilot:
            return text(locale, en: "Copilot", zhHans: "AI 助手")
        case .browserTabRecorder:
            return text(locale, en: "Recorder", zhHans: "录制器")
        case .browserTabSession:
            return text(locale, en: "Session", zhHans: "会话")
        case .browserTabStore:
            return text(locale, en: "Store", zhHans: "应用商店")
        case .browserControlIdle:
            return text(locale, en: "Idle", zhHans: "空闲")
        case .browserControlAiRunning:
            return text(locale, en: "AI Running", zhHans: "AI 运行中")
        case .browserControlWaitingTakeover:
            return text(locale, en: "Takeover Requested", zhHans: "请求接管")
        case .browserControlUserTakeover:
            return text(locale, en: "User Takeover", zhHans: "人工接管中")
        case .browserControlCancelled:
            return text(locale, en: "Cancelled", zhHans: "已取消")
        case .browserTakeOverAction:
            return text(locale, en: "Take Over", zhHans: "立即接管")
        case .browserResumeAiAction:
            return text(locale, en: "Resume AI", zhHans: "交还 AI")
        case .browserCancelAiAction:
            return text(locale, en: "Cancel AI", zhHans: "中止 AI")
        case .browserInvalidUrlError:
            return text(
                locale,
                en: "Only http:// and https:// URLs are allowed in the built-in browser.",
                zhHans: "内置浏览器仅允许访问 http:// 与 https:// 网址。"
            )
        case .browserStartRecording:
            return text(locale, en: "Start Recording", zhHans: "开始录制")
        case .browserStopRecording:
            return text(locale, en: "Stop Recording", zhHans: "停止录制")
        case .browserClearTrace:
            return text(locale, en: "Clear Trace", zhHans: "清空轨迹")
        case .browserDistillScript:
            return text(locale, en: "Distill Script", zhHans: "蒸馏脚本")
        case .browserRecordedActionsTitle:
            return text(locale, en: "Recorded DOM Actions", zhHans: "已录制 DOM 动作")
        case .browserNoRecordedActions:
            return text(
                locale,
                en: "Start recording and interact with the page to capture semantic actions.",
                zhHans: "开启录制并在网页中操作，即可捕获语义定位动作。"
            )
        case .browserSecretRedactedBadge:
            return text(locale, en: "Secret Redacted", zhHans: "敏感字段已脱敏")
        case .browserSessionProfilesTitle:
            return text(locale, en: "Isolated Browser Profiles", zhHans: "隔离浏览器配置")
        case .browserNewProfile:
            return text(locale, en: "New Profile", zhHans: "新建配置")
        case .browserEphemeralProfile:
            return text(locale, en: "Ephemeral (Non-Persistent)", zhHans: "临时无痕会话")
        case .browserImportCookiesTitle:
            return text(locale, en: "Import Cookies (JSON / Netscape)", zhHans: "导入 Cookie (JSON / Netscape)")
        case .browserImportCookiesPlaceholder:
            return text(
                locale,
                en: "Paste CDP JSON, EditThisCookie JSON, or cookies.txt…",
                zhHans: "粘贴 CDP JSON、EditThisCookie JSON 或 Netscape cookies.txt…"
            )
        case .browserImportCookiesPasteAction:
            return text(locale, en: "Import Text", zhHans: "导入文本")
        case .browserImportCookiesFileAction:
            return text(locale, en: "Import File…", zhHans: "选择文件导入…")
        case .browserExportCookiesAction:
            return text(locale, en: "Export Cookies", zhHans: "导出 Cookie")
        case .browserClearCookiesAction:
            return text(locale, en: "Clear Profile Data", zhHans: "清空配置数据")
        case .browserVaultBackupAction:
            return text(locale, en: "Backup to Vault", zhHans: "加密备份至凭证库")
        case .browserVaultRestoreAction:
            return text(locale, en: "Restore from Vault", zhHans: "从凭证库恢复")
        case .browserVaultLockedError:
            return text(
                locale,
                en: "Credential Vault is locked. Unlock Vault in Settings before backing up or restoring browser profiles.",
                zhHans: "加密凭证库尚未解锁，请先在设置中解锁凭证库后再同步浏览器会话。"
            )
        case .browserVaultBundleTooLargeError:
            return text(
                locale,
                en: "Browser profile cookie bundle exceeds the 512 KiB E2EE Vault cloud sync cap.",
                zhHans: "浏览器配置 Cookie 包超过 512 KiB 端到端加密云同步上限。"
            )
        case .browserSandboxCaption:
            return text(
                locale,
                en: "iOS Sandbox: Local Chromium subprocesses and desktop Chrome SQLite/Keychain scraping are unavailable. Import cookies via JSON/Netscape or sync encrypted profiles from Desktop Vault.",
                zhHans: "iOS 沙盒说明：不支持本地 Chromium 子进程或读取桌面 Chrome 数据库/钥匙串。请通过 JSON/Netscape 导入 Cookie，或从桌面端加密凭证库同步。"
            )
        case .browserStoreSearchPlaceholder:
            return text(locale, en: "Search automation apps…", zhHans: "搜索自动化应用…")
        case .browserStoreInstalledSection:
            return text(locale, en: "Installed Apps", zhHans: "已安装应用")
        case .browserStoreCatalogSection:
            return text(locale, en: "Automation App Store", zhHans: "自动化应用商店")
        case .browserStorePublishDraft:
            return text(locale, en: "Publish Distilled App", zhHans: "发布蒸馏应用")
        case .browserStoreRunScript:
            return text(locale, en: "Run (Zero-LLM)", zhHans: "运行（零 Token）")
        case .browserStoreZeroLlmBadge:
            return text(locale, en: "0-LLM Replay", zhHans: "零 LLM 回放")
        case .browserStoreNoAppsTitle:
            return text(locale, en: "No Automation Apps", zhHans: "暂无自动化应用")
        case .browserStoreNoAppsSubtitle:
            return text(
                locale,
                en: "Search the store or distill a recorded workflow in the Recorder tab.",
                zhHans: "可在商店中搜索，或在录制器中将操作轨迹蒸馏为可复用应用。"
            )
        case .browserStoreSecretLeakBlocked:
            return text(
                locale,
                en: "Publish blocked: secret leak detected",
                zhHans: "发布已拦截：检测到明文敏感信息"
            )
        case .browserStoreParametersTitle:
            return text(locale, en: "Run Parameters", zhHans: "运行参数")
        case .browserStoreSecretParamHint:
            return text(
                locale,
                en: "Resolved from unlocked Vault; never echoed in logs",
                zhHans: "从已解锁凭证库解析，绝不回显至日志"
            )
        case .browserStorePublishSuccess:
            return text(locale, en: "Automation app published", zhHans: "自动化应用已发布")
        case .browserStoreReplayComplete:
            return text(locale, en: "Deterministic replay completed", zhHans: "确定性脚本回放完成")
        }
    }

    // MARK: - Parameterized Formatters

    public static func disconnectAllTitle(count: Int, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return "Disconnect All (\(count))"
        case .zhHans:
            return "一键断开全部 (\(count))"
        }
    }

    public static func activeSessionsSummary(count: Int, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return count == 1 ? "1 active session" : "\(count) active sessions"
        case .zhHans:
            return "\(count) 个活动会话"
        }
    }

    public static func tailnetSummary(
        ip: String,
        relayLabel: String,
        rttMs: UInt32?,
        locale: FabushiLocale
    ) -> String {
        let displayIP = ip.isEmpty ? text(.remoteTailnetOffline, locale: locale) : ip
        if let rttMs {
            return "\(displayIP) · \(relayLabel) · \(rttMs)ms"
        }
        return "\(displayIP) · \(relayLabel)"
    }

    public static func stepsCount(_ count: Int, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return count == 1 ? "1 step" : "\(count) steps"
        case .zhHans:
            return "\(count) 个步骤"
        }
    }

    public static func installsCount(_ count: UInt64, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return "\(count) installs"
        case .zhHans:
            return "\(count) 次安装"
        }
    }

    public static func runsCount(_ count: UInt64, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return "\(count) runs"
        case .zhHans:
            return "\(count) 次运行"
        }
    }

    public static func stepProgress(current: Int, total: Int, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return "Step \(current) of \(total)"
        case .zhHans:
            return "第 \(current) / \(total) 步"
        }
    }

    public static func cookiesImportedSummary(count: Int, locale: FabushiLocale) -> String {
        switch locale {
        case .en:
            return count == 1 ? "Imported 1 cookie" : "Imported \(count) cookies"
        case .zhHans:
            return "已导入 \(count) 条 Cookie"
        }
    }
}
