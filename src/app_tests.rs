use super::*;
use crate::graph::types::GraphEdge;

fn session_edge(source: &str, target: &str) -> GraphEdge {
    GraphEdge {
        source: source.into(),
        target: target.into(),
        session_id: "s1".into(),
        timestamp: None,
        is_obsidian: false,
        is_topic: false,
        is_similarity: false,
        is_temporal: false,
        similarity: None,
    }
}

#[test]
fn adjacency_includes_all_edges_by_default() {
    let edges = vec![
        session_edge("A", "B"),
        GraphEdge::temporal("B".into(), "C".into(), 1.0),
    ];
    let adj = build_adjacency_list(&edges, true);
    assert!(adj["A"].contains(&"B".to_string()));
    assert!(adj["B"].contains(&"C".to_string()));
    assert!(adj["C"].contains(&"B".to_string()));
}

#[test]
fn adjacency_excludes_temporal_when_disabled() {
    let edges = vec![
        session_edge("A", "B"),
        GraphEdge::temporal("B".into(), "C".into(), 1.0),
    ];
    let adj = build_adjacency_list(&edges, false);
    // A-B should exist
    assert!(adj["A"].contains(&"B".to_string()));
    // B-C temporal edge should be excluded
    assert!(!adj.get("B").map_or(false, |v| v.contains(&"C".to_string())));
    assert!(!adj.contains_key("C"));
}

#[test]
fn expand_depth_0_returns_seeds() {
    let adj = build_adjacency_list(&[session_edge("A", "B")], true);
    let seeds: HashSet<String> = ["A".into()].into();
    let result = expand_to_neighbors(&seeds, 0, &adj);
    assert_eq!(result, seeds);
}

#[test]
fn expand_depth_1_returns_direct_neighbors() {
    // A - B - C
    let edges = vec![session_edge("A", "B"), session_edge("B", "C")];
    let adj = build_adjacency_list(&edges, true);

    let seeds: HashSet<String> = ["A".into()].into();
    let result = expand_to_neighbors(&seeds, 1, &adj);
    assert!(result.contains("A"));
    assert!(result.contains("B"));
    assert!(!result.contains("C"));
}

#[test]
fn expand_depth_2_reaches_two_hops() {
    // A - B - C - D
    let edges = vec![
        session_edge("A", "B"),
        session_edge("B", "C"),
        session_edge("C", "D"),
    ];
    let adj = build_adjacency_list(&edges, true);

    let seeds: HashSet<String> = ["A".into()].into();
    let result = expand_to_neighbors(&seeds, 2, &adj);
    assert!(result.contains("A"));
    assert!(result.contains("B"));
    assert!(result.contains("C"));
    assert!(!result.contains("D"));
}

#[test]
fn expand_depth_with_temporal_toggle() {
    // A -session- B -temporal- C -session- D
    let edges = vec![
        session_edge("A", "B"),
        GraphEdge::temporal("B".into(), "C".into(), 1.0),
        session_edge("C", "D"),
    ];

    // With temporal: A depth-2 should reach C
    let adj_all = build_adjacency_list(&edges, true);
    let seeds: HashSet<String> = ["A".into()].into();
    let result = expand_to_neighbors(&seeds, 2, &adj_all);
    assert!(result.contains("C"));

    // Without temporal: A depth-2 still only reaches B (no path to C)
    let adj_no_temp = build_adjacency_list(&edges, false);
    let result = expand_to_neighbors(&seeds, 2, &adj_no_temp);
    assert!(result.contains("B"));
    assert!(!result.contains("C"));
}
