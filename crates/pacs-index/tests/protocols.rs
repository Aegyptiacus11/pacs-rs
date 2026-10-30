mod common;

#[tokio::test]
async fn protocols_are_kept_replaced_and_removed() {
    for index in common::indexes().await {
        assert!(index.protocols().await.unwrap().is_empty());
        index.put_protocol("a", r#"{"name":"A"}"#).await.unwrap();
        index.put_protocol("b", r#"{"name":"B"}"#).await.unwrap();
        // saved again: replaced, and now the newest
        std::thread::sleep(std::time::Duration::from_millis(2));
        index.put_protocol("a", r#"{"name":"A2"}"#).await.unwrap();
        let list = index.protocols().await.unwrap();
        assert_eq!(
            list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(list[0].body, r#"{"name":"A2"}"#);

        assert!(index.delete_protocol("b").await.unwrap());
        assert!(!index.delete_protocol("b").await.unwrap());
        assert_eq!(index.protocols().await.unwrap().len(), 1);
    }
}
