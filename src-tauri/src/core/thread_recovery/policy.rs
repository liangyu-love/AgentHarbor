use super::*;

pub(super) fn map_catalog_policy_fields(
    payload: &serde_json::Value,
) -> Result<CatalogPolicyFields, String> {
    let object = payload
        .as_object()
        .ok_or_else(|| "thread settings are not a JSON object".to_owned())?;
    let approval_mode = object
        .get("approval_policy")
        .map(canonical_approval_mode)
        .transpose()?;
    let sandbox_policy = if let Some(permission_profile) = object.get("permission_profile") {
        Some(canonical_permission_profile(permission_profile)?)
    } else if let Some(legacy_sandbox) = object.get("sandbox_policy") {
        Some(canonical_legacy_permission_profile(
            legacy_sandbox,
            object.get("file_system_sandbox_policy"),
            object.get("cwd"),
        )?)
    } else if object.contains_key("file_system_sandbox_policy") {
        return Err(
            "file_system_sandbox_policy is present without sandbox_policy enforcement metadata"
                .into(),
        );
    } else {
        None
    };
    Ok(CatalogPolicyFields {
        sandbox_policy,
        approval_mode,
    })
}

pub(super) fn canonical_approval_mode(value: &serde_json::Value) -> Result<String, String> {
    if let Some(mode) = value.as_str() {
        return match mode {
            "untrusted" | "on-request" | "never" => Ok(mode.to_owned()),
            "on-failure" => Ok("on-request".into()),
            other => Err(format!("unsupported approval_policy value: {other}")),
        };
    }
    let object = strict_object(value, "approval_policy", &["granular"])?;
    let granular = object
        .get("granular")
        .ok_or_else(|| "approval_policy object must contain granular".to_owned())?;
    let config = strict_object(
        granular,
        "approval_policy.granular",
        &[
            "sandbox_approval",
            "rules",
            "skill_approval",
            "request_permissions",
            "mcp_elicitations",
        ],
    )?;
    let sandbox_approval = required_bool(config, "sandbox_approval", "approval_policy.granular")?;
    let rules = required_bool(config, "rules", "approval_policy.granular")?;
    let skill_approval =
        optional_bool(config, "skill_approval", false, "approval_policy.granular")?;
    let request_permissions = optional_bool(
        config,
        "request_permissions",
        false,
        "approval_policy.granular",
    )?;
    let mcp_elicitations = required_bool(config, "mcp_elicitations", "approval_policy.granular")?;
    Ok(serde_json::json!({
        "granular": {
            "sandbox_approval": sandbox_approval,
            "rules": rules,
            "skill_approval": skill_approval,
            "request_permissions": request_permissions,
            "mcp_elicitations": mcp_elicitations,
        }
    })
    .to_string())
}

pub(super) fn canonical_permission_profile(value: &serde_json::Value) -> Result<String, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "permission_profile is not a JSON object".to_owned())?;
    if let Some(kind) = object.get("type") {
        let kind = kind
            .as_str()
            .ok_or_else(|| "permission_profile.type is not a string".to_owned())?;
        let canonical = match kind {
            "disabled" => {
                ensure_allowed_keys(object, "permission_profile", &["type"])?;
                CatalogPermissionProfile::Disabled
            }
            "external" => {
                ensure_allowed_keys(object, "permission_profile", &["type", "network"])?;
                let network = canonical_network(
                    object
                        .get("network")
                        .ok_or_else(|| "permission_profile.external lacks network".to_owned())?,
                    "permission_profile.network",
                )?;
                CatalogPermissionProfile::External { network }
            }
            "managed" => {
                ensure_allowed_keys(
                    object,
                    "permission_profile",
                    &["type", "file_system", "network"],
                )?;
                let file_system =
                    canonical_managed_file_system(object.get("file_system").ok_or_else(|| {
                        "permission_profile.managed lacks file_system".to_owned()
                    })?)?;
                let network = canonical_network(
                    object
                        .get("network")
                        .ok_or_else(|| "permission_profile.managed lacks network".to_owned())?,
                    "permission_profile.network",
                )?;
                CatalogPermissionProfile::Managed {
                    file_system,
                    network,
                }
            }
            other => return Err(format!("unsupported permission_profile type: {other}")),
        };
        return serde_json::to_string(&canonical)
            .map_err(|error| format!("failed to serialize permission_profile: {error}"));
    }

    ensure_allowed_keys(
        object,
        "legacy permission_profile",
        &["network", "file_system"],
    )?;
    let network = match object.get("network") {
        Some(value) => {
            let network = strict_object(value, "permission_profile.network", &["enabled"])?;
            if optional_bool(network, "enabled", false, "permission_profile.network")? {
                CatalogNetworkPolicy::Enabled
            } else {
                CatalogNetworkPolicy::Restricted
            }
        }
        None => CatalogNetworkPolicy::Restricted,
    };
    let file_system = match object.get("file_system") {
        Some(value) => canonical_legacy_file_system(value)?,
        None => CatalogManagedFileSystem::Restricted {
            entries: Vec::new(),
            glob_scan_max_depth: None,
        },
    };
    serde_json::to_string(&CatalogPermissionProfile::Managed {
        file_system,
        network,
    })
    .map_err(|error| format!("failed to serialize legacy permission_profile: {error}"))
}

pub(super) fn canonical_legacy_permission_profile(
    sandbox: &serde_json::Value,
    split_file_system: Option<&serde_json::Value>,
    cwd: Option<&serde_json::Value>,
) -> Result<String, String> {
    let object = sandbox
        .as_object()
        .ok_or_else(|| "sandbox_policy is not a JSON object".to_owned())?;
    let kind = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "sandbox_policy.type is missing or invalid".to_owned())?;
    let (enforcement, network) = match kind {
        "danger-full-access" => {
            ensure_allowed_keys(object, "sandbox_policy", &["type"])?;
            (
                LegacySandboxEnforcement::Disabled,
                CatalogNetworkPolicy::Enabled,
            )
        }
        "read-only" => {
            ensure_allowed_keys(object, "sandbox_policy", &["type", "network_access"])?;
            let network = if optional_bool(object, "network_access", false, "sandbox_policy")? {
                CatalogNetworkPolicy::Enabled
            } else {
                CatalogNetworkPolicy::Restricted
            };
            (LegacySandboxEnforcement::Managed, network)
        }
        "external-sandbox" => {
            ensure_allowed_keys(object, "sandbox_policy", &["type", "network_access"])?;
            let network = match object.get("network_access") {
                Some(value) => canonical_network(value, "sandbox_policy.network_access")?,
                None => CatalogNetworkPolicy::Restricted,
            };
            (LegacySandboxEnforcement::External, network)
        }
        "workspace-write" => {
            ensure_allowed_keys(
                object,
                "sandbox_policy",
                &[
                    "type",
                    "writable_roots",
                    "network_access",
                    "exclude_tmpdir_env_var",
                    "exclude_slash_tmp",
                ],
            )?;
            let network = if optional_bool(object, "network_access", false, "sandbox_policy")? {
                CatalogNetworkPolicy::Enabled
            } else {
                CatalogNetworkPolicy::Restricted
            };
            (LegacySandboxEnforcement::Managed, network)
        }
        other => return Err(format!("unsupported sandbox_policy type: {other}")),
    };

    let file_system = match split_file_system {
        Some(value) => canonical_split_file_system(value)?,
        None if kind == "read-only" => {
            CatalogFileSystemPolicy::Managed(CatalogManagedFileSystem::Restricted {
                entries: vec![CatalogFileSystemEntry {
                    path: CatalogFileSystemPath::Special {
                        value: CatalogSpecialPath::Root,
                    },
                    access: CatalogFileSystemAccess::Read,
                    missing_path_behavior: None,
                }],
                glob_scan_max_depth: None,
            })
        }
        None if kind == "danger-full-access" => {
            CatalogFileSystemPolicy::Managed(CatalogManagedFileSystem::Unrestricted)
        }
        None if kind == "external-sandbox" => CatalogFileSystemPolicy::ExternalSandbox,
        None if kind == "workspace-write" => {
            CatalogFileSystemPolicy::Managed(canonical_legacy_workspace_file_system(object, cwd)?)
        }
        None => unreachable!("all supported legacy sandbox variants are handled"),
    };
    let compatible = matches!(
        (&file_system, enforcement),
        (
            CatalogFileSystemPolicy::Managed(CatalogManagedFileSystem::Unrestricted),
            LegacySandboxEnforcement::Disabled
        ) | (
            CatalogFileSystemPolicy::ExternalSandbox,
            LegacySandboxEnforcement::External
        ) | (
            CatalogFileSystemPolicy::Managed(CatalogManagedFileSystem::Restricted { .. }),
            LegacySandboxEnforcement::Managed
        )
    );
    if !compatible {
        return Err("sandbox_policy enforcement conflicts with file_system_sandbox_policy".into());
    }
    let canonical = match (file_system, enforcement) {
        (CatalogFileSystemPolicy::ExternalSandbox, _) | (_, LegacySandboxEnforcement::External) => {
            CatalogPermissionProfile::External { network }
        }
        (
            CatalogFileSystemPolicy::Managed(CatalogManagedFileSystem::Unrestricted),
            LegacySandboxEnforcement::Disabled,
        ) => CatalogPermissionProfile::Disabled,
        (CatalogFileSystemPolicy::Managed(file_system), _) => CatalogPermissionProfile::Managed {
            file_system,
            network,
        },
    };
    serde_json::to_string(&canonical)
        .map_err(|error| format!("failed to serialize legacy sandbox profile: {error}"))
}

pub(super) fn canonical_legacy_workspace_file_system(
    sandbox: &serde_json::Map<String, serde_json::Value>,
    cwd: Option<&serde_json::Value>,
) -> Result<CatalogManagedFileSystem, String> {
    let cwd = non_empty_string(cwd)
        .ok_or_else(|| "legacy workspace-write sandbox lacks a valid cwd".to_owned())?;
    let cwd = PathBuf::from(cwd);
    if !cwd.is_absolute() {
        return Err("legacy workspace-write cwd must be an absolute native path".into());
    }

    let writable_roots = match sandbox.get("writable_roots") {
        Some(value) => value
            .as_array()
            .ok_or_else(|| "sandbox_policy.writable_roots is not an array".to_owned())?
            .iter()
            .map(|value| {
                let root = non_empty_string(Some(value)).ok_or_else(|| {
                    "sandbox_policy.writable_roots contains an invalid path".to_owned()
                })?;
                let root = PathBuf::from(root);
                if !root.is_absolute() {
                    return Err(
                        "sandbox_policy.writable_roots must contain absolute native paths"
                            .to_owned(),
                    );
                }
                Ok(root)
            })
            .collect::<Result<Vec<_>, String>>()?,
        None => Vec::new(),
    };
    let exclude_tmpdir = optional_bool(sandbox, "exclude_tmpdir_env_var", false, "sandbox_policy")?;
    let exclude_slash_tmp = optional_bool(sandbox, "exclude_slash_tmp", false, "sandbox_policy")?;

    let mut entries = Vec::new();
    push_file_system_entry(
        &mut entries,
        CatalogFileSystemPath::Special {
            value: CatalogSpecialPath::Root,
        },
        CatalogFileSystemAccess::Read,
    );
    push_file_system_entry(
        &mut entries,
        CatalogFileSystemPath::Special {
            value: CatalogSpecialPath::ProjectRoots { subpath: None },
        },
        CatalogFileSystemAccess::Write,
    );
    if !exclude_slash_tmp {
        push_file_system_entry(
            &mut entries,
            CatalogFileSystemPath::Special {
                value: CatalogSpecialPath::SlashTmp,
            },
            CatalogFileSystemAccess::Write,
        );
    }
    if !exclude_tmpdir {
        push_file_system_entry(
            &mut entries,
            CatalogFileSystemPath::Special {
                value: CatalogSpecialPath::Tmpdir,
            },
            CatalogFileSystemAccess::Write,
        );
    }
    for root in &writable_roots {
        push_file_system_entry(
            &mut entries,
            absolute_path_value(root),
            CatalogFileSystemAccess::Write,
        );
    }
    for subpath in [".git", ".agents", ".codex"] {
        append_default_read_only_entry(
            &mut entries,
            CatalogFileSystemPath::Special {
                value: CatalogSpecialPath::ProjectRoots {
                    subpath: Some(subpath.into()),
                },
            },
        );
    }
    for path in protected_workspace_metadata_paths(&cwd, true) {
        append_default_read_only_entry(&mut entries, absolute_path_value(&path));
    }
    for root in &writable_roots {
        for path in protected_workspace_metadata_paths(root, false) {
            append_default_read_only_entry(&mut entries, absolute_path_value(&path));
        }
    }
    Ok(CatalogManagedFileSystem::Restricted {
        entries,
        glob_scan_max_depth: None,
    })
}

pub(super) fn push_file_system_entry(
    entries: &mut Vec<CatalogFileSystemEntry>,
    path: CatalogFileSystemPath,
    access: CatalogFileSystemAccess,
) {
    entries.push(CatalogFileSystemEntry {
        path,
        access,
        missing_path_behavior: None,
    });
}

pub(super) fn append_default_read_only_entry(
    entries: &mut Vec<CatalogFileSystemEntry>,
    path: CatalogFileSystemPath,
) {
    if entries
        .iter()
        .any(|entry| catalog_file_system_paths_share_target(&entry.path, &path))
    {
        return;
    }
    entries.push(CatalogFileSystemEntry {
        path,
        access: CatalogFileSystemAccess::Read,
        missing_path_behavior: Some(CatalogMissingPathBehavior::Skip),
    });
}

pub(super) fn catalog_file_system_paths_share_target(
    left: &CatalogFileSystemPath,
    right: &CatalogFileSystemPath,
) -> bool {
    match (left, right) {
        (
            CatalogFileSystemPath::Path { path: left },
            CatalogFileSystemPath::Path { path: right },
        ) => native_paths_equal(left, right),
        (
            CatalogFileSystemPath::GlobPattern { pattern: left },
            CatalogFileSystemPath::GlobPattern { pattern: right },
        ) => left == right,
        (
            CatalogFileSystemPath::Special { value: left },
            CatalogFileSystemPath::Special { value: right },
        ) => left == right,
        _ => false,
    }
}

#[cfg(target_os = "windows")]
pub(super) fn native_paths_equal(left: &str, right: &str) -> bool {
    fn normalized(path: &str) -> String {
        let normalized = path.replace('/', "\\");
        let normalized = normalized.trim_end_matches('\\');
        normalized.to_lowercase()
    }
    normalized(left) == normalized(right)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn native_paths_equal(left: &str, right: &str) -> bool {
    Path::new(left) == Path::new(right)
}

pub(super) fn absolute_path_value(path: &Path) -> CatalogFileSystemPath {
    CatalogFileSystemPath::Path {
        path: path.display().to_string(),
    }
}

pub(super) fn protected_workspace_metadata_paths(
    root: &Path,
    protect_missing_codex: bool,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let git = root.join(".git");
    if git.is_file() {
        if let Ok(contents) = std::fs::read_to_string(&git) {
            if let Some(pointer) = contents
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("gitdir:"))
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let pointer = PathBuf::from(pointer);
                let pointer = if pointer.is_absolute() {
                    pointer
                } else {
                    root.join(pointer)
                };
                if pointer.is_absolute() {
                    paths.push(pointer);
                }
            }
        }
        paths.push(git);
    } else if git.is_dir() {
        paths.push(git);
    }
    let agents = root.join(".agents");
    if agents.is_dir() {
        paths.push(agents);
    }
    let codex = root.join(".codex");
    if protect_missing_codex || codex.is_dir() {
        paths.push(codex);
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn canonical_managed_file_system(
    value: &serde_json::Value,
) -> Result<CatalogManagedFileSystem, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "permission_profile.file_system is not an object".to_owned())?;
    let kind = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "permission_profile.file_system.type is missing or invalid".to_owned())?;
    match kind {
        "restricted" => canonical_restricted_file_system(object, "permission_profile.file_system"),
        "unrestricted" => {
            ensure_allowed_keys(object, "permission_profile.file_system", &["type"])?;
            Ok(CatalogManagedFileSystem::Unrestricted)
        }
        other => Err(format!(
            "unsupported permission_profile.file_system type: {other}"
        )),
    }
}

pub(super) fn canonical_split_file_system(
    value: &serde_json::Value,
) -> Result<CatalogFileSystemPolicy, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "file_system_sandbox_policy is not an object".to_owned())?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "file_system_sandbox_policy.kind is missing or invalid".to_owned())?;
    match kind {
        "restricted" => {
            ensure_allowed_keys(
                object,
                "file_system_sandbox_policy",
                &["kind", "entries", "glob_scan_max_depth"],
            )?;
            canonical_restricted_file_system_without_type(object, "file_system_sandbox_policy")
                .map(CatalogFileSystemPolicy::Managed)
        }
        "unrestricted" => {
            ensure_allowed_keys(object, "file_system_sandbox_policy", &["kind"])?;
            Ok(CatalogFileSystemPolicy::Managed(
                CatalogManagedFileSystem::Unrestricted,
            ))
        }
        "external-sandbox" => {
            ensure_allowed_keys(object, "file_system_sandbox_policy", &["kind"])?;
            Ok(CatalogFileSystemPolicy::ExternalSandbox)
        }
        other => Err(format!(
            "unsupported file_system_sandbox_policy kind: {other}"
        )),
    }
}

pub(super) fn canonical_legacy_file_system(
    value: &serde_json::Value,
) -> Result<CatalogManagedFileSystem, String> {
    let object = strict_object(
        value,
        "legacy permission_profile.file_system",
        &["entries", "glob_scan_max_depth"],
    )?;
    canonical_restricted_file_system_without_type(object, "legacy permission_profile.file_system")
}

pub(super) fn canonical_restricted_file_system(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &str,
) -> Result<CatalogManagedFileSystem, String> {
    ensure_allowed_keys(object, context, &["type", "entries", "glob_scan_max_depth"])?;
    canonical_restricted_file_system_without_type(object, context)
}

pub(super) fn canonical_restricted_file_system_without_type(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &str,
) -> Result<CatalogManagedFileSystem, String> {
    let entries = object
        .get("entries")
        .ok_or_else(|| format!("{context}.entries is missing"))?
        .as_array()
        .ok_or_else(|| format!("{context}.entries is not an array"))?
        .iter()
        .map(canonical_file_system_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let depth = match object.get("glob_scan_max_depth") {
        Some(value) => {
            let depth = value.as_u64().filter(|depth| *depth > 0).ok_or_else(|| {
                format!("{context}.glob_scan_max_depth must be a positive integer")
            })?;
            Some(depth)
        }
        None => None,
    };
    Ok(CatalogManagedFileSystem::Restricted {
        entries,
        glob_scan_max_depth: depth,
    })
}

pub(super) fn canonical_file_system_entry(
    value: &serde_json::Value,
) -> Result<CatalogFileSystemEntry, String> {
    let object = strict_object(
        value,
        "file_system entry",
        &["path", "access", "missing_path_behavior"],
    )?;
    let path = canonical_file_system_path(
        object
            .get("path")
            .ok_or_else(|| "file_system entry lacks path".to_owned())?,
    )?;
    let access = object
        .get("access")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "file_system entry access is missing or invalid".to_owned())?;
    let access = match access {
        "read" => CatalogFileSystemAccess::Read,
        "write" => CatalogFileSystemAccess::Write,
        "deny" | "none" => CatalogFileSystemAccess::Deny,
        other => return Err(format!("unsupported file_system access: {other}")),
    };
    let missing_path_behavior = if let Some(behavior) = object.get("missing_path_behavior") {
        let behavior = behavior
            .as_str()
            .ok_or_else(|| "missing_path_behavior is not a string".to_owned())?;
        if behavior != "skip" {
            return Err(format!("unsupported missing_path_behavior: {behavior}"));
        }
        Some(CatalogMissingPathBehavior::Skip)
    } else {
        None
    };
    Ok(CatalogFileSystemEntry {
        path,
        access,
        missing_path_behavior,
    })
}

pub(super) fn canonical_file_system_path(
    value: &serde_json::Value,
) -> Result<CatalogFileSystemPath, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "file_system path is not an object".to_owned())?;
    let kind = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "file_system path.type is missing or invalid".to_owned())?;
    match kind {
        "path" => {
            ensure_allowed_keys(object, "file_system path", &["type", "path"])?;
            let path = non_empty_string(object.get("path"))
                .ok_or_else(|| "file_system path.path is missing or empty".to_owned())?;
            if path.contains('\0') || !Path::new(path).is_absolute() {
                return Err("file_system path.path must be a valid absolute native path".into());
            }
            Ok(CatalogFileSystemPath::Path { path: path.into() })
        }
        "glob_pattern" => {
            ensure_allowed_keys(object, "file_system path", &["type", "pattern"])?;
            let pattern = non_empty_string(object.get("pattern"))
                .ok_or_else(|| "file_system path.pattern is missing or empty".to_owned())?;
            Ok(CatalogFileSystemPath::GlobPattern {
                pattern: pattern.into(),
            })
        }
        "special" => {
            ensure_allowed_keys(object, "file_system path", &["type", "value"])?;
            let special = object
                .get("value")
                .ok_or_else(|| "file_system special path lacks value".to_owned())?
                .as_object()
                .ok_or_else(|| "file_system special path is not a JSON object".to_owned())?;
            let special_kind = special
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "file_system special path.kind is missing or invalid".to_owned())?;
            let special_value = match special_kind {
                "root" | "minimal" | "slash_tmp" | "tmpdir" => {
                    ensure_allowed_keys(special, "file_system special path", &["kind"])?;
                    match special_kind {
                        "root" => CatalogSpecialPath::Root,
                        "minimal" => CatalogSpecialPath::Minimal,
                        "slash_tmp" => CatalogSpecialPath::SlashTmp,
                        "tmpdir" => CatalogSpecialPath::Tmpdir,
                        _ => unreachable!("matched special path kind"),
                    }
                }
                "project_roots" => {
                    ensure_allowed_keys(special, "file_system special path", &["kind", "subpath"])?;
                    let subpath = special
                        .get("subpath")
                        .map(|subpath| {
                            subpath.as_str().map(str::to_owned).ok_or_else(|| {
                                "file_system special path.subpath is not a string".to_owned()
                            })
                        })
                        .transpose()?;
                    CatalogSpecialPath::ProjectRoots { subpath }
                }
                "unknown" => {
                    ensure_allowed_keys(
                        special,
                        "file_system special path",
                        &["kind", "path", "subpath"],
                    )?;
                    let path = non_empty_string(special.get("path")).ok_or_else(|| {
                        "file_system unknown special path.path is missing or empty".to_owned()
                    })?;
                    let subpath = special
                        .get("subpath")
                        .map(|subpath| {
                            subpath.as_str().map(str::to_owned).ok_or_else(|| {
                                "file_system special path.subpath is not a string".to_owned()
                            })
                        })
                        .transpose()?;
                    CatalogSpecialPath::Unknown {
                        path: path.into(),
                        subpath,
                    }
                }
                other => return Err(format!("unsupported special path kind: {other}")),
            };
            Ok(CatalogFileSystemPath::Special {
                value: special_value,
            })
        }
        other => Err(format!("unsupported file_system path type: {other}")),
    }
}

pub(super) fn canonical_network(
    value: &serde_json::Value,
    context: &str,
) -> Result<CatalogNetworkPolicy, String> {
    match value.as_str() {
        Some("restricted") => Ok(CatalogNetworkPolicy::Restricted),
        Some("enabled") => Ok(CatalogNetworkPolicy::Enabled),
        Some(other) => Err(format!("unsupported {context} value: {other}")),
        None => Err(format!("{context} is not a string")),
    }
}

pub(super) fn strict_object<'a>(
    value: &'a serde_json::Value,
    context: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, serde_json::Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} is not a JSON object"))?;
    ensure_allowed_keys(object, context, allowed)?;
    Ok(object)
}

pub(super) fn ensure_allowed_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &str,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = object
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        return Err(format!("{context} contains unsupported field: {key}"));
    }
    Ok(())
}

pub(super) fn required_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &str,
) -> Result<bool, String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format!("{context}.{key} is missing or not a boolean"))
}

pub(super) fn optional_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
    context: &str,
) -> Result<bool, String> {
    match object.get(key) {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("{context}.{key} is not a boolean")),
        None => Ok(default),
    }
}

pub(super) fn catalog_user_message(value: &serde_json::Value) -> Option<String> {
    let payload = value.get("payload")?;
    let record_type = value.get("type").and_then(serde_json::Value::as_str);
    let payload_type = payload.get("type").and_then(serde_json::Value::as_str);
    let content = if record_type == Some("response_item")
        && payload_type == Some("message")
        && payload.get("role").and_then(serde_json::Value::as_str) == Some("user")
    {
        payload.get("content")
    } else if payload_type == Some("user_message") {
        return payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
    } else if payload_type == Some("item_completed")
        && payload
            .get("item")
            .and_then(|item| item.get("type"))
            .and_then(serde_json::Value::as_str)
            == Some("UserMessage")
    {
        payload.get("item").and_then(|item| item.get("content"))
    } else {
        None
    }?;
    catalog_content_text(content)
}

pub(super) fn catalog_content_text(content: &serde_json::Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| text.to_owned());
    }
    let parts = content.as_array()?.iter().filter_map(|part| {
        part.get("text")
            .or_else(|| part.get("input_text"))
            .or_else(|| part.get("output_text"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    });
    let text = parts.collect::<Vec<_>>().join("\n");
    (!text.is_empty()).then_some(text)
}
