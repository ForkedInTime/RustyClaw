use serde_json::json;

#[test]
fn parse_ax_tree_nodes() {
    use rustyclaw::browser::snapshot::parse_ax_nodes;

    let nodes = json!([
        {"nodeId": "1", "role": {"value": "RootWebArea"}, "name": {"value": "Test Page"}, "childIds": ["2", "3"]},
        {"nodeId": "2", "role": {"value": "link"}, "name": {"value": "Home"}, "childIds": []},
        {"nodeId": "3", "role": {"value": "button"}, "name": {"value": "Submit"}, "childIds": []},
    ]);

    let (tree_text, ref_map) = parse_ax_nodes(&nodes);
    assert!(tree_text.contains("@e1"));
    assert!(tree_text.contains("link"));
    assert!(tree_text.contains("Home"));
    assert!(tree_text.contains("@e2"));
    assert!(tree_text.contains("button"));
    assert!(tree_text.contains("Submit"));
    assert_eq!(ref_map.len(), 2);
}

#[test]
fn filter_non_interactive_nodes() {
    use rustyclaw::browser::snapshot::parse_ax_nodes;

    let nodes = json!([
        {"nodeId": "1", "role": {"value": "RootWebArea"}, "name": {"value": ""}, "childIds": ["2", "3"]},
        {"nodeId": "2", "role": {"value": "generic"}, "name": {"value": ""}, "childIds": []},
        {"nodeId": "3", "role": {"value": "button"}, "name": {"value": "Click Me"}, "childIds": []},
    ]);

    let (tree_text, ref_map) = parse_ax_nodes(&nodes);
    assert_eq!(ref_map.len(), 1);
    assert!(tree_text.contains("Click Me"));
    assert!(!tree_text.contains("generic"));
}

#[test]
fn element_score_button_higher_than_generic() {
    use rustyclaw::browser::element::score_element;
    let button_score = score_element("button", true, true, true);
    let generic_score = score_element("generic", false, false, false);
    assert!(button_score > generic_score);
}
