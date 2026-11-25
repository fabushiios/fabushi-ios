import Foundation

/// Module-local bilingual (`en` + `zh-Hans`) string table shared across all Milestone M4 surfaces
/// (Agent Studio, Multi-Agent Protocol, Magpie AI Gateway, Coding Agents, and Official MCP Console).
///
/// Kept inside `Presentation/AgentStudio/` to honor the Parallel Freeze Matrix on `Localizable.xcstrings`
/// during M4; M6 consolidates these keys directly into `Localizable.xcstrings`.
public enum FabushiAgentStudioStrings: Sendable {
    public struct Entry: Sendable, Equatable {
        public let en: String
        public let zhHans: String

        public init(en: String, zhHans: String) {
            self.en = en
            self.zhHans = zhHans
        }
    }

    public static let table: [String: Entry] = [
        // MARK: - Common Actions & States
        "m4.common.refresh": Entry(en: "Refresh", zhHans: "刷新"),
        "m4.common.save": Entry(en: "Save", zhHans: "保存"),
        "m4.common.create": Entry(en: "Create", zhHans: "创建"),
        "m4.common.add": Entry(en: "Add", zhHans: "添加"),
        "m4.common.edit": Entry(en: "Edit", zhHans: "编辑"),
        "m4.common.delete": Entry(en: "Delete", zhHans: "删除"),
        "m4.common.cancel": Entry(en: "Cancel", zhHans: "取消"),
        "m4.common.close": Entry(en: "Close", zhHans: "关闭"),
        "m4.common.done": Entry(en: "Done", zhHans: "完成"),
        "m4.common.retry": Entry(en: "Retry", zhHans: "重试"),
        "m4.common.reset": Entry(en: "Reset", zhHans: "重置"),
        "m4.common.copy": Entry(en: "Copy", zhHans: "复制"),
        "m4.common.copied": Entry(en: "Copied to secure clipboard", zhHans: "已复制到安全剪贴板"),
        "m4.common.search": Entry(en: "Search", zhHans: "搜索"),
        "m4.common.clear": Entry(en: "Clear", zhHans: "清空"),
        "m4.common.apply": Entry(en: "Apply", zhHans: "应用"),
        "m4.common.inspect": Entry(en: "Inspect", zhHans: "查看详情"),
        "m4.common.enabled": Entry(en: "Enabled", zhHans: "已启用"),
        "m4.common.disabled": Entry(en: "Disabled", zhHans: "已停用"),
        "m4.common.loading": Entry(en: "Loading…", zhHans: "加载中…"),
        "m4.common.online": Entry(en: "Online", zhHans: "在线"),
        "m4.common.offline": Entry(en: "Offline", zhHans: "离线"),
        "m4.common.localMode": Entry(en: "Embedded Local Host", zhHans: "本地嵌入式宿主"),
        "m4.common.remoteMode": Entry(en: "Connected Server", zhHans: "已连接服务器"),
        "m4.common.iosSandboxNotice": Entry(en: "iOS Sandbox Boundary", zhHans: "iOS 沙盒能力边界"),
        "m4.common.unknownError": Entry(en: "Unknown local execution error", zhHans: "未知执行错误，请稍后重试"),
        "m4.common.signInRequired": Entry(en: "Sign in to sync server data", zhHans: "登录后可同步服务器数据"),

        // MARK: - 3-Tier AI Provider Error Classifier (D10-10)
        "m4.error.tier1.heading": Entry(
            en: "AI Provider Not Ready",
            zhHans: "未配置可用的 AI 供应商"
        ),
        "m4.error.tier1.cta": Entry(
            en: "Open AI Provider Settings",
            zhHans: "打开 AI 供应商设置"
        ),
        "m4.error.tier1.toast": Entry(
            en: "AI provider unavailable — tap card button to configure",
            zhHans: "AI 供应商未就绪，请点击消息按钮配置"
        ),
        "m4.error.tier2.heading": Entry(
            en: "AI Provider Request Failed",
            zhHans: "AI 服务请求失败"
        ),
        "m4.error.tier2.toast": Entry(
            en: "AI provider request failed — see error details",
            zhHans: "AI 服务请求失败，请查看错误详情"
        ),
        "m4.error.tier3.heading": Entry(
            en: "Local AI Turn Failed",
            zhHans: "本地 AI 执行失败"
        ),
        "m4.error.tier3.toast": Entry(
            en: "Local AI turn failed — see error details",
            zhHans: "本地 AI 执行失败，请查看错误详情"
        ),
        "m4.error.dismissHint": Entry(
            en: "Dismiss error banner",
            zhHans: "关闭错误提示"
        ),

        // MARK: - Agent Studio Root & 7 Tabs
        "m4.studio.title": Entry(en: "Agent Studio", zhHans: "智能体工作室"),
        "m4.studio.tab.overview": Entry(en: "Overview", zhHans: "总览"),
        "m4.studio.tab.kanban": Entry(en: "Kanban", zhHans: "看板"),
        "m4.studio.tab.skills": Entry(en: "Skills", zhHans: "技能"),
        "m4.studio.tab.memory": Entry(en: "Memory", zhHans: "记忆"),
        "m4.studio.tab.cron": Entry(en: "Cron", zhHans: "定时任务"),
        "m4.studio.tab.approvals": Entry(en: "Approvals", zhHans: "审批"),
        "m4.studio.tab.sessions": Entry(en: "Sessions", zhHans: "会话"),
        "m4.studio.tabBarLabel": Entry(en: "Agent Studio Sections", zhHans: "智能体工作室分栏"),

        // MARK: - Studio Overview & 29 Toolsets + Browser Automation (D11-03)
        "m4.studio.overview.runtimeSection": Entry(en: "Agent Runtime Status", zhHans: "智能体运行时状态"),
        "m4.studio.overview.engineLabel": Entry(en: "Engine", zhHans: "引擎"),
        "m4.studio.overview.commandsLabel": Entry(en: "Slash Commands", zhHans: "斜杠命令"),
        "m4.studio.overview.toolsetsLabel": Entry(en: "Active Toolsets", zhHans: "已启用工具集"),
        "m4.studio.overview.storageLabel": Entry(en: "SQLite Storage", zhHans: "SQLite 存储"),
        "m4.studio.overview.fts5Available": Entry(en: "FTS5 Full-Text Ready", zhHans: "FTS5 全文检索就绪"),
        "m4.studio.overview.fts5Fallback": Entry(en: "LIKE Search Fallback", zhHans: "LIKE 搜索回退"),
        "m4.studio.overview.quickActions": Entry(en: "Connected Consoles", zhHans: "关联控制台"),
        "m4.studio.overview.openMagpie": Entry(en: "AI Providers & Magpie", zhHans: "AI 供应商与喜鹊网关"),
        "m4.studio.overview.openMcp": Entry(en: "Official OAuth 2.1 MCP", zhHans: "官方 OAuth 2.1 MCP 授权"),
        "m4.studio.overview.openMultiAgent": Entry(en: "Multi-Agent Room Lab", zhHans: "多智能体协作诊断"),
        "m4.studio.overview.botsSection": Entry(en: "Configured Agents", zhHans: "已配置的智能体"),
        "m4.studio.overview.noBots": Entry(
            en: "No server bots loaded yet",
            zhHans: "尚未加载服务器智能体"
        ),
        "m4.studio.toolsets.sectionTitle": Entry(en: "29 Hermes Toolsets", zhHans: "29 项 Hermes 工具集"),
        "m4.studio.toolsets.presetsTitle": Entry(en: "Quick Presets", zhHans: "快捷预设"),
        "m4.studio.toolsets.presetSafe": Entry(en: "Safe (iOS)", zhHans: "安全预设 (iOS)"),
        "m4.studio.toolsets.presetMinimal": Entry(en: "Minimal", zhHans: "极简预设"),
        "m4.studio.toolsets.presetCli": Entry(en: "CLI (Desktop)", zhHans: "CLI (桌面)"),
        "m4.studio.toolsets.presetTelegram": Entry(en: "Full IM (Desktop)", zhHans: "全功能 (桌面)"),
        "m4.studio.toolsets.presetAcp": Entry(en: "ACP (Desktop)", zhHans: "ACP (桌面)"),
        "m4.studio.toolsets.presetFull": Entry(en: "Full (Desktop)", zhHans: "全部 29 项 (桌面)"),
        "m4.studio.toolsets.sandboxGatedBadge": Entry(en: "Desktop / Remote Only", zhHans: "仅桌面或远程宿主"),
        "m4.studio.toolsets.coreLockedBadge": Entry(en: "Required Core", zhHans: "核心必选"),
        "m4.studio.toolsets.toolCountFormat": Entry(en: "%d tools", zhHans: "%d 个工具"),
        "m4.studio.toolsets.updatedToast": Entry(en: "Toolset configuration updated", zhHans: "工具集配置已更新"),
        "m4.studio.toolsets.browserCatalogTitle": Entry(
            en: "Browser Automation Tools (16)",
            zhHans: "浏览器自动化工具 (16)"
        ),
        "m4.studio.toolsets.browserV1611Badge": Entry(
            en: "v1.6.11 New",
            zhHans: "v1.6.11 新增"
        ),
        "m4.studio.toolsets.browserIosNotice": Entry(
            en: "Headless Chromium is disabled in the iOS sandbox; remote execution or WKWebView bridge handles web automation safely",
            zhHans: "iOS 沙盒禁用本地无头浏览器进程，网页自动化将通过安全远程宿主或原生视图执行"
        ),

        // MARK: - Studio Kanban & Tasks FSM
        "m4.studio.kanban.tasksSection": Entry(en: "Active Agent Tasks", zhHans: "活跃智能体任务"),
        "m4.studio.kanban.newTask": Entry(en: "New Task", zhHans: "新建任务"),
        "m4.studio.kanban.taskIdPlaceholder": Entry(en: "Optional task ID", zhHans: "可选任务编号"),
        "m4.studio.kanban.noTasksTitle": Entry(en: "No Agent Tasks", zhHans: "暂无智能体任务"),
        "m4.studio.kanban.noTasksDesc": Entry(
            en: "Create a state-tracked task to orchestrate execution and approvals",
            zhHans: "创建状态机任务以编排智能体执行与人工审批"
        ),
        "m4.studio.kanban.approveTask": Entry(en: "Approve", zhHans: "批准运行"),
        "m4.studio.kanban.denyTask": Entry(en: "Abort", zhHans: "中止"),
        "m4.studio.kanban.boardsSection": Entry(en: "Kanban Boards", zhHans: "任务看板"),
        "m4.studio.kanban.newBoard": Entry(en: "New Board", zhHans: "新建看板"),
        "m4.studio.kanban.newCard": Entry(en: "New Card", zhHans: "新建卡片"),
        "m4.studio.kanban.boardNamePlaceholder": Entry(en: "Board name", zhHans: "看板名称"),
        "m4.studio.kanban.boardDescPlaceholder": Entry(en: "Description", zhHans: "描述"),
        "m4.studio.kanban.cardTitlePlaceholder": Entry(en: "Card title", zhHans: "卡片标题"),
        "m4.studio.kanban.cardDescPlaceholder": Entry(en: "Task notes", zhHans: "任务说明"),
        "m4.studio.kanban.priorityLabel": Entry(en: "Priority", zhHans: "优先级"),
        "m4.studio.kanban.moveCard": Entry(en: "Move to Column", zhHans: "移动到列"),
        "m4.studio.kanban.noBoardsTitle": Entry(en: "No Kanban Boards", zhHans: "暂无任务看板"),
        "m4.studio.kanban.noBoardsDesc": Entry(
            en: "Create a persistent SQLite Kanban board to organize subagent workflows",
            zhHans: "创建持久化 SQLite 看板来组织子智能体工作流"
        ),
        "m4.studio.kanban.emptyColumn": Entry(en: "No cards in this column", zhHans: "当前列暂无卡片"),
        "m4.studio.taskState.pending": Entry(en: "Pending", zhHans: "等待中"),
        "m4.studio.taskState.running": Entry(en: "Running", zhHans: "运行中"),
        "m4.studio.taskState.waitingAuth": Entry(en: "Waiting Auth", zhHans: "等待授权"),
        "m4.studio.taskState.waitingConnection": Entry(en: "Waiting Host", zhHans: "等待连接"),
        "m4.studio.taskState.completed": Entry(en: "Completed", zhHans: "已完成"),
        "m4.studio.taskState.failed": Entry(en: "Failed", zhHans: "已失败"),
        "m4.studio.taskState.cancelled": Entry(en: "Cancelled", zhHans: "已取消"),

        // MARK: - Studio Skills & Distilled Script Apps (D11-04)
        "m4.studio.skills.searchPlaceholder": Entry(en: "Filter SKILL.md library", zhHans: "筛选 SKILL.md 技能库"),
        "m4.studio.skills.newSkill": Entry(en: "New Skill", zhHans: "新建技能"),
        "m4.studio.skills.namePlaceholder": Entry(en: "Skill slug (e.g. weekly-brief)", zhHans: "技能标识（如 weekly-brief）"),
        "m4.studio.skills.descPlaceholder": Entry(en: "Short description", zhHans: "简短描述"),
        "m4.studio.skills.bodyPlaceholder": Entry(en: "Markdown instructions", zhHans: "Markdown 指令内容"),
        "m4.studio.skills.emptyTitle": Entry(en: "No Skills Found", zhHans: "未找到技能"),
        "m4.studio.skills.emptyDesc": Entry(
            en: "Create SKILL.md workflows or distill browser trajectories into reusable script apps",
            zhHans: "创建 SKILL.md 工作流，或将浏览器轨迹蒸馏为可复用的脚本应用"
        ),
        "m4.studio.skills.distilledBadge": Entry(en: "Distilled Script App", zhHans: "蒸馏脚本应用"),
        "m4.studio.skills.zeroLlmBadge": Entry(en: "Zero-LLM Replay", zhHans: "零大模型确定性回放"),
        "m4.studio.skills.trustedBadge": Entry(en: "Trusted", zhHans: "已信任"),
        "m4.studio.skills.inspectSteps": Entry(en: "Inspect Steps", zhHans: "查看步骤"),
        "m4.studio.skills.runDeterministic": Entry(en: "Run Deterministically", zhHans: "确定性运行"),
        "m4.studio.skills.publishPreflight": Entry(en: "Publish to Store", zhHans: "发布前安全检查"),
        "m4.studio.skills.preflightTitle": Entry(en: "Store Publish Preflight", zhHans: "商店发布前安全核验"),
        "m4.studio.skills.preflightPassed": Entry(
            en: "Preflight passed — zero secret leaks and valid step structure",
            zhHans: "核验通过：未检测到密钥泄漏且所有步骤结构合规"
        ),
        "m4.studio.skills.preflightFailed": Entry(
            en: "Publish blocked — resolve security or schema findings first",
            zhHans: "发布已拦截：请先修复以下安全或结构问题"
        ),
        "m4.studio.skills.stepsCountFormat": Entry(en: "%d deterministic steps", zhHans: "%d 个确定性步骤"),
        "m4.studio.skills.paramsSection": Entry(en: "Parameters", zhHans: "参数列表"),
        "m4.studio.skills.secretParamBadge": Entry(en: "Secret Vault Param", zhHans: "保险库敏感参数"),
        "m4.studio.skills.replayNoticeIos": Entry(
            en: "Deterministic browser replay dispatches to an authenticated server or remote browser runner because headless browsers are disabled on iOS",
            zhHans: "由于 iOS 禁用本地无头浏览器，确定性回放将通过已登录服务器或远程浏览器执行"
        ),
        "m4.studio.skills.patchSkill": Entry(en: "Patch Markdown", zhHans: "修补内容"),
        "m4.studio.skills.oldTextPlaceholder": Entry(en: "Existing text to match", zhHans: "要匹配的原文"),
        "m4.studio.skills.newTextPlaceholder": Entry(en: "Replacement text", zhHans: "替换后的内容"),

        // MARK: - Studio Memory, Soul, Honcho & Quarantine
        "m4.studio.memory.segmentMemory": Entry(en: "MEMORY.md", zhHans: "智能体记忆"),
        "m4.studio.memory.segmentUser": Entry(en: "USER.md", zhHans: "用户画像"),
        "m4.studio.memory.segmentSoul": Entry(en: "SOUL.md", zhHans: "灵魂设定"),
        "m4.studio.memory.segmentHoncho": Entry(en: "Honcho", zhHans: "辩证洞察"),
        "m4.studio.memory.segmentQuarantine": Entry(en: "Quarantine", zhHans: "隔离审核"),
        "m4.studio.memory.budgetFormat": Entry(en: "%d / %d chars (%d remaining)", zhHans: "%d / %d 字符（剩余 %d）"),
        "m4.studio.memory.addEntryPlaceholder": Entry(en: "Add bounded memory entry", zhHans: "添加记忆条目"),
        "m4.studio.memory.addEntryButton": Entry(en: "Add Entry", zhHans: "添加条目"),
        "m4.studio.memory.clearEntries": Entry(en: "Clear All", zhHans: "清空全部"),
        "m4.studio.memory.rawEditorTitle": Entry(en: "Raw Markdown", zhHans: "原始 Markdown"),
        "m4.studio.memory.soulRoleLabel": Entry(en: "Role & Tone", zhHans: "角色与语气"),
        "m4.studio.memory.soulSaveButton": Entry(en: "Save SOUL.md", zhHans: "保存灵魂设定"),
        "m4.studio.memory.honchoQueryPlaceholder": Entry(en: "Ask dialectic user profile", zhHans: "查询用户辩证画像"),
        "m4.studio.memory.honchoAskButton": Entry(en: "Query Profile", zhHans: "查询画像"),
        "m4.studio.memory.honchoEmpty": Entry(
            en: "No dialectic observations recorded yet",
            zhHans: "暂未记录用户辩证观察"
        ),
        "m4.studio.memory.quarantineEmptyTitle": Entry(en: "Quarantine Queue Clear", zhHans: "隔离队列为空"),
        "m4.studio.memory.quarantineEmptyDesc": Entry(
            en: "Untrusted memory proposals from external chats will appear here for owner review",
            zhHans: "来自外部会话的待审核记忆提议将在此处展示"
        ),
        "m4.studio.memory.quarantinePromote": Entry(en: "Copy to Memory", zhHans: "采纳并写入记忆"),
        "m4.studio.memory.quarantineHostNote": Entry(
            en: "In-place quarantine state transitions will sync automatically once the M6 host resolution RPC is enabled; Copy to Memory writes directly to your bounded store now",
            zhHans: "隔离队列原地结案接口将在 M6 接入，当前可直接“采纳并写入记忆”到你的受限记忆库"
        ),

        // MARK: - Studio Cron & Heartbeat
        "m4.studio.cron.newJob": Entry(en: "New Schedule", zhHans: "新建定时任务"),
        "m4.studio.cron.namePlaceholder": Entry(en: "Schedule name", zhHans: "任务名称"),
        "m4.studio.cron.exprPlaceholder": Entry(en: "5-field cron (e.g. 0 9 * * 1-5)", zhHans: "5 字段 Cron（如 0 9 * * 1-5）"),
        "m4.studio.cron.promptPlaceholder": Entry(en: "Prompt executed on trigger", zhHans: "触发时执行的提示词"),
        "m4.studio.cron.previewButton": Entry(en: "Preview 7 Days", zhHans: "预览 7 天触发"),
        "m4.studio.cron.previewFiresFormat": Entry(en: "%d fires in next 7 days", zhHans: "未来 7 天将触发 %d 次"),
        "m4.studio.cron.previewNextRun": Entry(en: "Next Run", zhHans: "下次触发"),
        "m4.studio.cron.previewWeekend": Entry(en: "Weekend fires", zhHans: "含周末触发"),
        "m4.studio.cron.previewOvernight": Entry(en: "Overnight fires", zhHans: "含夜间触发"),
        "m4.studio.cron.emptyTitle": Entry(en: "No Cron Schedules", zhHans: "暂无定时任务"),
        "m4.studio.cron.emptyDesc": Entry(
            en: "Schedule recurring 5-field cron routines with 7-day execution preview",
            zhHans: "使用标准 5 字段 Cron 表达式编排周期性智能体例程"
        ),

        // MARK: - Studio Approvals & Browser Takeover (D11-03)
        "m4.studio.approvals.dockTitle": Entry(en: "Tool Approval Dock", zhHans: "工具审批台"),
        "m4.studio.approvals.approveOnce": Entry(en: "Approve Once", zhHans: "单次批准"),
        "m4.studio.approvals.approveSession": Entry(en: "Approve Session", zhHans: "本会话批准"),
        "m4.studio.approvals.approvePermanent": Entry(en: "Always Allow", zhHans: "始终允许"),
        "m4.studio.approvals.deny": Entry(en: "Deny", zhHans: "拒绝"),
        "m4.studio.approvals.emptyTitle": Entry(en: "No Approval Records", zhHans: "暂无审批记录"),
        "m4.studio.approvals.emptyDesc": Entry(
            en: "High-risk tool invocations and single-use SHA-256 value-bound approvals appear here",
            zhHans: "高风险工具调用与一次性 SHA-256 值绑定审批令牌将在此展示"
        ),
        "m4.studio.approvals.valueBoundSection": Entry(en: "Value-Bound Approval Ledger", zhHans: "值绑定审批账本"),
        "m4.studio.approvals.issueTokenButton": Entry(en: "Issue Single-Use Token", zhHans: "签发一次性令牌"),
        "m4.studio.approvals.consumeTokenButton": Entry(en: "Verify & Consume", zhHans: "校验并消费"),
        "m4.studio.approvals.actionPlaceholder": Entry(en: "Action (e.g. file_write)", zhHans: "操作类型（如 file_write）"),
        "m4.studio.approvals.targetPlaceholder": Entry(en: "Exact target path or host", zhHans: "精确目标路径或主机"),
        "m4.studio.approvals.spentBadge": Entry(en: "Spent", zhHans: "已消费"),
        "m4.studio.approvals.activeBadge": Entry(en: "Active", zhHans: "有效"),
        "m4.studio.approvals.takeoverSection": Entry(en: "Browser Takeover Controls", zhHans: "浏览器人工接管控制"),
        "m4.studio.approvals.takeOverButton": Entry(en: "Take Over", zhHans: "人工接管"),
        "m4.studio.approvals.resumeAiButton": Entry(en: "Resume AI", zhHans: "恢复 AI 控制"),
        "m4.studio.approvals.cancelAiButton": Entry(en: "Cancel AI", zhHans: "取消 AI 任务"),
        "m4.studio.approvals.takeoverIdle": Entry(
            en: "No active browser automation takeover requests",
            zhHans: "当前无待处理的浏览器接管请求"
        ),
        "m4.studio.approvals.pendingOnlyToggle": Entry(en: "Pending Only", zhHans: "仅看待审批"),

        // MARK: - Studio Sessions & Redacted Trajectory Inspector (D11-05)
        "m4.studio.sessions.searchPlaceholder": Entry(en: "FTS5 / CJK search sessions", zhHans: "FTS5 / 中文全文检索会话"),
        "m4.studio.sessions.newSession": Entry(en: "New Session", zhHans: "新建会话"),
        "m4.studio.sessions.titlePlaceholder": Entry(en: "Session title", zhHans: "会话标题"),
        "m4.studio.sessions.emptyTitle": Entry(en: "No Recorded Sessions", zhHans: "暂无会话记录"),
        "m4.studio.sessions.emptyDesc": Entry(
            en: "Agent sessions, token metrics, and secret-redacted trajectories are stored locally in SQLite WAL",
            zhHans: "智能体会话、Token 统计与脱敏轨迹记录均持久化在本地 SQLite WAL"
        ),
        "m4.studio.sessions.redactionActiveBadge": Entry(
            en: "JWT & Secret Redaction Active",
            zhHans: "JWT 与密钥脱敏已开启"
        ),
        "m4.studio.sessions.redactionCountFormat": Entry(
            en: "%d sensitive tokens masked",
            zhHans: "已脱敏 %d 处敏感凭据"
        ),
        "m4.studio.sessions.searchResultsTitle": Entry(en: "Search Matches", zhHans: "搜索匹配结果"),
        "m4.studio.sessions.tokensFormat": Entry(en: "In %d · Out %d tokens", zhHans: "输入 %d · 输出 %d Token"),

        // MARK: - Multi-Agent Protocol (D14-01 & D14-02)
        "m4.multiAgent.bannerTitle": Entry(en: "Multi-Agent Floor", zhHans: "多智能体发言权"),
        "m4.multiAgent.idleSpeaker": Entry(en: "Floor Open", zhHans: "发言权空闲"),
        "m4.multiAgent.speakingFormat": Entry(en: "%@ speaking", zhHans: "%@ 正在发言"),
        "m4.multiAgent.queuedFormat": Entry(en: "%d queued", zhHans: "%d 个排队中"),
        "m4.multiAgent.leaseRemainingFormat": Entry(en: "%ds lease", zhHans: "租约剩余 %d 秒"),
        "m4.multiAgent.windingDownBadge": Entry(en: "Winding Down", zhHans: "收尾中"),
        "m4.multiAgent.interruptButton": Entry(en: "Interrupt", zhHans: "打断接管"),
        "m4.multiAgent.interruptedToast": Entry(
            en: "User interruption preempted active agent floor",
            zhHans: "已通过用户打断抢占当前智能体发言权"
        ),
        "m4.multiAgent.loopBudgetFormat": Entry(en: "Loop %d/%d", zhHans: "循环 %d/%d 跳"),
        "m4.multiAgent.loopExhaustedBadge": Entry(en: "Budget Exhausted", zhHans: "循环额度耗尽"),
        "m4.multiAgent.workspaceBadgeFormat": Entry(en: "Workspace rev %llu", zhHans: "共享工作区 v%llu"),
        "m4.multiAgent.sheetTitle": Entry(en: "Multi-Agent Room Inspector", zhHans: "多智能体房间检视器"),
        "m4.multiAgent.floorSection": Entry(en: "Floor Control Queue (120s Group Lease)", zhHans: "发言权队列（120 秒群组租约）"),
        "m4.multiAgent.requestFloorButton": Entry(en: "Request Floor (120s)", zhHans: "申请发言权 (120秒)"),
        "m4.multiAgent.releaseFloorButton": Entry(en: "Release Floor", zhHans: "释放发言权"),
        "m4.multiAgent.agentIdPlaceholder": Entry(en: "Agent ID (e.g. rust_bot)", zhHans: "智能体标识（如 rust_bot）"),
        "m4.multiAgent.loopSection": Entry(en: "5-Hop Loop Budget", zhHans: "5 跳循环预算"),
        "m4.multiAgent.recordHopButton": Entry(en: "Record Hop", zhHans: "记录一跳"),
        "m4.multiAgent.resetBudgetButton": Entry(en: "Reset Budget", zhHans: "重置预算"),
        "m4.multiAgent.workspaceSection": Entry(en: "Revision-Fenced Shared Workspace", zhHans: "版本锁定的共享工作区"),
        "m4.multiAgent.workspaceKeyPlaceholder": Entry(en: "Key (e.g. plan.md)", zhHans: "键名（如 plan.md）"),
        "m4.multiAgent.workspaceValuePlaceholder": Entry(en: "Markdown or JSON value", zhHans: "Markdown 或 JSON 内容"),
        "m4.multiAgent.workspaceSaveButton": Entry(en: "Commit Entry (CAS)", zhHans: "乐观锁提交"),
        "m4.multiAgent.workspaceConflictTitle": Entry(
            en: "Workspace Revision Conflict",
            zhHans: "工作区版本冲突"
        ),
        "m4.multiAgent.workspaceConflictReload": Entry(en: "Reload Latest", zhHans: "加载最新版本"),
        "m4.multiAgent.workspaceConflictOverwrite": Entry(en: "Overwrite Latest", zhHans: "覆盖写入"),
        "m4.multiAgent.wakeGateSection": Entry(
            en: "Group Wake Gate & Stay-Silent Diagnostics",
            zhHans: "群组唤醒门控与静默 [PASS] 诊断"
        ),
        "m4.multiAgent.wakeGateMessagePlaceholder": Entry(
            en: "Test message (e.g. @rust_bot你好 or ok thanks)",
            zhHans: "测试群消息（如 @rust_bot你好 或 收到）"
        ),
        "m4.multiAgent.wakeGateRunButton": Entry(en: "Evaluate Wake Gate", zhHans: "评估唤醒门控"),
        "m4.multiAgent.outboundDraftPlaceholder": Entry(
            en: "Test bot draft (e.g. [PASS] or reply text)",
            zhHans: "测试机器人草稿（如 [PASS] 或回复正文）"
        ),
        "m4.multiAgent.outboundReviewButton": Entry(en: "Review Outbound", zhHans: "校验外发回复"),
        "m4.multiAgent.passSentinelYes": Entry(en: "Stay-Silent [PASS] Detected", zhHans: "检测到静默哨兵 [PASS]"),
        "m4.multiAgent.passSentinelNo": Entry(en: "Normal Reply Content", zhHans: "常规回复内容"),
        "m4.multiAgent.wakeReason.explicitMention": Entry(en: "Explicit @Mention", zhHans: "显式 @提及"),
        "m4.multiAgent.wakeReason.mentionAll": Entry(en: "Mention All (@all)", zhHans: "提及全体 (@所有人)"),
        "m4.multiAgent.wakeReason.replyToBot": Entry(en: "Reply to Bot", zhHans: "回复目标智能体"),
        "m4.multiAgent.wakeReason.singleBotRoom": Entry(en: "Single-Bot Room", zhHans: "单智能体群组"),
        "m4.multiAgent.wakeReason.relevanceMatch": Entry(en: "Relevance Match", zhHans: "语义相关性匹配"),
        "m4.multiAgent.wakeReason.primaryFallback": Entry(en: "Primary Fallback", zhHans: "主回复兜底唤醒"),
        "m4.multiAgent.wakeReason.budgetExhausted": Entry(en: "Loop Budget Exhausted", zhHans: "循环额度已耗尽"),
        "m4.multiAgent.wakeReason.noRelevantCandidate": Entry(en: "Low-Signal / Stay Silent", zhHans: "低信号或无相关候选（静默）"),
        "m4.multiAgent.wakeReason.emptyCandidates": Entry(en: "No Room Bots", zhHans: "群组无候选智能体"),

        // MARK: - Magpie AI Provider Gateway (Shared for Worker 2 M4b)
        "m4.magpie.title": Entry(en: "AI Providers & Magpie", zhHans: "AI 供应商与喜鹊网关"),
        "m4.magpie.providersSection": Entry(en: "Configured Providers", zhHans: "已配置供应商"),
        "m4.magpie.presetsSection": Entry(en: "Provider Presets", zhHans: "预设供应商目录"),
        "m4.magpie.subscriptionsSection": Entry(en: "Active Subscriptions", zhHans: "已连接订阅"),
        "m4.magpie.addProvider": Entry(en: "Add Provider", zhHans: "添加供应商"),
        "m4.magpie.probeHealth": Entry(en: "Probe Health", zhHans: "探测连通性"),
        "m4.magpie.vaultOAuthSync": Entry(en: "Sync Desktop OAuth (Vault)", zhHans: "从保险库同步桌面 OAuth"),
        "m4.magpie.webOAuthButton": Entry(en: "Sign In with Browser OAuth", zhHans: "浏览器 OAuth 授权登录"),
        "m4.magpie.routingSection": Entry(en: "Smart Routing Groups", zhHans: "智能路由分组"),
        "m4.magpie.routingToggle": Entry(en: "Enable Smart Routing", zhHans: "启用智能路由"),
        "m4.magpie.addGroup": Entry(en: "New Routing Group", zhHans: "新建路由分组"),
        "m4.magpie.resolveRoute": Entry(en: "Test Route Resolution", zhHans: "测试路由解析"),
        "m4.magpie.circuitClosed": Entry(en: "Circuit Closed", zhHans: "熔断器正常"),
        "m4.magpie.circuitOpen": Entry(en: "Circuit Resting", zhHans: "熔断冷却中"),
        "m4.magpie.circuitHalfOpen": Entry(en: "Circuit Half-Open", zhHans: "熔断半开探测"),
        "m4.magpie.piiSection": Entry(en: "PII Privacy Masking", zhHans: "PII 隐私脱敏"),
        "m4.magpie.piiToggle": Entry(en: "Mask Secrets & Personal Data", zhHans: "自动脱敏密钥与个人隐私"),
        "m4.magpie.piiSecretsToggle": Entry(en: "Mask API Keys, JWT & PEM", zhHans: "脱敏 API 密钥、JWT 与私钥"),
        "m4.magpie.piiPersonalToggle": Entry(en: "Mask Email & Payment Cards", zhHans: "脱敏邮箱与银行卡号"),
        "m4.magpie.piiPreviewPlaceholder": Entry(en: "Enter text to preview redaction", zhHans: "输入测试文本预览脱敏效果"),
        "m4.magpie.piiPreviewButton": Entry(en: "Preview Masking", zhHans: "预览脱敏结果"),
        "m4.magpie.importSection": Entry(en: "Import Providers", zhHans: "导入供应商配置"),
        "m4.magpie.importLinkPlaceholder": Entry(en: "Deep link URL", zhHans: "导入深链接地址"),
        "m4.magpie.importDotenvPlaceholder": Entry(en: "Paste .env API keys", zhHans: "粘贴 .env 环境变量配置"),
        "m4.magpie.importButton": Entry(en: "Import", zhHans: "解析导入"),
        "m4.magpie.usageSection": Entry(en: "Usage & Cost Metrics", zhHans: "用量与费用统计"),
        "m4.magpie.followSettings": Entry(en: "Follow Active Model", zhHans: "跟随全局默认模型"),
        "m4.magpie.modelMismatchWarning": Entry(en: "Differs from active model", zhHans: "与全局设置模型不一致"),
        "m4.magpie.instantSwitchToast": Entry(en: "Switched active subscription model", zhHans: "已立即切换当前订阅模型"),
        "m4.magpie.noDotfileScanNotice": Entry(
            en: "Local home-directory credential scans are disabled on iOS; sync desktop AI OAuth safely via Zero-Knowledge Vault",
            zhHans: "iOS 禁止扫描本地主目录凭据，请通过零知识保险库安全同步桌面端 AI OAuth 凭据"
        ),

        // MARK: - Coding Agents Mobile Client (Shared for Worker 2 M4b)
        "m4.coding.title": Entry(en: "Coding Agents", zhHans: "编程智能体"),
        "m4.coding.providersSection": Entry(en: "Supported Providers", zhHans: "支持的编程引擎"),
        "m4.coding.botsSection": Entry(en: "Coding Bots", zhHans: "编程机器人"),
        "m4.coding.botfatherSection": Entry(en: "BotFather Natural Language Setup", zhHans: "BotFather 自然语言创建"),
        "m4.coding.botfatherPlaceholder": Entry(en: "Describe your coding agent", zhHans: "用自然语言描述你想创建的编程智能体"),
        "m4.coding.botfatherCreateButton": Entry(en: "Create via BotFather", zhHans: "通过 BotFather 创建"),
        "m4.coding.probeHost": Entry(en: "Probe Remote Coding Host", zhHans: "探测远程编程宿主"),
        "m4.coding.createPR": Entry(en: "Create Pull Request", zhHans: "创建合并请求 (PR)"),
        "m4.coding.checkpointButton": Entry(en: "Checkpoint", zhHans: "检查点"),
        "m4.coding.saveCheckpoint": Entry(en: "Save Checkpoint", zhHans: "保存检查点"),
        "m4.coding.rollbackCheckpoint": Entry(en: "Rollback Checkpoint", zhHans: "回滚至检查点"),
        "m4.coding.diffCardTitle": Entry(en: "Streaming Diff", zhHans: "实时代码差异"),
        "m4.coding.worktreeLabel": Entry(en: "Worktree", zhHans: "工作树"),
        "m4.coding.permissionAsk": Entry(en: "Ask", zhHans: "每次询问"),
        "m4.coding.permissionAutoEdit": Entry(en: "Auto Edit", zhHans: "自动编辑"),
        "m4.coding.permissionFullAccess": Entry(en: "Full Access", zhHans: "完全访问"),
        "m4.coding.remoteOnlyNotice": Entry(
            en: "Local coding CLI subprocesses are disabled in the iOS sandbox; turns execute on your bound desktop or cloud coding host",
            zhHans: "iOS 沙盒禁用本地命令行子进程，编程任务将转发至你绑定的桌面端或云端编码宿主执行"
        ),
        "m4.coding.accountIsolationBadge": Entry(
            en: "Account-Isolated Store",
            zhHans: "按账号隔离存储"
        ),

        // MARK: - Official OAuth 2.1 Remote MCP Console (Shared for Worker 2 M4b)
        "m4.mcp.title": Entry(en: "Official OAuth 2.1 MCP", zhHans: "官方 OAuth 2.1 MCP 控制台"),
        "m4.mcp.grantsSection": Entry(en: "Active OAuth 2.1 Grants", zhHans: "活跃 OAuth 2.1 授权"),
        "m4.mcp.qrApproveSection": Entry(en: "QR-Poll Login Approval", zhHans: "扫码轮询登录审批"),
        "m4.mcp.qrScanButton": Entry(en: "Scan QR Code", zhHans: "扫描二维码"),
        "m4.mcp.qrManualPlaceholder": Entry(en: "Paste pair token (pair_...)", zhHans: "粘贴配对令牌（pair_...）"),
        "m4.mcp.approveGrantButton": Entry(en: "Approve MCP Grant", zhHans: "批准 MCP 授权"),
        "m4.mcp.revokeGrantButton": Entry(en: "Revoke Grant", zhHans: "撤销授权"),
        "m4.mcp.killSwitchButton": Entry(en: "Emergency Kill-Switch", zhHans: "紧急一键断开全部会话"),
        "m4.mcp.killSwitchConfirm": Entry(
            en: "Disconnect every active remote MCP session immediately?",
            zhHans: "确认立即强制断开所有活跃的远程 MCP 会话吗？"
        ),
        "m4.mcp.auditSection": Entry(en: "Redacted Tool Audit Log", zhHans: "脱敏工具审计日志"),
        "m4.mcp.auditEmpty": Entry(
            en: "No MCP tool audit events recorded in this session",
            zhHans: "当前会话暂无 MCP 工具审计记录"
        ),
        "m4.mcp.serversSection": Entry(en: "Remote MCP Servers", zhHans: "远程 MCP 服务器"),
        "m4.mcp.localToolsSection": Entry(en: "Embedded Registry Tools", zhHans: "本地注册表工具"),
        "m4.mcp.stdioBlockedNotice": Entry(
            en: "Local stdio MCP subprocesses are blocked on iOS; use HTTPS Streamable HTTP or SSE servers",
            zhHans: "iOS 沙盒禁用本地 stdio 子进程服务器，请使用 HTTPS Streamable HTTP 或 SSE 远程地址"
        ),
        "m4.mcp.officialCatalogFormat": Entry(
            en: "%d official tools across %d OAuth scopes",
            zhHans: "涵盖 %d 项官方工具与 %d 项 OAuth 权限范围"
        ),
    ]

    public static var allKeys: [String] {
        table.keys.sorted()
    }

    public static func text(_ key: String, locale: FabushiLocale) -> String {
        guard let entry = table[key] else {
            return key
        }
        switch locale {
        case .en:
            return entry.en
        case .zhHans:
            return entry.zhHans
        }
    }

    public static func format(
        _ key: String,
        locale: FabushiLocale,
        _ arguments: CVarArg...
    ) -> String {
        let template = text(key, locale: locale)
        return String(format: template, locale: locale.foundationLocale, arguments: arguments)
    }
}
