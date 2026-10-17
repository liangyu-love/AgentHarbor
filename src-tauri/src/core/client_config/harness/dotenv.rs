//! `$DSH_HOME/.env`, where Harness's headless profile looks for keys: it
//! leaves out the credential store and reads the launch environment, then
//! `<cwd>/.env`, then this file (Node's `util.parseEnv`). Only the lines this
//! app names are touched; comments, order and other variables stay as they are.

/// A value written without quotes: nothing Node's parser would trim, end at
/// (`#`) or read as quoting.
fn plain(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-~+/=:@%,".contains(c))
}

fn encode(value: &str) -> Result<String, String> {
    if plain(value) {
        Ok(value.to_owned())
    } else if !value.contains('\'') && !value.contains('\n') && !value.contains('\r') {
        Ok(format!("'{value}'"))
    } else {
        Err("clients.errors.invalidConfig".into())
    }
}

/// The variable a line assigns, and where its value starts.
fn assignment(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").map(str::trim_start).unwrap_or(line);
    let (name, value) = line.split_once('=')?;
    let name = name.trim_end();
    (!name.is_empty() && !name.starts_with('#')).then_some((name, value))
}

fn decode(raw: &str) -> String {
    let raw = raw.trim();
    for quote in ['\'', '"', '`'] {
        if let Some(inner) = raw.strip_prefix(quote).and_then(|rest| rest.find(quote).map(|end| &rest[..end])) {
            return inner.to_owned();
        }
    }
    raw.split(" #").next().unwrap_or(raw).trim().to_owned()
}

/// The value the file gives `name`; a later line wins, as in Node.
pub(super) fn get(text: Option<&str>, name: &str) -> Option<String> {
    text?.lines().filter_map(assignment).filter(|(key, _)| *key == name).last().map(|(_, value)| decode(value))
}

/// The file with `name` set to `value`, or removed; `None` when nothing is left.
pub(super) fn set(text: Option<&str>, name: &str, value: Option<&str>) -> Result<Option<String>, String> {
    let line = value.map(|value| encode(value).map(|value| format!("{name}={value}"))).transpose()?;
    let text = text.unwrap_or("");
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = Vec::new();
    let mut placed = false;
    for existing in text.lines() {
        if assignment(existing).is_some_and(|(key, _)| key == name) {
            if let Some(line) = line.as_ref().filter(|_| !placed) {
                lines.push(line.clone());
            }
            placed = true;
        } else {
            lines.push(existing.to_owned());
        }
    }
    if let Some(line) = line.filter(|_| !placed) {
        lines.push(line);
    }
    if lines.iter().all(|line| line.trim().is_empty()) {
        return Ok(None);
    }
    let mut result = lines.join(newline);
    result.push_str(newline);
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_only_the_named_variable_and_keeps_the_rest() {
        let text = "# keys\r\nDEEPSEEK_API_KEY=sk-user\r\nexport AGENTHARBOR_X='old'\r\n";
        let next = set(Some(text), "AGENTHARBOR_X", Some("sk-new")).unwrap().unwrap();
        assert_eq!(next, "# keys\r\nDEEPSEEK_API_KEY=sk-user\r\nAGENTHARBOR_X=sk-new\r\n");
        assert_eq!(get(Some(&next), "AGENTHARBOR_X").as_deref(), Some("sk-new"));
        assert_eq!(get(Some(&next), "DEEPSEEK_API_KEY").as_deref(), Some("sk-user"));
        let removed = set(Some(&next), "AGENTHARBOR_X", None).unwrap().unwrap();
        assert_eq!(removed, "# keys\r\nDEEPSEEK_API_KEY=sk-user\r\n");
    }

    #[test]
    fn a_new_file_holds_just_the_variables_and_goes_once_empty() {
        let created = set(None, "AGENTHARBOR_ROUTER_TOKEN", Some("token 1#")).unwrap().unwrap();
        assert_eq!(created, "AGENTHARBOR_ROUTER_TOKEN='token 1#'\n");
        assert_eq!(get(Some(&created), "AGENTHARBOR_ROUTER_TOKEN").as_deref(), Some("token 1#"));
        assert_eq!(set(Some(&created), "AGENTHARBOR_ROUTER_TOKEN", None).unwrap(), None);
        assert!(set(None, "AGENTHARBOR_X", Some("it's\nmulti")).is_err());
    }
}
