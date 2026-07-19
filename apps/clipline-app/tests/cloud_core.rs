use boa_engine::{Context, Source};
use std::fs;
use std::path::Path;

fn context() -> Context {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/cloud-core.js");
    let source = fs::read_to_string(path).expect("read ui/cloud-core.js");
    let mut context = Context::default();
    context
        .eval(Source::from_bytes(&source))
        .expect("cloud-core.js evaluates without DOM or Tauri globals");
    context
}

fn eval(context: &mut Context, expression: &str) -> String {
    context
        .eval(Source::from_bytes(expression))
        .unwrap_or_else(|error| panic!("eval `{expression}`: {error}"))
        .to_string(context)
        .expect("stringify result")
        .to_std_string_escaped()
}

#[test]
fn request_gate_rejects_superseded_and_invalidated_requests() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "const gate = CloudCore.createRequestGate();\
             const first = gate.begin('host|user-a|credential-a');\
             const second = gate.begin('host|user-a|credential-a');\
             JSON.stringify([\
               gate.isCurrent(first, 'host|user-a|credential-a'),\
               gate.isCurrent(second, 'host|user-a|credential-a'),\
               gate.isCurrent(second, 'host|user-b|credential-b'),\
               gate.invalidate(),\
               gate.isCurrent(second, 'host|user-a|credential-a')\
             ])",
        ),
        "[false,true,false,3,false]"
    );
}

#[test]
fn account_change_invalidation_preserves_replacement_request_ownership() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "const gate = CloudCore.createRequestGate();\
             const first = gate.begin('host|user-a|credential-a');\
             gate.invalidate();\
             const replacement = gate.begin('host|user-b|credential-b');\
             JSON.stringify([\
               gate.isCurrent(first, 'host|user-a|credential-a'),\
               gate.isCurrent(first, 'host|user-b|credential-b'),\
               gate.isCurrent(replacement, 'host|user-b|credential-b')\
             ])",
        ),
        "[false,false,true]"
    );
}

#[test]
fn account_key_is_stable_and_account_scoped() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "CloudCore.accountKey({\
               host_url: 'https://clips.example',\
               connected_user_id: 'user-7',\
               credential_target: 'credential-7'\
             })",
        ),
        "https://clips.example|user-7|credential-7"
    );
}

#[test]
fn backend_cloud_merge_preserves_draft_preferences_and_unrelated_settings() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "const draft = { replay_window_s: 91, audio: { mic_enabled: true }, cloud: {\
               host_url: 'https://old.example', connected_user_id: 'old-user',\
               connected_username: 'old-name', connected_display_name: null,\
               credential_target: 'old-credential', public_url: null,\
               default_visibility: 'unlisted', delete_local_after_upload: true,\
               auto_upload_rules: false, uploads: { old: { path: 'old.mp4' } }\
             } };\
             const backend = { cloud: {\
               host_url: 'https://new.example', connected_user_id: 'new-user',\
               connected_username: 'new-name', connected_display_name: 'New Name',\
               credential_target: 'new-credential', public_url: 'https://clips.example',\
               default_visibility: 'private', delete_local_after_upload: false,\
               auto_upload_rules: true, uploads: { fresh: { path: 'fresh.mp4' } }\
             } };\
             const merged = CloudCore.mergeBackendCloudSettings(draft, backend);\
             JSON.stringify({\
               replay_window_s: merged.replay_window_s, audio: merged.audio,\
               cloud: merged.cloud, uploadsCloned: merged.cloud.uploads !== backend.cloud.uploads\
             })",
        ),
        r#"{"replay_window_s":91,"audio":{"mic_enabled":true},"cloud":{"host_url":"https://new.example","connected_user_id":"new-user","connected_username":"new-name","connected_display_name":"New Name","credential_target":"new-credential","public_url":"https://clips.example","default_visibility":"unlisted","delete_local_after_upload":true,"auto_upload_rules":false,"uploads":{"fresh":{"path":"fresh.mp4"}}},"uploadsCloned":true}"#
    );
}

#[test]
fn plain_http_confirmation_requires_checked_exact_origin() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "JSON.stringify([\
               CloudCore.plainHttpConfirmed('http://clips.local', 'http://clips.local', true),\
               CloudCore.plainHttpConfirmed('http://clips.local', 'http://clips.local', false),\
               CloudCore.plainHttpConfirmed('http://clips.local', 'http://other.local', true),\
               CloudCore.plainHttpConfirmed('http://clips.local:8080', 'http://clips.local', true),\
               CloudCore.plainHttpConfirmed('', '', true)\
             ])",
        ),
        "[true,false,false,false,false]"
    );
}
