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
