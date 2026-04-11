use super::*;

fn setup() -> (ConnPool, Arc<Notify>) {
    let p = convergio_db::pool::create_memory_pool().unwrap();
    let conn = p.get().unwrap();
    convergio_db::migration::ensure_registry(&conn).unwrap();
    convergio_db::migration::apply_migrations(&conn, "ipc", &crate::schema::migrations()).unwrap();
    (p, Arc::new(Notify::new()))
}

#[test]
fn send_and_receive() {
    let (p, n) = setup();
    let id = send(
        &p,
        &n,
        &SendParams {
            from: "elena",
            to: "baccio",
            content: "ciao",
            msg_type: "text",
            priority: 0,
            rate_limit: 100,
        },
    )
    .unwrap();
    assert!(!id.is_empty());
    let msgs = receive(&p, "baccio", None, None, 10, false).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "ciao");
}

#[test]
fn broadcast_visible_to_all() {
    let (p, n) = setup();
    broadcast(&p, &n, "elena", "hello all", "text", None, 100).unwrap();
    let msgs = receive(&p, "anyone", None, None, 10, false).unwrap();
    assert_eq!(msgs.len(), 1);
}

#[test]
fn rate_limit_enforced() {
    let (p, n) = setup();
    for i in 0..5 {
        send(
            &p,
            &n,
            &SendParams {
                from: "spammer",
                to: "target",
                content: &format!("msg{i}"),
                msg_type: "text",
                priority: 0,
                rate_limit: 5,
            },
        )
        .unwrap();
    }
    let err = send(
        &p,
        &n,
        &SendParams {
            from: "spammer",
            to: "target",
            content: "one more",
            msg_type: "text",
            priority: 0,
            rate_limit: 5,
        },
    );
    assert!(matches!(err, Err(IpcError::RateLimited(_))));
}

#[test]
fn history_returns_messages() {
    let (p, n) = setup();
    send(
        &p,
        &n,
        &SendParams {
            from: "a",
            to: "b",
            content: "first",
            msg_type: "text",
            priority: 0,
            rate_limit: 100,
        },
    )
    .unwrap();
    send(
        &p,
        &n,
        &SendParams {
            from: "b",
            to: "a",
            content: "reply",
            msg_type: "text",
            priority: 0,
            rate_limit: 100,
        },
    )
    .unwrap();
    let msgs = history(&p, Some("a"), None, 10, None).unwrap();
    assert_eq!(msgs.len(), 2);
}
