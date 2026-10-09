use crate::core::auth::current_timestamp;
use crate::core::models::{
    CoreError, CustomInstructionCurrentState, CustomInstructionHistoryAction,
    CustomInstructionHistoryEntry, CustomInstructionPreviewPayload,
    CustomInstructionProtectionState, CustomInstructionStatePayload,
};
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct InstructionPaths {
    pub global_agents_path: std::path::PathBuf,
    pub custom_instruction_history_dir: std::path::PathBuf,
}

impl InstructionPaths {
    pub fn for_target(
        paths: &CodexPaths,
        clients: &super::client_config::ClientPaths,
        target: super::targets::TargetId,
    ) -> Result<Self, CoreError> {
        use super::targets::TargetId;
        match target {
            TargetId::Codex => Ok(Self {
                global_agents_path: paths.global_agents_path.clone(),
                custom_instruction_history_dir: paths.custom_instruction_history_dir.clone(),
            }),
            TargetId::ClaudeCode => Ok(Self {
                global_agents_path: clients.claude_root().join("CLAUDE.md"),
                custom_instruction_history_dir: paths
                    .app_data_dir
                    .join("custom-instructions/claude-code/history"),
            }),
            _ => Err(CoreError::InvalidData(
                "Unsupported instruction target".into(),
            )),
        }
    }

    fn ensure_directories(&self) -> Result<(), CoreError> {
        std::fs::create_dir_all(&self.custom_instruction_history_dir)?;
        Ok(())
    }
}

const MANAGED_START_MARKER: &str = "<!-- AGENT_HARBOR_CUSTOM_INSTRUCTIONS_START -->";
const MANAGED_END_MARKER: &str = "<!-- AGENT_HARBOR_CUSTOM_INSTRUCTIONS_END -->";
const HISTORY_LIMIT: usize = 10;

#[derive(Debug, Clone)]
struct ParsedManagedBlock {
    file_exists: bool,
    protection_state: CustomInstructionProtectionState,
    issue_message: Option<String>,
    managed_block_present: bool,
    managed_content: String,
    raw_content: String,
    block_start: Option<usize>,
    block_end: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomInstructionHistorySnapshot {
    id: String,
    created_at: i64,
    action: CustomInstructionHistoryAction,
    source: String,
    template_code: Option<String>,
    template_title: Option<String>,
    full_content: String,
}

impl CustomInstructionHistorySnapshot {
    fn to_entry(&self) -> CustomInstructionHistoryEntry {
        CustomInstructionHistoryEntry {
            id: self.id.clone(),
            created_at: self.created_at,
            action: self.action.clone(),
            source: self.source.clone(),
            template_code: self.template_code.clone(),
            template_title: self.template_title.clone(),
        }
    }
}

pub fn load_state(paths: &InstructionPaths) -> Result<CustomInstructionStatePayload, CoreError> {
    paths.ensure_directories()?;
    let parsed = parse_global_file(paths)?;
    let history = load_history(paths)?;
    let latest = history.first().cloned();

    Ok(CustomInstructionStatePayload {
        current: CustomInstructionCurrentState {
            global_path: paths.global_agents_path.display().to_string(),
            file_exists: parsed.file_exists,
            managed_block_present: parsed.managed_block_present,
            protection_state: parsed.protection_state,
            issue_message: parsed.issue_message,
            managed_content: parsed.managed_content,
            file_content: parsed.raw_content,
            last_applied_at: latest.as_ref().map(|item| item.created_at),
            last_template_code: latest.as_ref().and_then(|item| item.template_code.clone()),
            last_template_title: latest.as_ref().and_then(|item| item.template_title.clone()),
        },
        history: history.into_iter().map(|item| item.to_entry()).collect(),
    })
}

pub fn preview_apply(
    paths: &InstructionPaths,
    content: &str,
) -> Result<CustomInstructionPreviewPayload, CoreError> {
    paths.ensure_directories()?;
    validate_managed_content(content)?;
    let parsed = parse_global_file(paths)?;

    if parsed.protection_state == CustomInstructionProtectionState::Protected {
        return Err(CoreError::InvalidData(parsed.issue_message.unwrap_or_else(
            || "Global instruction file is in protected mode".to_string(),
        )));
    }

    let resulting_content = compose_with_managed_content(&parsed, content)?;

    Ok(CustomInstructionPreviewPayload {
        global_path: paths.global_agents_path.display().to_string(),
        protection_state: parsed.protection_state,
        issue_message: parsed.issue_message,
        current_managed_content: parsed.managed_content,
        next_managed_content: normalize_managed_content(content),
        resulting_content,
    })
}

pub fn apply_managed_content(
    paths: &InstructionPaths,
    content: &str,
    template_code: Option<String>,
    template_title: Option<String>,
    source: Option<String>,
) -> Result<CustomInstructionStatePayload, CoreError> {
    paths.ensure_directories()?;
    validate_managed_content(content)?;
    let parsed = parse_global_file(paths)?;

    if parsed.protection_state == CustomInstructionProtectionState::Protected {
        return Err(CoreError::InvalidData(parsed.issue_message.unwrap_or_else(
            || "Global instruction file is in protected mode".to_string(),
        )));
    }

    let next_content = compose_with_managed_content(&parsed, content)?;
    if next_content == parsed.raw_content {
        return load_state(paths);
    }

    save_history_snapshot(
        paths,
        CustomInstructionHistoryAction::Apply,
        source.unwrap_or_else(|| "one_click".to_string()),
        template_code,
        template_title,
        parsed.raw_content.clone(),
    )?;
    write_checked(paths, &parsed.raw_content, &next_content)?;
    load_state(paths)
}

pub fn clear_managed_block(
    paths: &InstructionPaths,
) -> Result<CustomInstructionStatePayload, CoreError> {
    paths.ensure_directories()?;
    let parsed = parse_global_file(paths)?;

    if parsed.protection_state == CustomInstructionProtectionState::Protected {
        return Err(CoreError::InvalidData(parsed.issue_message.unwrap_or_else(
            || "Global instruction file is in protected mode".to_string(),
        )));
    }

    if !parsed.managed_block_present {
        return load_state(paths);
    }

    let cleared = clear_managed_content(&parsed)?;
    if cleared == parsed.raw_content {
        return load_state(paths);
    }

    save_history_snapshot(
        paths,
        CustomInstructionHistoryAction::Clear,
        "clear".to_string(),
        None,
        None,
        parsed.raw_content.clone(),
    )?;
    write_checked(paths, &parsed.raw_content, &cleared)?;
    load_state(paths)
}

pub fn rollback_history(
    paths: &InstructionPaths,
    history_id: &str,
) -> Result<CustomInstructionStatePayload, CoreError> {
    paths.ensure_directories()?;
    let snapshot = find_history_snapshot(paths, history_id)?
        .ok_or_else(|| CoreError::NotFound(format!("History entry not found: {history_id}")))?;
    let parsed = parse_global_file(paths)?;

    if parsed.protection_state == CustomInstructionProtectionState::Protected {
        return Err(CoreError::InvalidData(
            "Repair the instruction markers before rollback".into(),
        ));
    }
    let previous = parse_content(true, snapshot.full_content.clone())?;
    if previous.protection_state == CustomInstructionProtectionState::Protected {
        return Err(CoreError::InvalidData(
            "History contains invalid instruction markers".into(),
        ));
    }
    let restored = if previous.managed_block_present {
        compose_with_managed_content(&parsed, &previous.managed_content)?
    } else {
        clear_managed_content(&parsed)?
    };

    save_history_snapshot(
        paths,
        CustomInstructionHistoryAction::Rollback,
        "rollback".to_string(),
        snapshot.template_code.clone(),
        snapshot.template_title.clone(),
        parsed.raw_content.clone(),
    )?;

    write_checked(paths, &parsed.raw_content, &restored)?;

    load_state(paths)
}

fn parse_global_file(paths: &InstructionPaths) -> Result<ParsedManagedBlock, CoreError> {
    let file_exists = paths.global_agents_path.exists();
    let raw_content = if file_exists {
        std::fs::read_to_string(&paths.global_agents_path)?
    } else {
        String::new()
    };

    parse_content(file_exists, raw_content)
}

fn write_checked(paths: &InstructionPaths, before: &str, next: &str) -> Result<(), CoreError> {
    if parse_global_file(paths)?.raw_content != before {
        return Err(CoreError::OperationFailed(
            "customInstructions.fileChanged".into(),
        ));
    }
    if next.is_empty() {
        match std::fs::remove_file(&paths.global_agents_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    } else {
        super::storage::write_atomic(&paths.global_agents_path, next.as_bytes())?;
    }
    Ok(())
}

fn parse_content(file_exists: bool, raw_content: String) -> Result<ParsedManagedBlock, CoreError> {
    let start_positions: Vec<usize> = raw_content
        .match_indices(MANAGED_START_MARKER)
        .map(|(idx, _)| idx)
        .collect();
    let end_positions: Vec<usize> = raw_content
        .match_indices(MANAGED_END_MARKER)
        .map(|(idx, _)| idx)
        .collect();

    if start_positions.is_empty() && end_positions.is_empty() {
        return Ok(ParsedManagedBlock {
            file_exists,
            protection_state: CustomInstructionProtectionState::Unmanaged,
            issue_message: None,
            managed_block_present: false,
            managed_content: String::new(),
            raw_content,
            block_start: None,
            block_end: None,
        });
    }

    if start_positions.len() != 1 || end_positions.len() != 1 {
        return Ok(ParsedManagedBlock {
            file_exists,
            protection_state: CustomInstructionProtectionState::Protected,
            issue_message: Some(
                "检测到重复或不完整的 AgentHarbor 自定义指令标记，请先手动修复全局指令文件。"
                    .to_string(),
            ),
            managed_block_present: false,
            managed_content: String::new(),
            raw_content,
            block_start: None,
            block_end: None,
        });
    }

    let block_start = start_positions[0];
    let block_end_marker_start = end_positions[0];
    if block_end_marker_start < block_start {
        return Ok(ParsedManagedBlock {
            file_exists,
            protection_state: CustomInstructionProtectionState::Protected,
            issue_message: Some(
                "AgentHarbor 自定义指令标记顺序异常，请先手动修复全局指令文件。".to_string(),
            ),
            managed_block_present: false,
            managed_content: String::new(),
            raw_content,
            block_start: None,
            block_end: None,
        });
    }

    let content_start = block_start + MANAGED_START_MARKER.len();
    // Managed content is always exposed with LF so CRLF files do not look edited.
    let content = raw_content[content_start..block_end_marker_start]
        .trim_matches(['\r', '\n'])
        .replace("\r\n", "\n");

    Ok(ParsedManagedBlock {
        file_exists,
        protection_state: CustomInstructionProtectionState::Ready,
        issue_message: None,
        managed_block_present: true,
        managed_content: content,
        raw_content,
        block_start: Some(block_start),
        block_end: Some(block_end_marker_start + MANAGED_END_MARKER.len()),
    })
}

fn compose_with_managed_content(
    parsed: &ParsedManagedBlock,
    content: &str,
) -> Result<String, CoreError> {
    let normalized = normalize_managed_content(content);
    let rendered_block = render_managed_block(&normalized);

    if let (Some(start), Some(end)) = (parsed.block_start, parsed.block_end) {
        let mut next = String::with_capacity(parsed.raw_content.len() + rendered_block.len() + 8);
        next.push_str(&parsed.raw_content[..start]);
        next.push_str(&rendered_block);
        // The rendered block ends with its own line break; drop the one that followed
        // the old end marker so repeated applies do not grow the file by a blank line.
        let after = &parsed.raw_content[end..];
        let after = after
            .strip_prefix("\r\n")
            .or_else(|| after.strip_prefix('\n'))
            .unwrap_or(after);
        next.push_str(after);
        return Ok(match_line_endings(&parsed.raw_content, next));
    }

    if parsed.raw_content.trim().is_empty() {
        return Ok(rendered_block);
    }

    let mut next = parsed.raw_content.replace("\r\n", "\n");
    if !next.ends_with('\n') {
        next.push('\n');
    }
    if !next.ends_with("\n\n") {
        next.push('\n');
    }
    next.push_str(&rendered_block);
    Ok(match_line_endings(&parsed.raw_content, next))
}

/// Keep a CRLF file CRLF: the managed block is rendered with LF internally.
fn match_line_endings(original: &str, next: String) -> String {
    if original.contains("\r\n") {
        next.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        next
    }
}

fn clear_managed_content(parsed: &ParsedManagedBlock) -> Result<String, CoreError> {
    let (start, end) = match (parsed.block_start, parsed.block_end) {
        (Some(start), Some(end)) => (start, end),
        _ => return Ok(parsed.raw_content.clone()),
    };

    let before = parsed.raw_content[..start].trim_end_matches(['\r', '\n']);
    let after = parsed.raw_content[end..].trim_start_matches(['\r', '\n']);

    let next = if before.is_empty() && after.is_empty() {
        String::new()
    } else if before.is_empty() {
        after.to_string()
    } else if after.is_empty() {
        format!("{before}\n")
    } else {
        format!("{before}\n\n{after}")
    };

    Ok(match_line_endings(&parsed.raw_content, next))
}

fn render_managed_block(content: &str) -> String {
    if content.is_empty() {
        format!("{MANAGED_START_MARKER}\n{MANAGED_END_MARKER}\n")
    } else {
        format!("{MANAGED_START_MARKER}\n{content}\n{MANAGED_END_MARKER}\n")
    }
}

fn normalize_managed_content(content: &str) -> String {
    content.trim().trim_matches('\n').to_string()
}

fn validate_managed_content(content: &str) -> Result<(), CoreError> {
    if content.contains(MANAGED_START_MARKER) || content.contains(MANAGED_END_MARKER) {
        return Err(CoreError::InvalidData(
            "自定义指令内容不能包含 AgentHarbor 受控区块标记。".to_string(),
        ));
    }
    Ok(())
}

fn save_history_snapshot(
    paths: &InstructionPaths,
    action: CustomInstructionHistoryAction,
    source: String,
    template_code: Option<String>,
    template_title: Option<String>,
    full_content: String,
) -> Result<(), CoreError> {
    let created_at = current_timestamp();
    let id = format!("{created_at}-{}", &Uuid::new_v4().to_string()[..8]);
    let snapshot = CustomInstructionHistorySnapshot {
        id: id.clone(),
        created_at,
        action,
        source,
        template_code,
        template_title,
        full_content,
    };
    let path = paths
        .custom_instruction_history_dir
        .join(format!("{id}.json"));
    let serialized = serde_json::to_string_pretty(&snapshot)?;
    super::storage::write_atomic(&path, serialized.as_bytes())?;
    trim_history(paths)?;
    Ok(())
}

fn trim_history(paths: &InstructionPaths) -> Result<(), CoreError> {
    let mut items = load_history(paths)?;
    if items.len() <= HISTORY_LIMIT {
        return Ok(());
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    for snapshot in items.into_iter().skip(HISTORY_LIMIT) {
        let path = paths
            .custom_instruction_history_dir
            .join(format!("{}.json", snapshot.id));
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

fn load_history(
    paths: &InstructionPaths,
) -> Result<Vec<CustomInstructionHistorySnapshot>, CoreError> {
    if !paths.custom_instruction_history_dir.exists() {
        return Ok(vec![]);
    }

    let mut items = Vec::new();
    for entry in std::fs::read_dir(&paths.custom_instruction_history_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        let raw = std::fs::read_to_string(&path)?;
        if let Ok(snapshot) = serde_json::from_str::<CustomInstructionHistorySnapshot>(&raw) {
            if valid_history_id(&snapshot.id)
                && path.file_stem().and_then(|value| value.to_str()) == Some(snapshot.id.as_str())
            {
                items.push(snapshot);
            }
        }
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    Ok(items)
}

fn find_history_snapshot(
    paths: &InstructionPaths,
    history_id: &str,
) -> Result<Option<CustomInstructionHistorySnapshot>, CoreError> {
    if !valid_history_id(history_id) {
        return Err(CoreError::InvalidData(
            "Invalid instruction history ID".into(),
        ));
    }
    let path = paths
        .custom_instruction_history_dir
        .join(format!("{history_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
        return Err(CoreError::InvalidData(
            "History must be a regular file".into(),
        ));
    }
    let raw = std::fs::read_to_string(path)?;
    let snapshot = serde_json::from_str::<CustomInstructionHistorySnapshot>(&raw)?;
    Ok(Some(snapshot))
}

fn valid_history_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_file_content_is_readable_without_a_managed_block_or_with_broken_markers() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstructionPaths {
            global_agents_path: temp.path().join("AGENTS.md"),
            custom_instruction_history_dir: temp.path().join("history"),
        };
        let missing = load_state(&paths).unwrap().current;
        assert!(!missing.file_exists);
        assert_eq!(missing.file_content, "");
        let user_rules = "User rules\r\n<script>literal instruction</script>\r\n";
        std::fs::write(&paths.global_agents_path, user_rules).unwrap();
        let current = load_state(&paths).unwrap().current;
        assert!(!current.managed_block_present);
        assert_eq!(current.file_content, user_rules);
        let broken = format!("{user_rules}{MANAGED_START_MARKER}\r\nInterrupted edit");
        std::fs::write(&paths.global_agents_path, &broken).unwrap();
        let current = load_state(&paths).unwrap().current;
        assert_eq!(
            current.protection_state,
            CustomInstructionProtectionState::Protected
        );
        assert_eq!(current.file_content, broken);
    }

    #[test]
    fn clients_keep_files_and_history_separate_and_rollback_preserves_user_text() {
        let temp = tempfile::tempdir().unwrap();
        let codex = CodexPaths::from_home(temp.path().join(".codex"));
        let clients = crate::core::client_config::ClientPaths::from_home(temp.path());
        let a =
            InstructionPaths::for_target(&codex, &clients, crate::core::targets::TargetId::Codex)
                .unwrap();
        let b = InstructionPaths::for_target(
            &codex,
            &clients,
            crate::core::targets::TargetId::ClaudeCode,
        )
        .unwrap();
        std::fs::create_dir_all(clients.claude_root()).unwrap();
        std::fs::write(&b.global_agents_path, "User Claude rules\n").unwrap();
        assert_eq!(b.global_agents_path, temp.path().join(".claude/CLAUDE.md"));
        assert_eq!(
            a.custom_instruction_history_dir,
            codex.custom_instruction_history_dir
        );
        assert!(!a.global_agents_path.exists());
        let state = apply_managed_content(&b, "Claude instructions", None, None, None).unwrap();
        assert_eq!(
            state.current.file_content,
            std::fs::read_to_string(&b.global_agents_path).unwrap()
        );
        assert!(state.current.file_content.contains("User Claude rules"));
        apply_managed_content(&a, "Codex instructions", None, None, None).unwrap();
        let raw = std::fs::read_to_string(&b.global_agents_path).unwrap();
        std::fs::write(&b.global_agents_path, format!("{raw}\nLater user rules\n")).unwrap();
        rollback_history(&b, &state.history[0].id).unwrap();
        let raw = std::fs::read_to_string(&b.global_agents_path).unwrap();
        assert!(raw.contains("User Claude rules"));
        assert!(raw.contains("Later user rules"));
        assert!(!raw.contains("Claude instructions"));
        assert_eq!(load_state(&b).unwrap().current.file_content, raw);
        assert_eq!(
            load_state(&a).unwrap().current.managed_content,
            "Codex instructions"
        );
        assert!(rollback_history(&a, &state.history[0].id).is_err());
        assert!(InstructionPaths::for_target(
            &codex,
            &clients,
            crate::core::targets::TargetId::OpenCode
        )
        .is_err());
    }

    #[test]
    fn checked_write_rejects_external_changes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstructionPaths {
            global_agents_path: temp.path().join("CLAUDE.md"),
            custom_instruction_history_dir: temp.path().join("history"),
        };
        std::fs::write(&paths.global_agents_path, "External rules").unwrap();
        assert!(write_checked(&paths, "Old rules", "New rules").is_err());
        assert_eq!(
            std::fs::read_to_string(&paths.global_agents_path).unwrap(),
            "External rules"
        );
    }

    #[test]
    fn crlf_files_keep_crlf_and_managed_content_has_no_carriage_returns() {
        let raw = format!(
            "# Notes\r\n\r\n{MANAGED_START_MARKER}\r\nold\r\nline\r\n{MANAGED_END_MARKER}\r\ntail\r\n"
        );
        let parsed = parse_content(true, raw).unwrap();
        assert_eq!(parsed.managed_content, "old\nline");

        let next = compose_with_managed_content(&parsed, "new\nline").unwrap();
        assert!(
            !next.replace("\r\n", "").contains('\n'),
            "no bare LF: {next:?}"
        );
        assert!(next.contains("new\r\nline"));
        assert!(next.ends_with("tail\r\n"));

        let cleared = clear_managed_content(&parsed).unwrap();
        assert_eq!(cleared, "# Notes\r\n\r\ntail\r\n");
    }

    #[test]
    fn lf_files_are_unchanged_by_line_ending_handling() {
        let raw = format!("a\n\n{MANAGED_START_MARKER}\nold\n{MANAGED_END_MARKER}\n");
        let parsed = parse_content(true, raw).unwrap();
        let next = compose_with_managed_content(&parsed, "new").unwrap();
        assert_eq!(
            next,
            format!("a\n\n{MANAGED_START_MARKER}\nnew\n{MANAGED_END_MARKER}\n")
        );
        let again =
            compose_with_managed_content(&parse_content(true, next.clone()).unwrap(), "new")
                .unwrap();
        assert_eq!(again, next, "re-applying must be idempotent");
    }
}
