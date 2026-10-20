//! Servers pasted from documentation or another agent's file: JSON with
//! `mcpServers` (Claude, Cursor, Claude Desktop, Pi), `servers` (VS Code) or
//! `mcp` (OpenCode), a bare map of servers or one unnamed server, Codex's
//! `[mcp_servers.<name>]` TOML, or a command line (`npx -y …`, or
//! `claude mcp add <name> -- …`).
use super::apps::{McpKind, McpSpec};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const MAX_PASTE_CHARS: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PastedServer {
    /// Empty when the paste did not name it.
    pub name: String,
    pub spec: McpSpec,
}

pub fn parse_pasted(text: &str) -> Result<Vec<PastedServer>, String> {
    let text = text.trim().trim_start_matches('\u{feff}');
    if text.is_empty() || text.len() > MAX_PASTE_CHARS {
        return Err("mcp.errors.paste".into());
    }
    let servers = if text.starts_with('{') {
        let value: Value = serde_json::from_str(text)
            .or_else(|_| json5::from_str(text))
            .map_err(|_| "mcp.errors.paste")?;
        from_json(&value)
    } else if text.starts_with('[') {
        let value: toml::Value = text.parse().map_err(|_| "mcp.errors.paste")?;
        from_json(&serde_json::to_value(value).map_err(|_| "mcp.errors.paste")?)
    } else {
        from_command_line(text).into_iter().collect()
    };
    if servers.is_empty() {
        return Err("mcp.errors.paste".into());
    }
    Ok(servers)
}

fn from_json(value: &Value) -> Vec<PastedServer> {
    for key in ["mcpServers", "servers", "mcp", "mcp_servers"] {
        if let Some(map) = value.get(key).and_then(Value::as_object) {
            return named(map);
        }
    }
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    // One server without a name, or a map of named servers.
    if let Some(spec) = entry(object) {
        return vec![PastedServer {
            name: String::new(),
            spec,
        }];
    }
    named(object)
}

fn named(map: &Map<String, Value>) -> Vec<PastedServer> {
    map.iter()
        .filter_map(|(name, value)| {
            Some(PastedServer {
                name: name.clone(),
                spec: entry(value.as_object()?)?,
            })
        })
        .collect()
}

fn strings(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect()
}

/// One server in any of the agents' shapes.
fn entry(object: &Map<String, Value>) -> Option<McpSpec> {
    let kind = object
        .get("type")
        .or_else(|| object.get("transport"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(url) = object
        .get("url")
        .or_else(|| object.get("serverUrl"))
        .and_then(Value::as_str)
    {
        return Some(McpSpec {
            transport: if kind == "sse" {
                McpKind::Sse
            } else {
                McpKind::Http
            },
            url: Some(url.to_owned()),
            headers: strings(object.get("headers").or_else(|| object.get("http_headers"))),
            ..Default::default()
        });
    }
    // OpenCode keeps the command line in one array.
    let mut line: Vec<String> = match object.get("command")? {
        Value::String(command) => vec![command.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => return None,
    };
    line.extend(
        object
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|arg| arg.as_str().map(str::to_owned)),
    );
    let (command, args) = unwrap_cmd(line)?;
    Some(McpSpec {
        transport: McpKind::Stdio,
        command: Some(command),
        args,
        env: strings(object.get("env").or_else(|| object.get("environment"))),
        cwd: object.get("cwd").and_then(Value::as_str).map(str::to_owned),
        ..Default::default()
    })
}

/// `cmd /c npx …` is `npx …`: the wrapper belongs to Windows, not the server.
fn unwrap_cmd(mut line: Vec<String>) -> Option<(String, Vec<String>)> {
    if line.len() >= 3 && line[0].eq_ignore_ascii_case("cmd") && line[1].eq_ignore_ascii_case("/c")
    {
        line.drain(..2);
    }
    let mut line = line.into_iter();
    let command = line.next().filter(|command| !command.trim().is_empty())?;
    Some((command, line.collect()))
}

/// A command line, or a `claude mcp add` / `codex mcp add` invocation.
fn from_command_line(text: &str) -> Option<PastedServer> {
    let words = split_words(text.lines().next()?.trim())?;
    let mut name = String::new();
    let mut env = BTreeMap::new();
    let line: Vec<String> = match words.iter().position(|word| word == "add") {
        Some(add) if add >= 2 && words[add - 1] == "mcp" => {
            // `<agent> mcp add [options] <name> [--] <command…>`, or `<name> <url>`.
            let mut rest = words[add + 1..].iter();
            let mut tail = Vec::new();
            let mut transport = None;
            while let Some(word) = rest.next() {
                match word.as_str() {
                    "--" => {
                        tail.extend(rest.by_ref().cloned());
                        break;
                    }
                    "-e" | "--env" => {
                        if let Some((key, value)) =
                            rest.next().and_then(|pair| pair.split_once('='))
                        {
                            env.insert(key.to_owned(), value.to_owned());
                        }
                    }
                    "-t" | "--transport" => transport = rest.next().cloned(),
                    "-s" | "--scope" => {
                        rest.next();
                    }
                    word if word.starts_with('-') => {}
                    word if name.is_empty() => name = word.to_owned(),
                    word => tail.push(word.to_owned()),
                }
            }
            if tail.len() == 1 && tail[0].starts_with("http") {
                return Some(PastedServer {
                    name,
                    spec: McpSpec {
                        transport: if transport.as_deref() == Some("sse") {
                            McpKind::Sse
                        } else {
                            McpKind::Http
                        },
                        url: Some(tail.remove(0)),
                        ..Default::default()
                    },
                });
            }
            tail
        }
        _ => words,
    };
    if line.len() == 1 && line[0].starts_with("http") {
        return Some(PastedServer {
            name,
            spec: McpSpec {
                transport: McpKind::Http,
                url: line.into_iter().next(),
                ..Default::default()
            },
        });
    }
    let (command, args) = unwrap_cmd(line)?;
    Some(PastedServer {
        name,
        spec: McpSpec {
            transport: McpKind::Stdio,
            command: Some(command),
            args,
            env,
            ..Default::default()
        },
    })
}

/// Shell-like words: whitespace separates, quotes group, `\` escapes in double quotes.
fn split_words(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (Some(open), c) if c == open => quote = None,
            (Some('"'), '\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                word.push(chars.next()?);
            }
            (_, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        words.push(word);
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> PastedServer {
        let mut servers = parse_pasted(text).unwrap();
        assert_eq!(servers.len(), 1, "{servers:?}");
        servers.remove(0)
    }

    #[test]
    fn every_agent_shape_reads() {
        let claude = one(
            r#"{"mcpServers":{"fetch":{"type":"stdio","command":"cmd","args":["/c","npx","-y","pkg"],"env":{"K":"v"}}}}"#,
        );
        assert_eq!(claude.name, "fetch");
        assert_eq!(claude.spec.command.as_deref(), Some("npx"));
        assert_eq!(claude.spec.args, ["-y", "pkg"]);
        assert_eq!(claude.spec.env["K"], "v");

        let opencode = one(
            r#"{"mcp":{"docs":{"type":"remote","url":"https://d.invalid/mcp","headers":{"A":"b"}}}}"#,
        );
        assert_eq!(opencode.spec.transport, McpKind::Http);
        assert_eq!(opencode.spec.headers["A"], "b");
        let local = one(
            r#"{"mcp":{"x":{"type":"local","command":["node","s.js"],"environment":{"K":"v"}}}}"#,
        );
        assert_eq!(
            (local.spec.command.as_deref(), local.spec.args.as_slice()),
            (Some("node"), &["s.js".to_owned()][..])
        );
        assert_eq!(local.spec.env["K"], "v");

        let vscode = one(r#"{"servers":{"gh":{"type":"sse","url":"https://g.invalid/sse"}}}"#);
        assert_eq!(vscode.spec.transport, McpKind::Sse);

        let codex = one("[mcp_servers.ctx]\ncommand = \"npx\"\nargs = [\"-y\", \"@upstash/context7-mcp\"]\ncwd = \"D:/w\"\n");
        assert_eq!(codex.name, "ctx");
        assert_eq!(codex.spec.cwd.as_deref(), Some("D:/w"));

        let unnamed = one(r#"{"command":"uvx","args":["mcp-server-fetch"]}"#);
        assert_eq!(unnamed.name, "");
        assert_eq!(unnamed.spec.command.as_deref(), Some("uvx"));

        let many =
            parse_pasted(r#"{"a":{"command":"x"},"b":{"url":"https://b.invalid"}}"#).unwrap();
        assert_eq!(many.len(), 2);
    }

    #[test]
    fn command_lines_read() {
        let plain = one("npx -y @modelcontextprotocol/server-filesystem \"C:/My Files\"");
        assert_eq!(plain.spec.command.as_deref(), Some("npx"));
        assert_eq!(
            plain.spec.args,
            [
                "-y",
                "@modelcontextprotocol/server-filesystem",
                "C:/My Files"
            ]
        );

        let added = one("claude mcp add context7 -s user -e KEY=v -- npx -y @upstash/context7-mcp");
        assert_eq!(added.name, "context7");
        assert_eq!(added.spec.args, ["-y", "@upstash/context7-mcp"]);
        assert_eq!(added.spec.env["KEY"], "v");

        let remote = one("claude mcp add --transport sse linear https://mcp.linear.app/sse");
        assert_eq!(remote.name, "linear");
        assert_eq!(remote.spec.transport, McpKind::Sse);
        assert_eq!(
            remote.spec.url.as_deref(),
            Some("https://mcp.linear.app/sse")
        );
    }

    #[test]
    fn nothing_usable_is_refused() {
        for text in ["", "{", "{\"theme\":\"dark\"}", "\"unterminated", "[broken"] {
            assert_eq!(
                parse_pasted(text).unwrap_err(),
                "mcp.errors.paste",
                "{text}"
            );
        }
    }
}
