use super::*;
use std::cell::Cell;
use std::collections::BTreeMap;

#[derive(Default)]
struct MemoryEnvironment {
    values: BTreeMap<String, EnvironmentValue>,
    fail_next: Option<String>,
    notified: Cell<usize>,
}
impl Environment for MemoryEnvironment {
    fn read(&mut self, name: &str) -> Result<Option<EnvironmentValue>, String> {
        Ok(self.values.get(name).cloned())
    }
    fn write(&mut self, name: &str, value: Option<&EnvironmentValue>) -> Result<(), String> {
        if self.fail_next.as_deref() == Some(name) {
            self.fail_next = None;
            return Err("synthetic write failure".into());
        }
        match value {
            Some(value) => {
                self.values.insert(name.into(), value.clone());
            }
            None => {
                self.values.remove(name);
            }
        }
        Ok(())
    }
    fn notify(&self) -> bool {
        self.notified.set(self.notified.get() + 1);
        true
    }
}
fn path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("terminal-proxy.dat")
}

#[test]
fn activation_and_updates_restore_exact_original_values_and_types() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    let mut original = EnvironmentValue::string("http://%OLD_PROXY%:8080");
    original.kind = 2;
    env.values.insert("HTTPS_PROXY".into(), original);
    env.values
        .insert("NO_PROXY".into(), EnvironmentValue::string(".company.test"));
    let before = env.values.clone();
    let status = set_with_environment(
        &path(&dir),
        &mut env,
        true,
        "http://127.0.0.1:7890",
        ".company.test",
    )
    .unwrap();
    assert!(status.applied);
    assert_eq!(
        env.values["NO_PROXY"].text().unwrap(),
        ".company.test,localhost,127.0.0.1,::1"
    );
    set_with_environment(
        &path(&dir),
        &mut env,
        true,
        "http://127.0.0.1:8888",
        "internal.test",
    )
    .unwrap();
    assert_eq!(
        env.values["HTTP_PROXY"].text().unwrap(),
        "http://127.0.0.1:8888/"
    );
    // Restoration remains available even if a user has typed an invalid URL.
    let restored = set_with_environment(&path(&dir), &mut env, false, "broken", "\0").unwrap();
    assert!(!restored.enabled);
    assert_eq!(env.values, before);
}

#[test]
fn disable_preserves_external_changes_and_does_not_leave_other_owned_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    set_with_environment(&path(&dir), &mut env, true, "https://proxy.test:443", "").unwrap();
    let external = EnvironmentValue::string("http://another.test:8080");
    env.values.insert("HTTPS_PROXY".into(), external.clone());
    assert_eq!(
        set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "")
            .err()
            .unwrap(),
        "settings.terminalProxy.errors.changed"
    );
    let result = set_with_environment(&path(&dir), &mut env, false, "", "").unwrap();
    assert_eq!(result.preserved_variables, vec!["HTTPS_PROXY"]);
    assert_eq!(env.values.len(), 1);
    assert_eq!(env.values["HTTPS_PROXY"], external);
}

#[test]
fn failed_activation_rolls_back_without_losing_the_preexisting_environment() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    env.values.insert(
        "HTTP_PROXY".into(),
        EnvironmentValue::string("http://original.test"),
    );
    let before = env.values.clone();
    env.fail_next = Some("ALL_PROXY".into());
    assert!(
        set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "").is_err()
    );
    assert_eq!(env.values, before);
    assert!(load(&path(&dir)).unwrap().lease.is_none());
}

#[test]
fn failed_update_restores_the_previous_proxy_and_keeps_the_original_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    env.values.insert(
        "HTTPS_PROXY".into(),
        EnvironmentValue::string("http://original.test:8080"),
    );
    let original = env.values.clone();
    set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "").unwrap();
    let active = env.values.clone();
    env.fail_next = Some("ALL_PROXY".into());
    assert!(set_with_environment(
        &path(&dir),
        &mut env,
        true,
        "http://127.0.0.1:8888",
        "new.test"
    )
    .is_err());
    assert_eq!(env.values, active);
    let saved = load(&path(&dir)).unwrap();
    assert!(status(&saved, &mut env).unwrap().applied);
    assert_eq!(saved.url.as_deref(), Some("http://127.0.0.1:7890/"));
    set_with_environment(&path(&dir), &mut env, false, "", "").unwrap();
    assert_eq!(env.values, original);
}

#[test]
fn interrupted_apply_and_interrupted_restore_have_a_recoverable_ownership_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "").unwrap();
    let mut pending = load(&path(&dir)).unwrap();
    pending.lease.as_mut().unwrap().phase = Phase::Applying;
    persist(&path(&dir), &pending).unwrap();
    env.values.remove("ALL_PROXY"); // A crash after only some fields were installed.
    assert!(status(&pending, &mut env).unwrap().recovery_required);
    assert_eq!(
        set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:8888", "")
            .err()
            .unwrap(),
        "settings.terminalProxy.errors.recovery"
    );
    env.fail_next = Some("HTTPS_PROXY".into());
    assert!(set_with_environment(&path(&dir), &mut env, false, "", "").is_err());
    assert_eq!(
        env.notified.get(),
        2,
        "Partial restoration must be broadcast"
    );
    assert!(matches!(
        load(&path(&dir)).unwrap().lease.unwrap().phase,
        Phase::Restoring
    ));
    set_with_environment(&path(&dir), &mut env, false, "", "").unwrap();
    assert!(env.values.is_empty());
    assert_eq!(env.notified.get(), 3);
}

#[test]
fn large_raw_original_values_keep_the_journal_readable_and_restore_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    for name in NAMES {
        env.values.insert(
            name.into(),
            EnvironmentValue {
                kind: 3, // REG_BINARY: raw data is preserved even when not text.
                bytes: vec![255; 65_536],
            },
        );
    }
    let before = env.values.clone();
    set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "").unwrap();
    assert!(std::fs::metadata(path(&dir)).unwrap().len() > 1_048_576);
    assert!(load(&path(&dir)).unwrap().lease.is_some());
    set_with_environment(&path(&dir), &mut env, false, "", "").unwrap();
    assert_eq!(env.values, before);
}

#[test]
fn readonly_status_never_creates_storage_or_changes_environment() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    env.values
        .insert("NO_PROXY".into(), EnvironmentValue::string("internal.test"));
    let before = env.values.clone();
    let loaded = load(&path(&dir)).unwrap();
    let status = status(&loaded, &mut env).unwrap();
    assert!(!status.enabled);
    assert!(status.no_proxy.contains("internal.test"));
    assert!(!path(&dir).exists());
    assert_eq!(env.values, before);
}

#[test]
fn corrupt_ownership_cannot_write_unrelated_environment_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "").unwrap();
    let mut settings = load(&path(&dir)).unwrap();
    settings.lease.as_mut().unwrap().entries[0].name = "PATH".into();
    persist(&path(&dir), &settings).unwrap();
    let before = env.values.clone();
    assert!(set_with_environment(&path(&dir), &mut env, false, "", "").is_err());
    assert_eq!(env.values, before);
}

#[test]
fn proxy_validation_rejects_socks_credentials_paths_and_control_characters() {
    assert_eq!(
        normalize_url("socks5://127.0.0.1:7890").err().unwrap(),
        "settings.terminalProxy.errors.scheme"
    );
    for value in [
        "http://user:password@proxy.test",
        "http://proxy.test/path",
        "http://proxy.test/?secret=x",
        "http://proxy.test\nmalicious",
        "bad",
    ] {
        assert!(normalize_url(value).is_err(), "{value}");
    }
    assert_eq!(
        normalize_url(" http://127.0.0.1:7890 ").unwrap(),
        "http://127.0.0.1:7890/"
    );
}

fn domain_bypass(count: usize) -> String {
    (0..count)
        .map(|index| format!("h{index:04}.example.test"))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn bypass_limit_includes_loopback_addresses_before_any_mutation() {
    let bypass = domain_bypass(215);
    assert_eq!(bypass.len(), 4_084);
    assert_eq!(format!("{bypass},{}", LOOPBACK.join(",")).len(), 4_108);
    for already_enabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut env = MemoryEnvironment::default();
        env.values.insert(
            "HTTP_PROXY".into(),
            EnvironmentValue::string("http://original.test"),
        );
        if already_enabled {
            set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "").unwrap();
        }
        let before = env.values.clone();
        let journal_before = std::fs::read(path(&dir)).ok();
        let notifications_before = env.notified.get();
        assert_eq!(
            set_with_environment(
                &path(&dir),
                &mut env,
                true,
                "http://127.0.0.1:8888",
                &bypass,
            )
            .err()
            .unwrap(),
            "settings.terminalProxy.errors.bypass"
        );
        assert_eq!(env.values, before);
        assert_eq!(std::fs::read(path(&dir)).ok(), journal_before);
        assert_eq!(env.notified.get(), notifications_before);
    }
}

#[test]
fn maximum_normalized_bypass_remains_readable_on_activation_and_update() {
    let bypass = format!("prefix7{}", domain_bypass(214));
    let normalized = normalize_bypass(&bypass).unwrap();
    assert_eq!(normalized.len(), MAX_BYPASS_BYTES);
    assert_eq!(normalize_bypass(&normalized).unwrap(), normalized);
    let dir = tempfile::tempdir().unwrap();
    let mut env = MemoryEnvironment::default();
    env.values.insert(
        "NO_PROXY".into(),
        EnvironmentValue::string("original.internal"),
    );
    let original = env.values.clone();
    for (url, input) in [
        ("http://127.0.0.1:7890", bypass.as_str()),
        ("http://127.0.0.1:8888", normalized.as_str()),
    ] {
        let result = set_with_environment(&path(&dir), &mut env, true, url, input).unwrap();
        assert!(result.applied);
        assert_eq!(result.no_proxy, normalized);
        let saved = load(&path(&dir)).unwrap();
        assert_eq!(saved.no_proxy, normalized);
        assert!(status(&saved, &mut env).unwrap().applied);
    }
    set_with_environment(&path(&dir), &mut env, false, "", "").unwrap();
    assert_eq!(env.values, original);
}

#[test]
fn legacy_oversized_bypass_journals_restore_all_phases_and_preserve_external_edits() {
    for prefix in ["", "abcdefghijkl"] {
        let legacy = format!("{prefix}{},{}", domain_bypass(215), LOOPBACK.join(","));
        assert!(legacy.len() > MAX_BYPASS_BYTES);
        assert!(legacy.len() <= MAX_STORED_BYPASS_BYTES);
        for phase in [Phase::Active, Phase::Applying, Phase::Restoring] {
            let dir = tempfile::tempdir().unwrap();
            let mut env = MemoryEnvironment::default();
            let mut original_proxy = EnvironmentValue::string("http://%OLD_PROXY%:8080");
            original_proxy.kind = 2;
            env.values.insert("HTTP_PROXY".into(), original_proxy);
            env.values.insert(
                "NO_PROXY".into(),
                EnvironmentValue::string("original.internal"),
            );
            let mut expected = env.values.clone();
            set_with_environment(&path(&dir), &mut env, true, "http://127.0.0.1:7890", "").unwrap();
            let mut saved = load(&path(&dir)).unwrap();
            saved.no_proxy = legacy.clone();
            let lease = saved.lease.as_mut().unwrap();
            lease.phase = phase.clone();
            let entry = lease
                .entries
                .iter_mut()
                .find(|entry| entry.name == "NO_PROXY")
                .unwrap();
            entry.owned = vec![EnvironmentValue::string(&legacy)];
            env.values.insert("NO_PROXY".into(), entry.owned[0].clone());
            if phase == Phase::Applying {
                env.values.remove("ALL_PROXY");
            } else if phase == Phase::Restoring {
                env.values
                    .insert("HTTP_PROXY".into(), expected["HTTP_PROXY"].clone());
            }
            persist(&path(&dir), &saved).unwrap();
            let saved = load(&path(&dir)).unwrap();
            let result = status(&saved, &mut env).unwrap();
            assert!(result.enabled);
            assert_eq!(result.applied, phase == Phase::Active);
            assert_eq!(result.recovery_required, phase != Phase::Active);
            assert_eq!(result.no_proxy, legacy);
            let external = EnvironmentValue::string("http://external.test:8888");
            env.values.insert("HTTPS_PROXY".into(), external.clone());
            expected.insert("HTTPS_PROXY".into(), external);
            let restored =
                set_with_environment(&path(&dir), &mut env, false, "broken", "\0").unwrap();
            assert!(!restored.enabled);
            assert_eq!(restored.preserved_variables, vec!["HTTPS_PROXY"]);
            assert_eq!(env.values, expected);
            assert!(load(&path(&dir)).unwrap().lease.is_none());
        }
    }
}
