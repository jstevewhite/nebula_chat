// Verifies the CSP is configured (not null) and that the policy includes the
// critical origins and lockdowns the rest of the app relies on. Parses the JSON
// rather than substring-matching the file so reformatting or key reordering
// doesn't silently break the check.
use serde_json::Value;

#[test]
fn csp_policy_is_configured_and_restrictive() {
    let raw = std::fs::read_to_string("tauri.conf.json").expect("read tauri.conf.json");
    let cfg: Value = serde_json::from_str(&raw).expect("valid JSON");

    let csp = cfg["app"]["security"]["csp"]
        .as_str()
        .expect("csp must be a non-null string");

    // Required source allowlists.
    assert!(csp.contains("default-src 'self'"), "CSP missing default-src 'self'");
    assert!(csp.contains("connect-src"), "CSP missing connect-src directive");
    assert!(csp.contains("api.openai.com"), "CSP missing OpenAI origin");
    assert!(csp.contains("api.anthropic.com"), "CSP missing Anthropic origin");
    assert!(
        csp.contains("http://localhost:"),
        "CSP missing localhost (dev server) origin"
    );

    // Required lockdowns.
    assert!(csp.contains("frame-src 'none'"), "CSP must forbid framing");
    assert!(csp.contains("object-src 'none'"), "CSP must forbid plugin objects");
}
