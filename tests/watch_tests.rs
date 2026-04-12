#[test]
fn scan_ai_markers() {
    use rustyclaw::watch::scan_markers;
    let content = r#"
fn main() {
    // AI: add error handling here
    let x = 1;
    // TODO: clean up later
    // AI: refactor this block
}
"#;
    let markers = scan_markers(content, &["AI:", "AGENT:"]);
    assert_eq!(markers.len(), 2);
    assert!(markers[0].text.contains("add error handling"));
    assert!(markers[1].text.contains("refactor this block"));
    assert_eq!(markers[0].line, 3);
    assert_eq!(markers[1].line, 6);
}

#[test]
fn scan_no_markers() {
    use rustyclaw::watch::scan_markers;
    let content = "fn main() { let x = 1; }";
    let markers = scan_markers(content, &["AI:", "AGENT:"]);
    assert!(markers.is_empty());
}

#[test]
fn scan_custom_markers() {
    use rustyclaw::watch::scan_markers;
    let content = "// FIXME: broken\n// AGENT: implement auth";
    let markers = scan_markers(content, &["AGENT:"]);
    assert_eq!(markers.len(), 1);
    assert!(markers[0].text.contains("implement auth"));
}
