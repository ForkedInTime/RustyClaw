//! Integration tests for auto-rollback core logic — TDD, written before implementation.

use rustyclaw::rollback::{
    detect_test_command, should_trigger, RollbackConfig, RollbackTrigger,
};
use std::fs;
use tempfile::TempDir;

// ── detect_test_command ──────────────────────────────────────────────────────

#[test]
fn test_detect_rust_project() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let got = detect_test_command(tmp.path(), &None);
    assert_eq!(got, Some("cargo test".to_string()));
}

#[test]
fn test_detect_node_project() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("package.json"), "{}").unwrap();
    let got = detect_test_command(tmp.path(), &None);
    assert_eq!(got, Some("npm test".to_string()));
}

#[test]
fn test_detect_python_project_pyproject() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
    let got = detect_test_command(tmp.path(), &None);
    assert_eq!(got, Some("pytest".to_string()));
}

#[test]
fn test_detect_python_project_setup() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("setup.py"), "from setuptools import setup\n").unwrap();
    let got = detect_test_command(tmp.path(), &None);
    assert_eq!(got, Some("pytest".to_string()));
}

#[test]
fn test_detect_go_project() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("go.mod"), "module x\n").unwrap();
    let got = detect_test_command(tmp.path(), &None);
    assert_eq!(got, Some("go test ./...".to_string()));
}

#[test]
fn test_detect_no_runner() {
    let tmp = TempDir::new().unwrap();
    let got = detect_test_command(tmp.path(), &None);
    assert_eq!(got, None);
}

#[test]
fn test_custom_override() {
    let tmp = TempDir::new().unwrap();
    let override_cmd = Some("make test".to_string());
    let got = detect_test_command(tmp.path(), &override_cmd);
    assert_eq!(got, Some("make test".to_string()));
}

// ── should_trigger ────────────────────────────────────────────────────────────

#[test]
fn test_should_trigger_off() {
    let cfg = RollbackConfig {
        enabled: true,
        trigger: RollbackTrigger::Off,
        test_command: None,
        max_retries: 3,
    };
    assert!(!should_trigger(&cfg, "auto-edit"));
    assert!(!should_trigger(&cfg, "full-auto"));
    assert!(!should_trigger(&cfg, "read-only"));
}

#[test]
fn test_should_trigger_always() {
    let cfg = RollbackConfig {
        enabled: true,
        trigger: RollbackTrigger::Always,
        test_command: None,
        max_retries: 3,
    };
    assert!(should_trigger(&cfg, "read-only"));
    assert!(should_trigger(&cfg, "plan-only"));
    assert!(should_trigger(&cfg, "auto-edit"));
    assert!(should_trigger(&cfg, "full-auto"));
}

#[test]
fn test_should_trigger_autonomous_auto_edit() {
    let cfg = RollbackConfig {
        enabled: true,
        trigger: RollbackTrigger::Autonomous,
        test_command: None,
        max_retries: 3,
    };
    assert!(should_trigger(&cfg, "auto-edit"));
    assert!(should_trigger(&cfg, "full-auto"));
}

#[test]
fn test_should_trigger_autonomous_read_only() {
    let cfg = RollbackConfig {
        enabled: true,
        trigger: RollbackTrigger::Autonomous,
        test_command: None,
        max_retries: 3,
    };
    assert!(!should_trigger(&cfg, "read-only"));
    assert!(!should_trigger(&cfg, "plan-only"));
}

#[test]
fn test_should_trigger_disabled() {
    let cfg = RollbackConfig {
        enabled: false,
        trigger: RollbackTrigger::Always,
        test_command: None,
        max_retries: 3,
    };
    assert!(!should_trigger(&cfg, "auto-edit"));
    assert!(!should_trigger(&cfg, "full-auto"));
}

// ── default config ────────────────────────────────────────────────────────────

#[test]
fn test_default_config() {
    let cfg = RollbackConfig::default();
    assert!(cfg.enabled);
    assert_eq!(cfg.trigger, RollbackTrigger::Autonomous);
    assert_eq!(cfg.max_retries, 3);
    assert!(cfg.test_command.is_none());
}
