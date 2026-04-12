//! Accessibility tree extraction and @ref numbering.
//!
//! Uses Chrome's Accessibility.getFullAXTree CDP command. Filters by
//! interactive/content roles, assigns integer refs (@e1, @e2, ...).

use std::collections::HashMap;

/// ARIA roles that are interactive (buttons, links, inputs, etc.)
const INTERACTIVE_ROLES: &[&str] = &[
    "button", "link", "textbox", "searchbox", "checkbox", "radio",
    "combobox", "listbox", "option", "menuitem", "menuitemcheckbox",
    "menuitemradio", "tab", "switch", "slider", "spinbutton",
    "scrollbar", "treeitem", "gridcell", "columnheader", "rowheader",
];

/// ARIA roles that carry content worth showing
const CONTENT_ROLES: &[&str] = &[
    "heading", "img", "image", "paragraph", "list", "listitem",
    "table", "row", "cell", "navigation", "banner", "main",
    "complementary", "contentinfo", "form", "region", "alert",
    "status", "dialog",
];

/// Roles to always skip
const SKIP_ROLES: &[&str] = &[
    "none", "presentation", "generic", "RootWebArea", "InlineTextBox",
    "LineBreak",
];

/// Parse CDP Accessibility.getFullAXTree node array into a text representation
/// and a ref map (ref_string -> nodeId as i64).
pub fn parse_ax_nodes(nodes: &serde_json::Value) -> (String, HashMap<String, i64>) {
    let mut ref_map = HashMap::new();
    let mut lines = Vec::new();
    let mut ref_counter = 1u32;

    let arr = match nodes.as_array() {
        Some(a) => a,
        None => return (String::new(), ref_map),
    };

    for node in arr {
        let role = node["role"]["value"].as_str().unwrap_or("");
        let name = node["name"]["value"].as_str().unwrap_or("").trim();
        let node_id_str = node["nodeId"].as_str().unwrap_or("0");
        let node_id: i64 = node_id_str.parse().unwrap_or(0);

        if SKIP_ROLES.contains(&role) {
            continue;
        }

        let is_interactive = INTERACTIVE_ROLES.contains(&role);
        let is_content = CONTENT_ROLES.contains(&role);
        if !is_interactive && !is_content {
            continue;
        }
        if name.is_empty() && !is_interactive {
            continue;
        }

        let ref_str = format!("@e{ref_counter}");
        ref_map.insert(ref_str.clone(), node_id);

        let display_name = if name.is_empty() {
            format!("{ref_str} [{role}]")
        } else {
            format!("{ref_str} [{role}] \"{name}\"")
        };
        lines.push(display_name);
        ref_counter += 1;
    }

    (lines.join("\n"), ref_map)
}

/// Take a full accessibility snapshot via CDP.
pub async fn take_snapshot(
    client: &super::cdp::CdpClient,
) -> anyhow::Result<(String, HashMap<String, i64>)> {
    let result = client
        .send("Accessibility.getFullAXTree", serde_json::json!({}))
        .await?;
    let nodes = &result["nodes"];
    Ok(parse_ax_nodes(nodes))
}
