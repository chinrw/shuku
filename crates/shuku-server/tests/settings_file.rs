mod common;

use common::Fixture;
use serde_json::json;

#[tokio::test]
async fn settings_file_overrides_only_the_fields_it_names() {
    let fixture = Fixture::new();
    let app = fixture.app().await;
    let router = app.router();
    // 先在控制台改两项，文件只覆盖其中一项。
    common::request(
        &router,
        "PUT",
        "/api/admin/settings/scan",
        Some(json!({"scan_hidden": true, "scan_interval_hours": 6})),
        &[],
    )
    .await;
    drop(router);
    drop(app);

    let file = fixture.root.path().join("settings.json");
    std::fs::write(
        &file,
        json!({
            "scan": {"scan_interval_hours": 24, "series_from_top_dir": true, "extra_skip": ["@eaDir"]},
            "server": {"name": "漫画", "detect_webtoons": false, "allowed_hosts": ["nas.example.ts.net"]},
        })
        .to_string(),
    )
    .unwrap();
    let app = fixture.app().await;
    app.apply_settings_file(&file).unwrap();
    let router = app.router();
    assert_eq!(
        common::json(&router, "/api/admin/settings/scan").await,
        json!({
            "watch": true, "scan_hidden": true, "scan_interval_hours": 24,
            "extra_skip": ["@eaDir"], "series_from_top_dir": true,
        })
    );
    assert_eq!(
        common::json(&router, "/api/admin/settings/server").await,
        json!({"name": "漫画", "detect_webtoons": false, "allowed_hosts": ["nas.example.ts.net"]})
    );
    // 写回了库里，不带文件重启也还在。
    drop(router);
    drop(app);
    let app = fixture.app().await;
    let settings = common::json(&app.router(), "/api/admin/settings/scan").await;
    assert_eq!(settings["scan_interval_hours"], 24);

    // 和控制台一样校验，不认识的字段和超出范围的值都让启动失败。
    for invalid in [
        json!({"scan": {"content_dir": "/"}}),
        json!({"scan": {"scan_interval_hours": 100000}}),
        json!({"auth": {"require_token": false}}),
    ] {
        std::fs::write(&file, invalid.to_string()).unwrap();
        assert!(app.apply_settings_file(&file).is_err(), "{invalid}");
    }
    assert!(
        app.apply_settings_file(&fixture.root.path().join("missing.json"))
            .is_err()
    );
}
