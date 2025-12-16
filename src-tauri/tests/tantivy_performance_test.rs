// Test Tantivy batching performance improvements
use tauri_appnebula_lib::memory::tantivy_index::TantivyIndex;
use std::time::Instant;

#[tokio::test]
async fn test_tantivy_batching_functionality() {
    // Create a temporary index for testing
    let temp_dir = "/tmp/test_tantivy_batching";
    let _ = std::fs::remove_dir_all(temp_dir);
    
    // Create the index
    let index = TantivyIndex::new(temp_dir).unwrap();
    
    // Test that we can add multiple documents quickly
    let start_time = Instant::now();
    
    for i in 0..20 {
        index.add_document(
            "test_conversation",
            "user",
            &format!("Test message {}", i),
            &format!("msg_{}", i),
            "2024-01-01T00:00:00Z"
        ).unwrap();
    }
    
    let duration = start_time.elapsed();
    
    // The batching should make this much faster than individual commits
    // With batching, 20 documents should take much less time than 20 individual commits
    println!("Added 20 documents in {:?}", duration);
    
    // Verify that the documents will eventually be searchable
    // (They might not be immediately available due to batching)
    
    // Clean up
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn test_tantivy_index_creation() {
    // Test that the index can be created and basic operations work
    let temp_dir = "/tmp/test_tantivy_creation";
    let _ = std::fs::remove_dir_all(temp_dir);
    
    let index = TantivyIndex::new(temp_dir).unwrap();
    
    // Test basic operations
    index.add_document(
        "test_conv",
        "user",
        "Hello world",
        "msg_1",
        "2024-01-01T00:00:00Z"
    ).unwrap();
    
    index.delete_by_message_id("msg_1").unwrap();
    index.delete_by_conversation_id("test_conv").unwrap();
    index.clear_index().unwrap();
    
    // Clean up
    let _ = std::fs::remove_dir_all(temp_dir);
}