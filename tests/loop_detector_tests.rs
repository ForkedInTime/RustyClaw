#[test]
fn no_stagnation_on_unique_actions() {
    use rustyclaw::browser::loop_detector::LoopDetector;
    let mut ld = LoopDetector::new();
    ld.record_action("click", "@e1", "page content A");
    ld.record_action("fill", "@e2", "page content B");
    ld.record_action("click", "@e3", "page content C");
    assert!(ld.check_stagnation().is_none());
}

#[test]
fn detect_repeated_action_same_page() {
    use rustyclaw::browser::loop_detector::LoopDetector;
    let mut ld = LoopDetector::new();
    for _ in 0..4 {
        ld.record_action("click", "@e1", "same page content");
    }
    let nudge = ld.check_stagnation();
    assert!(nudge.is_some());
    assert!(nudge.unwrap().contains("repeating"));
}

#[test]
fn escalating_nudge_levels() {
    use rustyclaw::browser::loop_detector::LoopDetector;
    let mut ld = LoopDetector::new();

    // First detection — level 1
    for _ in 0..4 { ld.record_action("click", "@e1", "same"); }
    let n1 = ld.check_stagnation().unwrap();

    // Acknowledge and repeat — level 2
    ld.record_action("click", "@e2", "same");
    for _ in 0..3 { ld.record_action("click", "@e1", "same"); }
    let n2 = ld.check_stagnation().unwrap();
    assert_ne!(n1, n2); // Different nudge text
    // Level 2 nudge enumerates failure modes — pin to one of them so a
    // future refactor that silently reorders nudges is caught.
    assert!(
        n2.contains("disabled") || n2.contains("overlaid") || n2.contains("scroll"),
        "expected level-2 nudge to mention disabled/overlaid/scroll, got: {n2}"
    );

    // Third time — level 3 (stop)
    ld.record_action("click", "@e3", "same");
    for _ in 0..3 { ld.record_action("click", "@e1", "same"); }
    let n3 = ld.check_stagnation().unwrap();
    assert!(n3.contains("Stopping") || n3.contains("stop"));
}

#[test]
fn no_stagnation_when_page_state_changes() {
    // The same action repeated on DIFFERENT pages must not fire — the agent
    // is making progress, even if the action name is identical. This is what
    // the page_hash half of the fingerprint buys us.
    use rustyclaw::browser::loop_detector::LoopDetector;
    let mut ld = LoopDetector::new();
    for i in 0..4 {
        ld.record_action("click", "@e1", &format!("page snapshot {i}"));
    }
    assert!(
        ld.check_stagnation().is_none(),
        "same action on evolving page state must not count as stagnation"
    );
}

#[test]
fn fingerprint_is_deterministic() {
    use rustyclaw::browser::loop_detector::fingerprint_action;
    let h1 = fingerprint_action("click", "@e1", "hello");
    let h2 = fingerprint_action("click", "@e1", "hello");
    assert_eq!(h1, h2);
    let h3 = fingerprint_action("click", "@e2", "hello");
    assert_ne!(h1, h3);
}
