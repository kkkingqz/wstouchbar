use touchbar_sets::SetFile;
const HEADER: &str =
    "version=1\nid='picker'\nlabel='Picker'\n[layout]\nkind='scrubber'\nid='list'\n";
const LIST: &str =
    "repeat_from='rows'\nrepeat_key='id'\ntemplate={kind='text',text_from='record.label'}\n";
#[test]
fn scrubber_accepts_record_template() {
    let set = SetFile::from_toml(
        "picker.toml",
        &format!("{HEADER}{LIST}on_selected='choose'"),
    )
    .unwrap();
    assert_eq!(set.layout.repeat_from.as_deref(), Some("rows"));
}
#[test]
fn scrubber_requires_source_key_and_template() {
    let error = SetFile::from_toml("picker.toml", HEADER).unwrap_err();
    assert_eq!(error.path, "layout.repeat_from");
}
#[test]
fn scrubber_rejects_scalar_value() {
    for property in ["value=0.5", "value_from='position'"] {
        let error =
            SetFile::from_toml("picker.toml", &format!("{HEADER}{LIST}{property}")).unwrap_err();
        assert_eq!(
            error.path,
            format!("layout.{}", property.split('=').next().unwrap())
        );
        assert!(error.message.contains("scrubber"));
    }
}
#[test]
fn scrubber_template_cannot_own_interactions() {
    for template in [
        "{kind='button',id='b',on_activate='go'}",
        "{kind='slider'}",
        "{kind='key',key='f1'}",
        "{kind='row',children=[{kind='button'}]}",
    ] {
        let error = SetFile::from_toml(
            "picker.toml",
            &format!("{HEADER}repeat_from='rows'\nrepeat_key='id'\ntemplate={template}"),
        )
        .unwrap_err();
        assert!(error.path.starts_with("layout.template"), "{error}");
        assert!(error.message.contains("visual"), "{error}");
    }
}
