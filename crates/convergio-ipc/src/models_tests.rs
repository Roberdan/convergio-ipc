use super::*;

fn pool() -> ConnPool {
    let p = convergio_db::pool::create_memory_pool().unwrap();
    let conn = p.get().unwrap();
    convergio_db::migration::ensure_registry(&conn).unwrap();
    convergio_db::migration::apply_migrations(&conn, "ipc", &crate::schema::migrations()).unwrap();
    p
}

#[test]
fn store_and_list_models() {
    let p = pool();
    let models = vec![
        ("qwen2.5:7b".to_string(), 4.5, "Q4_K_M".to_string()),
        ("llama3:8b".to_string(), 5.0, "Q5_K_S".to_string()),
    ];
    store_models(&p, "m5max", "ollama", &models).unwrap();
    let all = get_all_models(&p).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn subscription_crud() {
    let p = pool();
    let sub = Subscription {
        name: "anthropic-pro".into(),
        provider: "anthropic".into(),
        plan: "pro".into(),
        budget_usd: 100.0,
        reset_day: 15,
        models: vec!["claude-opus".into()],
    };
    add_subscription(&p, &sub).unwrap();
    let subs = list_subscriptions(&p).unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].models, vec!["claude-opus"]);
    remove_subscription(&p, "anthropic-pro").unwrap();
    assert_eq!(list_subscriptions(&p).unwrap().len(), 0);
}

#[test]
fn capabilities_advertised() {
    let p = pool();
    let models = vec![("m1".to_string(), 1.0, "Q4".to_string())];
    store_models(&p, "m5max", "ollama", &models).unwrap();
    advertise_capabilities(&p, "m5max").unwrap();
    let caps = get_all_capabilities(&p).unwrap();
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].host, "m5max");
}
