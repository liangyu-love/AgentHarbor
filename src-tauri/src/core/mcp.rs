use crate::core::codex_config::CodexConfigRepository;
use crate::core::models::*;
use crate::platform::paths::CodexPaths;
use std::collections::HashMap;

mod claude;

pub enum McpConfig<'a> {
    Codex(&'a CodexPaths),
    Claude(std::path::PathBuf),
}

impl<'a> McpConfig<'a> {
    pub fn for_target(
        paths: &'a CodexPaths,
        target: super::targets::TargetId,
    ) -> Result<Self, CoreError> {
        match target {
            super::targets::TargetId::Codex => Ok(Self::Codex(paths)),
            super::targets::TargetId::ClaudeCode => Ok(Self::Claude(
                super::client_config::ClientPaths::discover().claude_user_config(),
            )),
            _ => Err(CoreError::InvalidData("Unsupported MCP target".into())),
        }
    }

    pub fn source_path(&self) -> String {
        match self {
            Self::Codex(paths) => paths.config_path.display().to_string(),
            Self::Claude(path) => path.display().to_string(),
        }
    }

    pub fn load(&self) -> Result<Vec<McpServerSummary>, CoreError> {
        match self {
            Self::Codex(paths) => load_mcp_servers(paths),
            Self::Claude(path) => claude::load(path),
        }
    }

    pub fn upsert(&self, server: &McpServerSummary) -> Result<McpServerSummary, CoreError> {
        match self {
            Self::Codex(paths) => upsert_mcp_server(paths, server),
            Self::Claude(path) => claude::upsert(path, server),
        }
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<McpServerSummary, CoreError> {
        match self {
            Self::Codex(paths) => set_mcp_server_enabled(paths, name, enabled),
            Self::Claude(path) => claude::set_enabled(path, name, enabled),
        }
    }

    pub fn remove(&self, name: &str) -> Result<(), CoreError> {
        match self {
            Self::Codex(paths) => remove_mcp_server(paths, name),
            Self::Claude(path) => claude::remove(path, name),
        }
    }
}

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

pub fn load_mcp_servers(paths: &CodexPaths) -> Result<Vec<McpServerSummary>, CoreError> {
    let text = CodexConfigRepository::new(paths).read_text()?;
    if text.is_empty() {
        return Ok(vec![]);
    }
    parse_mcp_servers(&text, &paths.config_path.display().to_string())
}

pub fn upsert_mcp_server(
    paths: &CodexPaths,
    server: &McpServerSummary,
) -> Result<McpServerSummary, CoreError> {
    write_mcp_server(paths, server)?;
    let servers = load_mcp_servers(paths)?;
    Ok(servers
        .into_iter()
        .find(|s| s.name == server.name)
        .unwrap_or_else(|| server.clone()))
}

pub fn set_mcp_server_enabled(
    paths: &CodexPaths,
    name: &str,
    enabled: bool,
) -> Result<McpServerSummary, CoreError> {
    let source_path = paths.config_path.display().to_string();
    let mut saved = None;
    CodexConfigRepository::new(paths).update(|original| {
        let current = parse_mcp_servers(original, &source_path)?
            .into_iter()
            .find(|server| server.name == name)
            .ok_or_else(|| CoreError::NotFound(format!("MCP server not found: {name}")))?;
        let mut updated = current;
        updated.enabled = enabled;
        let next = render_mcp_update(original, &updated)?;
        saved = Some(updated);
        Ok(next)
    })?;
    saved.ok_or_else(|| CoreError::OperationFailed("MCP server update produced no result".into()))
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

fn write_mcp_server(paths: &CodexPaths, server: &McpServerSummary) -> Result<(), CoreError> {
    CodexConfigRepository::new(paths).update(|original| render_mcp_update(original, server))?;
    Ok(())
}

fn render_mcp_update(original: &str, server: &McpServerSummary) -> Result<String, CoreError> {
    if server.name.trim().is_empty() {
        return Err(CoreError::InvalidData("MCP server name is required".into()));
    }
    if server.name.trim() != server.name {
        return Err(CoreError::InvalidData(
            "MCP server name must not start or end with whitespace".into(),
        ));
    }
    match server.transport {
        McpTransport::Stdio
            if server
                .command
                .as_deref()
                .map_or(true, |s| s.trim().is_empty()) =>
        {
            return Err(CoreError::InvalidData("MCP command is required".into()))
        }
        McpTransport::Http | McpTransport::Sse => {
            let url = server
                .url
                .as_deref()
                .and_then(|s| reqwest::Url::parse(s).ok())
                .ok_or_else(|| CoreError::InvalidData("MCP URL is required".into()))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(CoreError::InvalidData(
                    "MCP URL must use HTTP or HTTPS".into(),
                ));
            }
        }
        McpTransport::Unknown => {
            return Err(CoreError::InvalidData("Unsupported MCP transport".into()))
        }
        _ => {}
    }
    let original_value: toml::Value = original.parse()?;
    let document = parse_mcp_document(original)?;
    let managed: toml::Value = render_mcp_block(server).join("\n").parse()?;
    let mut preserved = original_value
        .get("mcp_servers")
        .and_then(|v| v.get(&server.name))
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    if !preserved.is_empty() && !document.blocks.contains_key(&server.name) {
        return Err(CoreError::InvalidData(
            "Expand inline MCP configuration into a table before editing".into(),
        ));
    }
    for key in [
        "enabled",
        "transport",
        "type",
        "command",
        "args",
        "url",
        "env",
        "headers",
        "http_headers",
    ] {
        preserved.remove(key);
    }
    preserved.extend(
        managed["mcp_servers"][&server.name]
            .as_table()
            .unwrap()
            .clone(),
    );
    let mut servers = toml::Table::new();
    servers.insert(server.name.clone(), toml::Value::Table(preserved));
    let mut wrapper = toml::Table::new();
    wrapper.insert("mcp_servers".into(), toml::Value::Table(servers));
    let rendered = toml::to_string_pretty(&wrapper)?
        .lines()
        .map(str::to_owned)
        .collect();

    let updated = if let Some(existing) = document.blocks.get(&server.name) {
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

    Ok(updated.join("\n"))
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

fn render_mcp_block(server: &McpServerSummary) -> Vec<String> {
    let header = quote_toml(&server.name);
    let mut lines = vec![format!("[mcp_servers.{header}]")];
    lines.push(format!(
        "enabled = {}",
        if server.enabled { "true" } else { "false" }
    ));
    let transport_str = match server.transport {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
        McpTransport::Sse => "sse",
        McpTransport::Unknown => "stdio",
    };
    lines.push(format!("transport = {}", quote_toml(transport_str)));

    match server.transport {
        McpTransport::Stdio | McpTransport::Unknown => {
            if let Some(ref cmd) = server.command {
                if !cmd.trim().is_empty() {
                    lines.push(format!("command = {}", quote_toml(cmd)));
                }
            }
            if !server.args.is_empty() {
                let args_str = server
                    .args
                    .iter()
                    .map(|a| quote_toml(a))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("args = [{args_str}]"));
            }
        }
        McpTransport::Http | McpTransport::Sse => {
            if let Some(ref url) = server.url {
                if !url.trim().is_empty() {
                    lines.push(format!("url = {}", quote_toml(url)));
                }
            }
        }
    }

    if !server.environment.is_empty() {
        lines.push(String::new());
        lines.push(format!("[mcp_servers.{header}.env]"));
        let mut keys: Vec<&String> = server.environment.keys().collect();
        keys.sort();
        for key in keys {
            let value = server
                .environment
                .get(key)
                .map(|s| s.as_str())
                .unwrap_or("");
            lines.push(format!("{} = {}", quote_toml(key), quote_toml(value)));
        }
    }

    if !server.headers.is_empty() {
        lines.push(String::new());
        lines.push(format!("[mcp_servers.{header}.http_headers]"));
        let mut keys: Vec<&String> = server.headers.keys().collect();
        keys.sort();
        for key in keys {
            let value = server.headers.get(key).map(|s| s.as_str()).unwrap_or("");
            lines.push(format!("{} = {}", quote_toml(key), quote_toml(value)));
        }
    }

    lines
}

fn quote_toml(s: &str) -> String {
    toml::Value::String(s.into()).to_string()
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

fn parse_mcp_servers(text: &str, source_path: &str) -> Result<Vec<McpServerSummary>, CoreError> {
    let document: toml::Value = text.parse()?;
    let Some(servers) = document.get("mcp_servers").and_then(toml::Value::as_table) else {
        return Ok(vec![]);
    };
    let string_map = |value: Option<&toml::Value>| -> HashMap<String, String> {
        value
            .and_then(toml::Value::as_table)
            .map(|table| {
                table
                    .iter()
                    .filter_map(|(key, value)| value.as_str().map(|s| (key.clone(), s.to_owned())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut items = Vec::new();
    for (name, value) in servers {
        let command = value
            .get("command")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        let url = value
            .get("url")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        let transport = match value
            .get("transport")
            .or_else(|| value.get("type"))
            .and_then(toml::Value::as_str)
        {
            Some("sse") => McpTransport::Sse,
            _ if command.is_some() => McpTransport::Stdio,
            _ if url.is_some() => McpTransport::Http,
            _ => McpTransport::Unknown,
        };
        items.push(McpServerSummary {
            name: name.clone(),
            transport,
            command,
            url,
            enabled: value
                .get("enabled")
                .and_then(toml::Value::as_bool)
                .unwrap_or(true),
            source_path: source_path.into(),
            args: value
                .get("args")
                .and_then(toml::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            environment: string_map(value.get("env")),
            headers: string_map(value.get("http_headers").or_else(|| value.get("headers"))),
        });
    }
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTTOM_MANAGED_BLOCK_END: &str = "# --- End AgentHarbor Managed Block (bottom) ---";
    const ROUTER_TOP_MANAGED_BLOCK_END: &str =
        "# --- End AgentHarbor Managed Block (router-top) ---";

    fn stdio_server(name: &str) -> McpServerSummary {
        McpServerSummary {
            name: name.into(),
            transport: McpTransport::Stdio,
            enabled: true,
            source_path: String::new(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "@upstash/context7-mcp@latest".into()],
            url: None,
            headers: HashMap::new(),
            environment: HashMap::new(),
        }
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
        let remaining = load_mcp_servers(&paths).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "keep");
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
        assert!(load_mcp_servers(&paths).unwrap().is_empty());
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
        assert!(load_mcp_servers(&paths).unwrap().is_empty());
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
        let mut server = set_mcp_server_enabled(&paths, "foo", false).unwrap();
        assert!(!server.enabled);
        server.command = Some("updated".into());
        upsert_mcp_server(&paths, &server).unwrap();
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
            let mut server = set_mcp_server_enabled(&paths, name, false).unwrap();
            server.command = Some("updated".into());
            upsert_mcp_server(&paths, &server).unwrap();
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
        set_mcp_server_enabled(&paths, "foo", false).unwrap();
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

        upsert_mcp_server(&paths, &stdio_server("context7")).unwrap();
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

        let mut server = stdio_server("context7");
        server.enabled = false;
        upsert_mcp_server(&paths, &server).unwrap();
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
