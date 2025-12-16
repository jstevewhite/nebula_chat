// Simple test to verify CSP configuration is loaded correctly
#[test]
fn test_csp_configuration_loaded() {
    // This test verifies that the CSP configuration can be parsed
    // The actual CSP enforcement is tested during runtime
    
    // Read the tauri.conf.json file
    let config_content = std::fs::read_to_string("tauri.conf.json").unwrap();
    
    // Verify that CSP is no longer null
    assert!(!config_content.contains("\"csp\": null"));
    
    // Verify that our CSP configuration is present
    assert!(config_content.contains("default-src 'self'"));
    assert!(config_content.contains("connect-src"));
    assert!(config_content.contains("api.openai.com"));
    assert!(config_content.contains("api.anthropic.com"));
    assert!(config_content.contains("http://localhost:"));
    
    // Verify security headers are restrictive
    assert!(config_content.contains("frame-src 'none'"));
    assert!(config_content.contains("object-src 'none'"));
}