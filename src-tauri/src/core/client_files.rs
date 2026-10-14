//! Each client's key files on this machine, for the dashboard's data map.
//! Existence checks only: nothing is read, created or repaired here.

use super::claude_desktop::paths::DesktopPaths;
use super::client_config::ClientPaths;
use crate::platform::paths::CodexPaths;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyFile {
    /// Stable role, so the UI can label files that share a name.
    pub id: &'static str,
    pub name: String,
    pub path: String,
    pub directory: bool,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClientFiles {
    pub client: &'static str,
    pub files: Vec<KeyFile>,
}

fn entry(id: &'static str, path: PathBuf, directory: bool) -> KeyFile {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    KeyFile {
        id,
        name: if directory { format!("{name}/") } else { name },
        exists: if directory { path.is_dir() } else { path.is_file() },
        path: path.display().to_string(),
        directory,
    }
}

fn file(id: &'static str, path: PathBuf) -> KeyFile {
    entry(id, path, false)
}

fn dir(id: &'static str, path: PathBuf) -> KeyFile {
    entry(id, path, true)
}

/// Claude Desktop is listed only when its files can be located; the other
/// clients always list their expected paths, present or not.
pub fn collect(
    codex: &CodexPaths,
    clients: &ClientPaths,
    desktop: Option<&DesktopPaths>,
) -> Vec<ClientFiles> {
    let claude = clients.claude_root();
    let pi = clients.pi.parent().unwrap_or(Path::new(""));
    let mut groups = vec![
        ClientFiles {
            client: "codex",
            files: vec![
                file("config", codex.config_path.clone()),
                file("auth", codex.auth_path.clone()),
                file("registry", codex.registry_path.clone()),
                dir("sessions", codex.sessions_dir.clone()),
                file("instructions", codex.codex_home.join("AGENTS.md")),
            ],
        },
        ClientFiles {
            client: "claudeCode",
            files: vec![
                file("settings", clients.claude.clone()),
                file("userConfig", clients.claude_user_config()),
                dir("sessions", claude.join("projects")),
                file("instructions", claude.join("CLAUDE.md")),
            ],
        },
    ];
    if let Some(desktop) = desktop {
        groups.push(ClientFiles {
            client: "claudeDesktop",
            files: vec![
                file("normalConfig", desktop.normal_config.clone()),
                file("thirdPartyConfig", desktop.third_party_config.clone()),
                file("profile", desktop.profile_path.clone()),
                file("meta", desktop.meta_path.clone()),
            ],
        });
    }
    groups.push(ClientFiles {
        client: "opencode",
        files: vec![file("config", clients.opencode.clone())],
    });
    groups.push(ClientFiles {
        client: "pi",
        files: vec![
            file("models", clients.pi.clone()),
            file("auth", pi.join("auth.json")),
            file("settings", pi.join("settings.json")),
            dir("sessions", pi.join("sessions")),
        ],
    });
    let engine = codex.app_data_dir.join("cursor");
    groups.push(ClientFiles {
        client: "cursor",
        files: vec![
            file("settings", clients.cursor.join("settings.json")),
            file("cliConfig", clients.cursor_cli.join("cli-config.json")),
            file("certificate", engine.join("cursor-ca.crt")),
            file("conversations", engine.join("conversations.db")),
        ],
    });
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn listing(root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path.clone());
                }
                entries.push(path);
            }
        }
        entries.sort();
        entries
    }

    #[test]
    fn every_client_reports_its_key_files_without_touching_them() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let codex = CodexPaths::from_home(home.join(".codex"));
        let clients = ClientPaths::from_home(home);
        let desktop =
            DesktopPaths::from_roots(home.join("desktop"), home.join("desktop-3p")).unwrap();
        fs::create_dir_all(&codex.sessions_dir).unwrap();
        fs::write(&codex.auth_path, "{}").unwrap();
        fs::create_dir_all(home.join(".claude/projects")).unwrap();
        fs::write(&clients.claude, "{}").unwrap();
        fs::create_dir_all(&desktop.normal_root).unwrap();
        fs::write(&desktop.normal_config, "{}").unwrap();
        fs::create_dir_all(clients.pi.parent().unwrap()).unwrap();
        fs::write(&clients.pi, "{}").unwrap();
        // A file where a directory is expected is not that directory.
        fs::write(clients.pi.parent().unwrap().join("sessions"), "").unwrap();
        let before = listing(home);

        let groups = collect(&codex, &clients, Some(&desktop));
        assert_eq!(listing(home), before, "collecting must not create anything");
        let present = |client: &str| {
            groups
                .iter()
                .find(|group| group.client == client)
                .unwrap()
                .files
                .iter()
                .map(|file| (file.id, file.name.as_str(), file.exists))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            groups.iter().map(|group| group.client).collect::<Vec<_>>(),
            ["codex", "claudeCode", "claudeDesktop", "opencode", "pi", "cursor"]
        );
        assert_eq!(
            present("codex"),
            [
                ("config", "config.toml", false),
                ("auth", "auth.json", true),
                ("registry", "registry.json", false),
                ("sessions", "sessions/", true),
                ("instructions", "AGENTS.md", false),
            ]
        );
        assert_eq!(
            present("claudeCode"),
            [
                ("settings", "settings.json", true),
                ("userConfig", ".claude.json", false),
                ("sessions", "projects/", true),
                ("instructions", "CLAUDE.md", false),
            ]
        );
        let desktop_files = present("claudeDesktop");
        assert_eq!(desktop_files[0], ("normalConfig", "claude_desktop_config.json", true));
        assert!(desktop_files[1..].iter().all(|(_, _, exists)| !exists));
        assert_eq!(present("opencode").len(), 1);
        assert_eq!(
            present("pi"),
            [
                ("models", "models.json", true),
                ("auth", "auth.json", false),
                ("settings", "settings.json", false),
                ("sessions", "sessions/", false),
            ]
        );
        assert_eq!(
            present("cursor"),
            [
                ("settings", "settings.json", false),
                ("cliConfig", "cli-config.json", false),
                ("certificate", "cursor-ca.crt", false),
                ("conversations", "conversations.db", false),
            ]
        );
        let paths: Vec<_> = groups
            .iter()
            .flat_map(|group| &group.files)
            .filter(|file| file.exists)
            .map(|file| PathBuf::from(&file.path))
            .collect();
        assert!(paths.iter().all(|path| path.starts_with(home)));

        // Without located Desktop files, that client is simply absent.
        assert!(collect(&codex, &clients, None)
            .iter()
            .all(|group| group.client != "claudeDesktop"));
    }
}
