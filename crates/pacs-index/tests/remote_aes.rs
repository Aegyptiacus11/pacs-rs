mod common;

use pacs_index::StoredRemoteAe;

fn remote(id: &str, port: u16, may_receive: bool) -> StoredRemoteAe {
    StoredRemoteAe {
        id: id.into(),
        ae_title: format!("{}_AE", id.to_uppercase()),
        host: "10.0.0.5".into(),
        port,
        secure: false,
        may_receive,
    }
}

#[tokio::test]
async fn remote_aes_are_kept_replaced_by_id_and_removed() {
    for index in common::indexes().await {
        assert!(index.remote_aes().await.unwrap().is_empty());
        index
            .put_remote_ae(&remote("ws", 104, false))
            .await
            .unwrap();
        index
            .put_remote_ae(&remote("archive", 11112, true))
            .await
            .unwrap();
        let mut changed = remote("ws", 4242, true);
        changed.secure = true;
        index.put_remote_ae(&changed).await.unwrap();
        assert_eq!(
            index.remote_aes().await.unwrap(),
            vec![remote("archive", 11112, true), changed]
        );
        assert!(index.delete_remote_ae("ws").await.unwrap());
        assert!(!index.delete_remote_ae("ws").await.unwrap());
        assert_eq!(
            index.remote_aes().await.unwrap(),
            vec![remote("archive", 11112, true)]
        );
    }
}
