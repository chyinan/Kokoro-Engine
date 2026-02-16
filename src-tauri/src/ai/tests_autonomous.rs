use super::curiosity::CuriosityModule;

#[test]
fn curiosity_queue_ordering() {
    let mut cm = CuriosityModule::new();
    cm.add_topic("high relevance", 0.9, "memory");
    cm.add_topic("low relevance", 0.1, "memory");

    // Pick highest relevance
    let picked = cm.pick_topic().unwrap();
    assert_eq!(picked.topic, "high relevance");

    // Pick next
    let next = cm.pick_topic().unwrap();
    assert_eq!(next.topic, "low relevance");

    // Should be empty
    assert!(cm.pick_topic().is_none());
}

#[test]
fn curiosity_decay() {
    let mut cm = CuriosityModule::new();
    cm.add_topic("topic", 1.0, "conv");

    // Decay once
    cm.decay();

    if let Some(item) = cm.pick_topic() {
        assert!(item.relevance < 1.0);
    }
}
