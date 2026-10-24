use ferrum_kv::storage::Db;

#[test]
fn set_get_del_and_size() {
    let db = Db::new();
    db.set("key", "one");
    db.set("key", "two");
    assert_eq!(db.len(), 1);
    assert_eq!(db.get("key"), Some("two".to_string()));
    assert!(db.del("key"));
    assert!(!db.del("key"));
    assert!(db.is_empty());
}

#[test]
fn incr_preserves_invalid_and_overflow_values() {
    let db = Db::new();
    assert_eq!(db.incr("key"), Ok(1));
    db.set("key", "texto");
    assert!(db.incr("key").is_err());
    assert_eq!(db.get("key").as_deref(), Some("texto"));
    db.set("key", &i64::MAX.to_string());
    assert!(db.incr("key").is_err());
    assert_eq!(db.get("key"), Some(i64::MAX.to_string()));
}

#[test]
fn concurrent_increments_are_atomic() {
    let db = Db::new();
    let handles: Vec<_> = (0..24)
        .map(|_| {
            let db = db.clone();
            std::thread::spawn(move || {
                for _ in 0..40 {
                    db.incr("counter").unwrap();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(db.get("counter").as_deref(), Some("960"));
}
