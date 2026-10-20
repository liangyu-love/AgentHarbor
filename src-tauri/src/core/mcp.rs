//! MCP servers: the library the MCP page keeps, and how each agent's own
//! configuration holds them. This file is Codex's `config.toml` side, which
//! keeps comments, managed blocks and the rest of every server table; the
//! other agents are in `apps.rs`, the library in `library.rs`.
use crate::core::codex_config::CodexConfigRepository;
use crate::core::models::*;
use crate::platform::paths::CodexPaths;
use std::collections::HashMap;

mod apps;
mod library;
mod paste;
pub use apps::{McpApp, McpKind, McpSpec};
pub use library::*;
pub use paste::{parse_pasted, PastedServer};

/// Keys of a Codex server table the MCP page writes. The rest of the table
/// (timeouts, tool lists, auth) belongs to the user and is kept.
const CODEX_MANAGED_KEYS: [&str; 10] = [
    "enabled",
    "transport",
    "type",
    "command",
    "args",
    "env",
    "cwd",
    "url",
    "headers",
    "http_headers",
];

const MANAGED_BLOCK_BEGIN_PREFIX: &str = "# --- AgentHarbor Managed";
const MANAGED_BLOCK_END_SUFFIX: &str = "# --- End AgentHarbor Managed";

fn is_managed_block_begin(line: &str) -> bool {
    line.starts_with(MANAGED_BLOCK_BEGIN_PREFIX)
}

fn is_managed_block_end(line: &str) -> bool {
    line.starts_with(MANAGED_BLOCK_END_SUFFIX)
}

const BOTTOM_MANAGED_BLOCK_BEGIN: &str = "# --- AgentHarbor Managed Block (bottom) ---";
const LEGACY_TOP_MANAGED_BLOCK_BEGIN: &str = "# --- AgentHarbor Managed Block (top) ---";
const ROUTER_TOP_MANAGED_BLOCK_BEGIN: &str = "# --- AgentHarbor Managed Block (router-top) ---";

/// Every `[mcp_servers.<name>]` table in Codex's config text.
pub(crate) fn codex_servers(text: &str) -> Result<Vec<(String, toml::Table)>, CoreError> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let document: toml::Value = text.parse()?;
    Ok(document
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .map(|servers| {
            servers
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), value.as_table()?.clone())))
                .collect()
        })
        .unwrap_or_default())
}

pub fn read_codex_servers(paths: &CodexPaths) -> Result<Vec<(String, toml::Table)>, CoreError> {
    codex_servers(&CodexConfigRepository::new(paths).read_text()?)
}

/// Sets server `name`'s managed keys to `managed`, keeping the rest of its
/// table and of the file.
pub(crate) fn set_codex_server(
    paths: &CodexPaths,
    name: &str,
    managed: &toml::Table,
) -> Result<(), CoreError> {
    CodexConfigRepository::new(paths)
        .update(|original| render_codex_server(original, name, managed))?;
    Ok(())
}

fn render_codex_server(
    original: &str,
    name: &str,
    managed: &toml::Table,
) -> Result<String, CoreError> {
    let original_value: toml::Value = original.parse()?;
    let document = parse_mcp_document(original)?;
    let mut preserved = original_value
        .get("mcp_servers")
        .and_then(|v| v.get(name))
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    if !preserved.is_empty() && !document.blocks.contains_key(name) {
        return Err(CoreError::InvalidData(
            "Expand inline MCP configuration into a table before editing".into(),
        ));
    }
    for key in CODEX_MANAGED_KEYS {
        preserved.remove(key);
    }
    preserved.extend(managed.clone());
    let mut servers = toml::Table::new();
    servers.insert(name.to_owned(), toml::Value::Table(preserved));
    let mut wrapper = toml::Table::new();
    wrapper.insert("mcp_servers".into(), toml::Value::Table(servers));
    let rendered = toml::to_string_pretty(&wrapper)?
        .lines()
        .map(str::to_owned)
        .collect();

    let updated = if let Some(existing) = document.blocks.get(name) {
        let relocate = existing
            .iter()
            .any(|block| should_relocate_mcp_block(&document.lines, block));
        let mut lines = document.lines.clone();
        for block in existing.iter().rev() {
            lines.drain(block.start..block.end);
        }
        if relocate {
            insert_mcp_block(lines, rendered)
        } else {
            lines.splice(existing[0].start..existing[0].start, rendered);
            lines
        }
    } else {
        insert_mcp_block(document.lines.clone(), rendered)
    };

    Ok(updated.join(
        "
",
    ))
}

pub fn remove_mcp_server(paths: &CodexPaths, name: &str) -> Result<(), CoreError> {
    CodexConfigRepository::new(paths).update(|original| {
        let mut document = original
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| CoreError::InvalidData(error.to_string()))?;
        // Establish the complete semantic deletion, including dotted keys
        // and child tables that may occur elsewhere in the document.
        document
            .get_mut("mcp_servers")
            .and_then(toml_edit::Item::as_table_like_mut)
            .and_then(|servers| servers.remove(name))
            .ok_or_else(|| CoreError::NotFound(format!("MCP server not found: {name}")))?;
        let semantic = document.to_string();
        let expected: toml::Value = semantic.parse()?;
        let source = parse_mcp_document(original)?;
        if let Some(blocks) = source.blocks.get(name) {
            // Keep managed-block boundary comments, which TOML associates
            // with the following table even when they belong to another block.
            let mut lines = source.lines.clone();
            for block in blocks.iter().rev() {
                lines.drain(block.start..block.end);
            }
            let updated = lines.join("\n");
            if updated.parse::<toml::Value>().ok().as_ref() == Some(&expected) {
                return Ok(updated);
            }
        }
        // Mixed dotted-key/table configurations can leave fields outside the
        // header ranges. Use semantic removal only if its comments are safe.
        for marker in original
            .lines()
            .map(str::trim)
            .filter(|line| is_managed_block_begin(line) || is_managed_block_end(line))
        {
            if original
                .lines()
                .filter(|line| line.trim() == marker)
                .count()
                != semantic
                    .lines()
                    .filter(|line| line.trim() == marker)
                    .count()
            {
                return Err(CoreError::InvalidData(
                    "Expand mixed MCP configuration into tables before removing it".into(),
                ));
            }
        }
        Ok(semantic)
    })?;
    Ok(())
}

fn insert_mcp_block(mut lines: Vec<String>, rendered: Vec<String>) -> Vec<String> {
    let mut insert_pos = find_mcp_insert_pos(&lines);

    // 移除插入点之前的尾部空行，避免反复新增 MCP 后空行膨胀。
    while insert_pos > 0
        && lines
            .get(insert_pos - 1)
            .map_or(false, |l| l.trim().is_empty())
    {
        lines.remove(insert_pos - 1);
        insert_pos -= 1;
    }

    if insert_pos > 0 {
        lines.insert(insert_pos, String::new());
        insert_pos += 1;
    }
    lines.splice(insert_pos..insert_pos, rendered);
    lines
}

fn find_mcp_insert_pos(lines: &[String]) -> usize {
    lines
        .iter()
        .position(|line| line.trim() == BOTTOM_MANAGED_BLOCK_BEGIN)
        .unwrap_or(lines.len())
}

fn should_relocate_mcp_block(lines: &[String], block: &McpBlock) -> bool {
    is_inside_managed_block(lines, block.start) || is_before_top_managed_block(lines, block.start)
}

fn is_before_top_managed_block(lines: &[String], index: usize) -> bool {
    lines
        .iter()
        .position(|line| {
            let trimmed = line.trim();
            trimmed == LEGACY_TOP_MANAGED_BLOCK_BEGIN || trimmed == ROUTER_TOP_MANAGED_BLOCK_BEGIN
        })
        .map_or(false, |top_start| index < top_start)
}

fn is_inside_managed_block(lines: &[String], index: usize) -> bool {
    let mut inside = false;
    for (i, line) in lines.iter().enumerate() {
        if i == index {
            return inside;
        }
        let trimmed = line.trim();
        if is_managed_block_begin(trimmed) {
            inside = true;
            continue;
        }
        if is_managed_block_end(trimmed) {
            inside = false;
        }
    }
    false
}

struct McpDocument {
    lines: Vec<String>,
    blocks: HashMap<String, Vec<McpBlock>>,
}

struct McpBlock {
    start: usize,
    end: usize,
}

fn collect_value_spans(item: &toml_edit::Item, spans: &mut Vec<std::ops::Range<usize>>) {
    match item {
        toml_edit::Item::Value(value) => {
            if let Some(span) = value.span() {
                spans.push(span);
            }
        }
        toml_edit::Item::Table(table) => {
            for (_, child) in table.iter() {
                collect_value_spans(child, spans);
            }
        }
        toml_edit::Item::ArrayOfTables(tables) => {
            for table in tables.iter() {
                for (_, child) in table.iter() {
                    collect_value_spans(child, spans);
                }
            }
        }
        toml_edit::Item::None => {}
    }
}

fn parse_mcp_document(text: &str) -> Result<McpDocument, CoreError> {
    let parsed = toml_edit::ImDocument::parse(text)
        .map_err(|error| CoreError::InvalidData(error.to_string()))?;
    let mut value_spans = Vec::new();
    collect_value_spans(parsed.as_item(), &mut value_spans);
    value_spans.sort_by_key(|span| span.start);
    let mut value_spans = value_spans.into_iter().peekable();
    let line_offsets: Vec<_> = text
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some(start)
        })
        .collect();
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut blocks: HashMap<String, Vec<McpBlock>> = HashMap::new();
    let mut current_name: Option<String> = None;
    let mut current_start: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        let offset = line_offsets[i];
        while value_spans.peek().is_some_and(|span| span.end <= offset) {
            value_spans.next();
        }
        // Header-like lines and managed markers inside multiline strings or
        // arrays are values, not configuration boundaries.
        if value_spans
            .peek()
            .is_some_and(|span| span.contains(&offset))
        {
            continue;
        }
        if is_managed_block_begin(line.trim()) || is_managed_block_end(line.trim()) {
            if let (Some(name), Some(start)) = (current_name.take(), current_start.take()) {
                blocks
                    .entry(name)
                    .or_default()
                    .push(McpBlock { start, end: i });
            }
        }
        let trimmed = line.trim();
        if !trimmed.starts_with('[') {
            continue;
        }
        // Let TOML handle quoted names, escapes and trailing comments.
        let Ok(header) = trimmed.parse::<toml::Value>() else {
            continue;
        };
        let section = header
            .get("mcp_servers")
            .and_then(toml::Value::as_table)
            .and_then(|servers| servers.keys().next());

        if let (Some(ref name), Some(start)) = (&current_name, current_start) {
            let continues = section.is_some_and(|server_name| server_name == name);
            if !continues {
                blocks
                    .entry(name.clone())
                    .or_default()
                    .push(McpBlock { start, end: i });
                current_name = None;
                current_start = None;
            }
        }

        if let Some(server_name) = section {
            if current_name.is_none() {
                current_name = Some(server_name.clone());
                current_start = Some(i);
            }
        }
    }

    if let (Some(name), Some(start)) = (current_name, current_start) {
        blocks.entry(name).or_default().push(McpBlock {
            start,
            end: lines.len(),
        });
    }

    Ok(McpDocument { lines, blocks })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTTOM_MANAGED_BLOCK_END: &str = "# --- End AgentHarbor Managed Block (bottom) ---";
    const ROUTER_TOP_MANAGED_BLOCK_END: &str =
        "# --- End AgentHarbor Managed Block (router-top) ---";

    /// The managed keys for a stdio server running `command`.
    fn stdio(command: &str) -> toml::Table {
        let mut table = toml::Table::new();
        table.insert("enabled".into(), true.into());
        table.insert("command".into(), command.into());
        table
    }

    fn names(paths: &CodexPaths) -> Vec<String> {
        read_codex_servers(paths)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    fn temp_paths() -> (tempfile::TempDir, CodexPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join(".codex"));
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        (dir, paths)
    }

    #[test]
    fn remove_mcp_removes_separated_child_tables_and_keeps_other_servers() {
        let (_dir, paths) = temp_paths();
        let original = "# User config\n[mcp_servers.foo]\ncommand = 'npx'\n\n[features]\nweb_search_request = true\n\n[mcp_servers.keep]\ncommand = 'node'\n\n[mcp_servers.foo.env]\nTOKEN = 'synthetic'\n";
        std::fs::write(&paths.config_path, original).unwrap();
        remove_mcp_server(&paths, "foo").unwrap();
        assert_eq!(names(&paths), ["keep"]);
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(updated.contains("# User config"));
        let parsed: toml::Value = updated.parse().unwrap();
        assert_eq!(
            parsed["features"]["web_search_request"].as_bool(),
            Some(true)
        );
        assert!(parsed["mcp_servers"].get("foo").is_none());
    }

    #[test]
    fn remove_mcp_supports_literal_quoted_server_names() {
        let (_dir, paths) = temp_paths();
        std::fs::write(
            &paths.config_path,
            "[mcp_servers.'quoted.name']\ncommand = 'node'\n",
        )
        .unwrap();
        remove_mcp_server(&paths, "quoted.name").unwrap();
        assert!(names(&paths).is_empty());
    }

    #[test]
    fn remove_mcp_removes_mixed_dotted_keys_and_explicit_child_tables() {
        let (_dir, paths) = temp_paths();
        std::fs::write(
            &paths.config_path,
            "mcp_servers.foo.command = 'node'\n\n[mcp_servers.foo.env]\nTOKEN = 'synthetic'\n\n[features]\nweb_search_request = true\n",
        )
        .unwrap();
        remove_mcp_server(&paths, "foo").unwrap();
        assert!(names(&paths).is_empty());
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();
        let parsed: toml::Value = updated.parse().unwrap();
        assert_eq!(
            parsed["features"]["web_search_request"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn mcp_edits_preserve_separated_child_tables_unknown_fields_and_boundaries() {
        let (_dir, paths) = temp_paths();
        let original = format!(
            "{ROUTER_TOP_MANAGED_BLOCK_BEGIN}\nmodel = 'original'\n{ROUTER_TOP_MANAGED_BLOCK_END}\n\n\
             [mcp_servers.foo]\ncommand = 'old'\nstartup_timeout_sec = 30\n\n\
             [features]\nweb_search_request = true\n\n\
             [mcp_servers.keep]\ncommand = 'node'\n\n\
             [mcp_servers.foo.env]\nTOKEN = 'synthetic'\n\n\
             [projects.'D:/work']\ntrust_level = 'trusted'\n\n\
             [mcp_servers.foo.extra]\nretries = 3\n\n\
             {BOTTOM_MANAGED_BLOCK_BEGIN}\n[model_providers.relay]\nname = 'Relay'\n{BOTTOM_MANAGED_BLOCK_END}\n"
        );
        std::fs::write(&paths.config_path, original).unwrap();
        let mut managed = stdio("updated");
        managed.insert("enabled".into(), false.into());
        managed.insert(
            "env".into(),
            toml::Value::Table(toml::Table::from_iter([(
                "TOKEN".into(),
                "synthetic".into(),
            )])),
        );
        set_codex_server(&paths, "foo", &managed).unwrap();
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();
        let parsed: toml::Value = updated.parse().unwrap();
        let foo = &parsed["mcp_servers"]["foo"];
        assert_eq!(foo["command"].as_str(), Some("updated"));
        assert_eq!(foo["enabled"].as_bool(), Some(false));
        assert_eq!(foo["startup_timeout_sec"].as_integer(), Some(30));
        assert_eq!(foo["env"]["TOKEN"].as_str(), Some("synthetic"));
        assert_eq!(foo["extra"]["retries"].as_integer(), Some(3));
        assert_eq!(parsed["model"].as_str(), Some("original"));
        assert_eq!(
            parsed["features"]["web_search_request"].as_bool(),
            Some(true)
        );
        assert_eq!(
            parsed["mcp_servers"]["keep"]["command"].as_str(),
            Some("node")
        );
        assert_eq!(
            parsed["projects"]["D:/work"]["trust_level"].as_str(),
            Some("trusted")
        );
        for marker in [
            ROUTER_TOP_MANAGED_BLOCK_BEGIN,
            ROUTER_TOP_MANAGED_BLOCK_END,
            BOTTOM_MANAGED_BLOCK_BEGIN,
            BOTTOM_MANAGED_BLOCK_END,
        ] {
            assert_eq!(updated.matches(marker).count(), 1, "{marker}");
        }
    }

    #[test]
    fn mcp_edits_support_literal_quoted_and_escaped_server_names() {
        for (header, name) in [
            ("'quoted.name'", "quoted.name"),
            ("'quoted.#name'", "quoted.#name"),
            (r#""escaped\u002ename""#, "escaped.name"),
            (r#""quote\"name""#, "quote\"name"),
        ] {
            let (_dir, paths) = temp_paths();
            let original = format!(
                "[mcp_servers.{header}] # server\ncommand = 'old'\nstartup_timeout_sec = 15\n\n\
                 [features]\nweb_search_request = true\n\n\
                 [mcp_servers.{header}.env] # child\nTOKEN = 'synthetic'\n"
            );
            std::fs::write(&paths.config_path, original).unwrap();
            let mut managed = stdio("updated");
            managed.insert("enabled".into(), false.into());
            managed.insert(
                "env".into(),
                toml::Value::Table(toml::Table::from_iter([(
                    "TOKEN".into(),
                    "synthetic".into(),
                )])),
            );
            set_codex_server(&paths, name, &managed).unwrap();
            let updated = std::fs::read_to_string(&paths.config_path).unwrap();
            let parsed: toml::Value = updated.parse().unwrap();
            let servers = parsed["mcp_servers"].as_table().unwrap();
            assert_eq!(servers.len(), 1);
            assert_eq!(servers[name]["command"].as_str(), Some("updated"));
            assert_eq!(servers[name]["enabled"].as_bool(), Some(false));
            assert_eq!(servers[name]["startup_timeout_sec"].as_integer(), Some(15));
            assert_eq!(servers[name]["env"]["TOKEN"].as_str(), Some("synthetic"));
        }
    }

    #[test]
    fn remove_mcp_preserves_adjacent_managed_block_boundaries() {
        let (_dir, paths) = temp_paths();
        let original = format!(
            "{ROUTER_TOP_MANAGED_BLOCK_BEGIN}\nmodel = 'original'\n{ROUTER_TOP_MANAGED_BLOCK_END}\n\n\
             [mcp_servers.foo]\ncommand = 'old'\n\n\
             {BOTTOM_MANAGED_BLOCK_BEGIN}\n[model_providers.relay]\nname = 'Relay'\n{BOTTOM_MANAGED_BLOCK_END}\n\n\
             [mcp_servers.foo.env]\nTOKEN = 'synthetic'\n"
        );
        std::fs::write(&paths.config_path, original).unwrap();
        remove_mcp_server(&paths, "foo").unwrap();
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();
        let parsed: toml::Value = updated.parse().unwrap();
        assert_eq!(parsed["model"].as_str(), Some("original"));
        assert_eq!(
            parsed["model_providers"]["relay"]["name"].as_str(),
            Some("Relay")
        );
        assert!(parsed.get("mcp_servers").is_none());
        for marker in [
            ROUTER_TOP_MANAGED_BLOCK_BEGIN,
            ROUTER_TOP_MANAGED_BLOCK_END,
            BOTTOM_MANAGED_BLOCK_BEGIN,
            BOTTOM_MANAGED_BLOCK_END,
        ] {
            assert_eq!(updated.matches(marker).count(), 1, "{marker}");
        }
    }

    #[test]
    fn mcp_edits_do_not_treat_multiline_values_as_table_headers() {
        let (_dir, paths) = temp_paths();
        let description = "[mcp_servers.ghost]\ncommand = 'not a real server'\n";
        let original = format!(
            "[mcp_servers.foo]\ncommand = 'old'\ndescription = '''\n{description}'''\n\n\
             [features]\nweb_search_request = true\n\n\
             [mcp_servers.foo.env]\nTOKEN = 'synthetic'\n"
        );
        std::fs::write(&paths.config_path, original).unwrap();
        set_codex_server(&paths, "foo", &stdio("old")).unwrap();
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();
        let parsed: toml::Value = updated.parse().unwrap();
        assert_eq!(
            parsed["mcp_servers"]["foo"]["description"].as_str(),
            Some(description)
        );
        assert_eq!(parsed["mcp_servers"].as_table().unwrap().len(), 1);
        remove_mcp_server(&paths, "foo").unwrap();
        let remaining = std::fs::read_to_string(&paths.config_path).unwrap();
        let parsed: toml::Value = remaining.parse().unwrap();
        assert!(parsed.get("mcp_servers").is_none());
        assert_eq!(
            parsed["features"]["web_search_request"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn upsert_mcp_keeps_router_top_block_before_mcp_table() {
        let (_dir, paths) = temp_paths();
        let original = format!(
            "{ROUTER_TOP_MANAGED_BLOCK_BEGIN}\n\
             profile = \"aimai1\"\n\
             model_catalog_json = \"/tmp/catalog.json\"\n\
             {ROUTER_TOP_MANAGED_BLOCK_END}\n\n\
             {BOTTOM_MANAGED_BLOCK_BEGIN}\n\
             [model_providers.aimai1]\n\
             name = \"AgentHarbor 智能路由\"\n\
             {BOTTOM_MANAGED_BLOCK_END}\n"
        );
        std::fs::write(&paths.config_path, original).unwrap();

        set_codex_server(&paths, "context7", &stdio("npx")).unwrap();
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();

        let top_pos = updated.find(ROUTER_TOP_MANAGED_BLOCK_BEGIN).unwrap();
        let mcp_pos = updated.find("[mcp_servers.context7]").unwrap();
        let bottom_pos = updated.find(BOTTOM_MANAGED_BLOCK_BEGIN).unwrap();
        assert!(top_pos < mcp_pos);
        assert!(mcp_pos < bottom_pos);

        let parsed: toml::Value = updated.parse().expect("config must remain valid TOML");
        assert_eq!(parsed["profile"].as_str(), Some("aimai1"));
        assert_eq!(
            parsed["mcp_servers"]["context7"]["command"].as_str(),
            Some("npx")
        );
    }

    #[test]
    fn upsert_mcp_relocates_existing_block_out_of_managed_block() {
        let (_dir, paths) = temp_paths();
        let original = format!(
            "{BOTTOM_MANAGED_BLOCK_BEGIN}\n\
             [model_providers.test_provider]\n\
             name = \"X\"\n\n\
             [mcp_servers.context7]\n\
             command = \"old\"\n\n\
             [profiles.test_provider]\n\
             model_provider = \"test_provider\"\n\
             {BOTTOM_MANAGED_BLOCK_END}\n"
        );
        std::fs::write(&paths.config_path, original).unwrap();

        let mut managed = stdio("npx");
        managed.insert("enabled".into(), false.into());
        set_codex_server(&paths, "context7", &managed).unwrap();
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();

        let mcp_pos = updated.find("[mcp_servers.context7]").unwrap();
        let bottom_pos = updated.find(BOTTOM_MANAGED_BLOCK_BEGIN).unwrap();
        assert!(mcp_pos < bottom_pos);
        assert!(updated.contains("enabled = false"));
        assert!(!updated.contains("command = \"old\""));

        let managed_block = &updated[bottom_pos..];
        assert!(!managed_block.contains("[mcp_servers.context7]"));
    }
}
