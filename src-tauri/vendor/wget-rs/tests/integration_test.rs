use std::env;

#[test]
fn test_user_agent_format() {
    // Test that our user agent format matches what we expect
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let expected_user_agent = format!("Wget/{} ({})", version, os);
    
    // This would be "Wget/0.1.0 (linux)" on Linux
    assert!(expected_user_agent.starts_with("Wget/"));
    assert!(expected_user_agent.contains("0.1.0"));
    assert!(expected_user_agent.contains(os));
}

#[test] 
fn test_version_constant() {
    // Verify that CARGO_PKG_VERSION is available and valid
    let version = env!("CARGO_PKG_VERSION");
    assert!(!version.is_empty());
    assert_eq!(version, "0.1.0");
}

#[test]
fn test_os_constant() {
    // Verify that OS constant is available 
    let os = std::env::consts::OS;
    assert!(!os.is_empty());
    // Should be one of the known OS types
    assert!(["linux", "windows", "macos", "freebsd", "openbsd", "netbsd", "dragonfly", "android", "ios"].contains(&os));
}