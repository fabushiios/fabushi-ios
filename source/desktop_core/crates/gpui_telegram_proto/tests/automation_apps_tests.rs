use gpui_telegram_proto::automation_apps::{
    compute_automation_manifest_sha256, AutomationAppDetail, AutomationAppParameterDto,
    AutomationAppSummary, InstallAutomationAppResponse, InstalledAutomationApp,
    ListAutomationAppsQuery, ListAutomationAppsResponse, ListInstalledAutomationAppsResponse,
    PublishAutomationAppRequest, RecordAutomationRunRequest, RecordAutomationRunResponse,
    UninstallAutomationAppResponse, AUTOMATION_APP_SCHEMA_VERSION_V1,
};
use serde_json::json;

#[test]
fn test_publish_request_defaults_and_lossless_roundtrip() {
    let manifest = json!({
        "schema_version": AUTOMATION_APP_SCHEMA_VERSION_V1,
        "app_id": "x-post-scheduler",
        "slug": "x-post-scheduler",
        "name": "X Post Publisher",
        "version": "1.0.0",
        "description": "Publishes a post on X",
        "category": "social",
        "target_domains": ["x.com"],
        "parameters": [
            {
                "name": "post_text",
                "description": "Text to publish",
                "required": true,
                "is_secret": false
            },
            {
                "name": "auth_token",
                "description": "Session token",
                "required": true,
                "is_secret": true
            }
        ],
        "steps": [
            { "kind": "navigate", "url_template": "https://x.com/compose/post" },
            { "kind": "scroll", "delta_x": 0.0, "delta_y": 120.5 },
            {
                "kind": "fill",
                "locator": { "test_id": "tweetTextarea_0" },
                "value_template": "{{post_text}}"
            }
        ],
        "required_capabilities": ["browser"],
        "author_id": "1001",
        "unknown_future_field": { "nested": [1, 2, 3] }
    });

    // Omitting `is_published` defaults to `true`.
    let raw = json!({
        "manifest": manifest,
        "changelog": "Initial release"
    });
    let req: PublishAutomationAppRequest = serde_json::from_value(raw).unwrap();
    assert!(req.is_published);
    assert_eq!(req.changelog.as_deref(), Some("Initial release"));
    assert_eq!(
        req.manifest["unknown_future_field"]["nested"],
        json!([1, 2, 3])
    );

    let reserialized = serde_json::to_string(&req).unwrap();
    let decoded: PublishAutomationAppRequest = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_store_dtos_roundtrip_and_sha256() {
    let manifest = json!({
        "schema_version": 1,
        "app_id": "github-star",
        "slug": "github-star",
        "name": "Star Repo",
        "version": "1.2.0",
        "description": "Stars a repo",
        "category": "developer",
        "target_domains": ["github.com"],
        "parameters": [],
        "steps": [{ "kind": "navigate", "url_template": "https://github.com" }]
    });
    let manifest_str = serde_json::to_string(&manifest).unwrap();
    let sha = compute_automation_manifest_sha256(&manifest_str);
    assert_eq!(sha.len(), 64);
    assert_eq!(sha, compute_automation_manifest_sha256(&manifest_str));
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));

    let summary = AutomationAppSummary {
        app_id: "github-star".into(),
        slug: "github-star".into(),
        author_user_id: 42,
        author_username: Some("alice".into()),
        name: "Star Repo".into(),
        description: "Stars a repo".into(),
        category: "developer".into(),
        target_domains: vec!["github.com".into()],
        latest_version: "1.2.0".into(),
        required_capabilities: vec!["browser".into()],
        step_count: 1,
        parameters: vec![AutomationAppParameterDto {
            name: "repo".into(),
            description: "owner/name".into(),
            required: true,
            default_value: None,
            is_secret: false,
        }],
        install_count: 10,
        run_count: 25,
        is_published: true,
        is_installed: true,
        created_at_ms: 1_700_000_000_000,
        updated_at_ms: 1_700_000_005_000,
    };

    let list_resp = ListAutomationAppsResponse {
        apps: vec![summary],
        total: 1,
        limit: 50,
        offset: 0,
    };
    let list_json = serde_json::to_string(&list_resp).unwrap();
    assert_eq!(
        serde_json::from_str::<ListAutomationAppsResponse>(&list_json).unwrap(),
        list_resp
    );

    let detail = AutomationAppDetail {
        app_id: "github-star".into(),
        slug: "github-star".into(),
        author_user_id: 42,
        author_username: Some("alice".into()),
        name: "Star Repo".into(),
        description: "Stars a repo".into(),
        category: "developer".into(),
        target_domains: vec!["github.com".into()],
        latest_version: "1.2.0".into(),
        versions: vec!["1.2.0".into(), "1.0.0".into()],
        manifest: manifest.clone(),
        content_sha256: sha.clone(),
        install_count: 10,
        run_count: 25,
        is_published: true,
        is_installed: true,
        installed_version: Some("1.2.0".into()),
        created_at_ms: 1_700_000_000_000,
        updated_at_ms: 1_700_000_005_000,
    };
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert_eq!(
        serde_json::from_str::<AutomationAppDetail>(&detail_json).unwrap(),
        detail
    );

    let installed = InstalledAutomationApp {
        app_id: "github-star".into(),
        slug: "github-star".into(),
        name: "Star Repo".into(),
        description: "Stars a repo".into(),
        category: "developer".into(),
        target_domains: vec!["github.com".into()],
        installed_version: "1.2.0".into(),
        latest_version: "1.2.0".into(),
        manifest,
        content_sha256: sha,
        installed_at_ms: 1_700_000_001_000,
        last_run_at_ms: Some(1_700_000_009_000),
        local_run_count: 3,
    };

    let install_resp = InstallAutomationAppResponse {
        ok: true,
        app: installed.clone(),
        install_count: 11,
        newly_installed: true,
    };
    assert_eq!(
        serde_json::from_str::<InstallAutomationAppResponse>(
            &serde_json::to_string(&install_resp).unwrap()
        )
        .unwrap(),
        install_resp
    );

    let installed_list = ListInstalledAutomationAppsResponse {
        apps: vec![installed],
        total: 1,
    };
    assert_eq!(
        serde_json::from_str::<ListInstalledAutomationAppsResponse>(
            &serde_json::to_string(&installed_list).unwrap()
        )
        .unwrap(),
        installed_list
    );

    let uninstall_resp = UninstallAutomationAppResponse {
        ok: true,
        app_id: "github-star".into(),
        removed: true,
    };
    assert_eq!(
        serde_json::from_str::<UninstallAutomationAppResponse>(
            &serde_json::to_string(&uninstall_resp).unwrap()
        )
        .unwrap(),
        uninstall_resp
    );

    let run_req = RecordAutomationRunRequest {
        version: Some("1.2.0".into()),
        success: true,
        steps_executed: 4,
        total_duration_ms: 420,
        used_llm: false,
        error: None,
    };
    assert_eq!(
        serde_json::from_str::<RecordAutomationRunRequest>(
            &serde_json::to_string(&run_req).unwrap()
        )
        .unwrap(),
        run_req
    );

    let run_resp = RecordAutomationRunResponse {
        ok: true,
        app_id: "github-star".into(),
        run_count: 26,
        last_run_at_ms: 1_700_000_009_500,
    };
    assert_eq!(
        serde_json::from_str::<RecordAutomationRunResponse>(
            &serde_json::to_string(&run_resp).unwrap()
        )
        .unwrap(),
        run_resp
    );

    let query = ListAutomationAppsQuery {
        q: Some("star".into()),
        category: Some("developer".into()),
        domain: Some("github.com".into()),
        limit: Some(25),
        offset: Some(0),
    };
    assert_eq!(
        serde_json::from_str::<ListAutomationAppsQuery>(&serde_json::to_string(&query).unwrap())
            .unwrap(),
        query
    );
}
