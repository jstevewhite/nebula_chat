// Smoke tests for TantivyIndex public API. These verify that the basic add /
// delete / clear operations don't panic and that the batching channel accepts
// queued writes; they do NOT assert search correctness or any timing budget.
use tauri_appnebula_lib::memory::tantivy_index::TantivyIndex;

#[tokio::test]
async fn add_document_accepts_multiple_queued_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyIndex::new(tmp.path().to_str().unwrap()).unwrap();

    for i in 0..20 {
        index
            .add_document(
                "test_conversation",
                "user",
                &format!("Test message {}", i),
                &format!("msg_{}", i),
                "2024-01-01T00:00:00Z",
            )
            .unwrap();
    }
}

#[tokio::test]
async fn basic_lifecycle_operations_succeed() {
    let tmp = tempfile::tempdir().unwrap();
    let index = TantivyIndex::new(tmp.path().to_str().unwrap()).unwrap();

    index
        .add_document(
            "test_conv",
            "user",
            "Hello world",
            "msg_1",
            "2024-01-01T00:00:00Z",
        )
        .unwrap();
    index.delete_by_message_id("msg_1").unwrap();
    index.delete_by_conversation_id("test_conv").unwrap();
    index.clear_index().unwrap();
}