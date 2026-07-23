use boa_engine::{Context, Source};
use std::fs;
use std::path::Path;

fn context() -> Context {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/support-core.js");
    let source = fs::read_to_string(path).expect("read ui/support-core.js");
    let mut context = Context::default();
    context
        .eval(Source::from_bytes(&source))
        .expect("support-core.js evaluates without DOM or Tauri globals");
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
fn workflow_transitions_are_explicit_and_retryable() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "JSON.stringify([\
               SupportCore.transition('idle', 'prepare_started'),\
               SupportCore.transition('preparing', 'prepare_succeeded'),\
               SupportCore.transition('prepared', 'upload_started'),\
               SupportCore.transition('uploading', 'upload_failed'),\
               SupportCore.transition('prepared', 'upload_started'),\
               SupportCore.transition('uploading', 'upload_cancelled'),\
               SupportCore.transition('prepared', 'discarded'),\
               SupportCore.transition('uploading', 'upload_succeeded')\
             ])",
        ),
        r#"["preparing","prepared","uploading","prepared","uploading","prepared","idle","success"]"#
    );
}

#[test]
fn each_phase_exposes_at_most_one_transient_panel() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "JSON.stringify(['idle', 'preparing', 'prepared', 'uploading', 'success'].map((phase) => {\
               const view = SupportCore.view(phase, { uploadAvailable: true, settingsDirty: false });\
               return {\
                 phase,\
                 panels: [view.showPreparing, view.showPreview, view.showProgress, view.showSuccess]\
                   .filter(Boolean).length,\
                 locked: view.descriptionLocked\
               };\
             }))",
        ),
        r#"[{"phase":"idle","panels":0,"locked":false},{"phase":"preparing","panels":1,"locked":true},{"phase":"prepared","panels":1,"locked":true},{"phase":"uploading","panels":1,"locked":true},{"phase":"success","panels":1,"locked":false}]"#
    );
}

#[test]
fn unavailable_upload_keeps_local_bundle_actions_honest() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "JSON.stringify({\
               unavailable: SupportCore.view('prepared', { uploadAvailable: false, settingsDirty: false }),\
               available: SupportCore.view('prepared', { uploadAvailable: true, settingsDirty: true })\
             })",
        ),
        r#"{"unavailable":{"showPreparing":false,"showPreview":true,"showProgress":false,"showSuccess":false,"descriptionLocked":true,"prepareDisabled":true,"sendDisabled":true,"settingsSaveVisible":false,"settingsSaveLabel":"Save Settings"},"available":{"showPreparing":false,"showPreview":true,"showProgress":false,"showSuccess":false,"descriptionLocked":true,"prepareDisabled":true,"sendDisabled":false,"settingsSaveVisible":true,"settingsSaveLabel":"Save Other Changes"}}"#
    );
}
