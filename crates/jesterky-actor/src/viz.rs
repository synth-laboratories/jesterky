//! Terminal rendering for optimizer process trees.

use jesterky_contract::ProcessNode;

/// Render a process tree into deterministic, indented terminal text.
pub fn render_tree(root: &ProcessNode) -> String {
    let mut out = String::new();
    render_node(root, 0, &mut out);
    out
}

fn render_node(node: &ProcessNode, depth: usize, out: &mut String) {
    out.push_str(&"  ".repeat(depth));
    out.push_str(&node.label);
    if let Some(score) = node.score {
        out.push_str(&format!(" score={score:.3}"));
    }
    out.push_str(&format!(" artifacts={}", node.artifacts.len()));
    out.push('\n');

    for child in &node.children {
        render_node(child, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::render_tree;
    use jesterky_contract::{Addr, ArtifactRef, NodePath, ProcessNode};
    use serde_json::json;

    #[test]
    fn render_tree_is_deterministic_indented_text() {
        let root = ProcessNode {
            addr: addr(0),
            label: "workflow:quality".to_string(),
            inputs: json!({}),
            outputs: json!({}),
            score: Some(0.875),
            signal: None,
            artifacts: vec![artifact("blob/root")],
            children: vec![
                ProcessNode {
                    addr: addr(1),
                    label: "expand".to_string(),
                    inputs: json!({}),
                    outputs: json!({}),
                    score: None,
                    signal: None,
                    artifacts: Vec::new(),
                    children: Vec::new(),
                },
                ProcessNode {
                    addr: addr(2),
                    label: "actor:scanner".to_string(),
                    inputs: json!({}),
                    outputs: json!({ "ok": true }),
                    score: Some(0.5),
                    signal: None,
                    artifacts: vec![artifact("blob/scan")],
                    children: Vec::new(),
                },
            ],
        };

        const EXPECTED: &str = "\
workflow:quality score=0.875 artifacts=1
  expand artifacts=0
  actor:scanner score=0.500 artifacts=1
";

        assert_eq!(render_tree(&root), EXPECTED);
    }

    fn addr(local_seq: u32) -> Addr {
        Addr {
            run_id: "render-run".to_string(),
            node_path: NodePath::root(),
            iteration: 0,
            local_seq,
        }
    }

    fn artifact(key: &str) -> ArtifactRef {
        ArtifactRef {
            key: key.to_string(),
            size_bytes: 10,
            content_type: "application/json".to_string(),
        }
    }
}
