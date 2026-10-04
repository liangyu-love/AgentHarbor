//! Flatten Responses function namespaces for targets without namespace support.
//! Restore identity only from this request's map; arguments are never traversed.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Default, Debug)]
pub struct ToolNamespaces {
    names: HashMap<String, (String, String)>,
}

impl ToolNamespaces {
    fn encode(&mut self, namespace: &str, name: &str) -> Result<String, String> {
        if namespace.is_empty()
            || name.is_empty()
            || namespace.len() > 256
            || name.len() > 256
            || namespace.chars().chain(name.chars()).any(char::is_control)
        {
            return Err("protocol.errors.unsupportedToolDefinition".into());
        }
        let mut hash = Sha256::new();
        for field in [namespace, name] {
            hash.update((field.len() as u64).to_le_bytes());
            hash.update(field.as_bytes());
        }
        let encoded = format!("ahns_{:x}", hash.finalize());
        let encoded = encoded[..61].to_owned();
        self.names
            .insert(encoded.clone(), (namespace.into(), name.into()));
        Ok(encoded)
    }

    pub(super) fn prepare(value: &mut Value) -> Result<Self, String> {
        let mut map = Self::default();
        let mut plain_names = HashSet::new();
        if let Some(tools) = value.get_mut("tools") {
            let original = tools.as_array().ok_or("protocol.errors.invalidRequest")?;
            let mut flattened = Vec::new();
            let mut declared = HashSet::new();
            for tool in original {
                if tool["type"] != "namespace" {
                    if let Some(name) = tool["name"].as_str() {
                        plain_names.insert(name.to_owned());
                    }
                    flattened.push(tool.clone());
                    continue;
                }
                let namespace = tool["name"]
                    .as_str()
                    .ok_or("protocol.errors.unsupportedToolDefinition")?;
                let children = tool["tools"]
                    .as_array()
                    .ok_or("protocol.errors.unsupportedToolDefinition")?;
                for child in children {
                    if child["type"] != "function" {
                        return Err("protocol.errors.unsupportedNamespacedTool".into());
                    }
                    let name = child["name"]
                        .as_str()
                        .ok_or("protocol.errors.unsupportedToolDefinition")?;
                    if !declared.insert((namespace.to_owned(), name.to_owned())) {
                        return Err("protocol.errors.unsupportedToolDefinition".into());
                    }
                    let encoded = map.encode(namespace, name)?;
                    let mut function = child.clone();
                    function["name"] = json!(encoded);
                    // Preserve the group's guidance alongside the function's description.
                    if let Some(group) = tool.get("description").and_then(Value::as_str) {
                        if !group.is_empty() {
                            let description = child
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            function["description"] = json!(format!("{group}\n{description}"));
                        }
                    }
                    flattened.push(function);
                }
            }
            *tools = Value::Array(flattened);
        }
        if let Some(items) = value.get_mut("input").and_then(Value::as_array_mut) {
            for item in items {
                if item["type"] == "function_call" {
                    if item.get("namespace").is_none_or(Value::is_null) {
                        if let Some(name) = item["name"].as_str() {
                            plain_names.insert(name.to_owned());
                        }
                    }
                    map.rename_call(item)?;
                }
            }
        }
        if let Some(choice) = value.get_mut("tool_choice").filter(|v| v.is_object()) {
            if choice.get("namespace").is_none_or(Value::is_null) {
                if let Some(name) = choice["name"].as_str() {
                    plain_names.insert(name.to_owned());
                }
            }
            map.rename_call(choice)?;
        }
        if map
            .names
            .keys()
            .any(|encoded| plain_names.contains(encoded))
        {
            return Err("protocol.errors.unsupportedToolDefinition".into());
        }
        Ok(map)
    }

    fn rename_call(&mut self, item: &mut Value) -> Result<(), String> {
        let Some(namespace) = item.get("namespace").filter(|v| !v.is_null()) else {
            return Ok(());
        };
        let namespace = namespace
            .as_str()
            .ok_or("protocol.errors.unsupportedToolDefinition")?;
        let name = item["name"]
            .as_str()
            .ok_or("protocol.errors.unsupportedToolDefinition")?;
        let encoded = self.encode(namespace, name)?;
        item["name"] = json!(encoded);
        item.as_object_mut().unwrap().remove("namespace");
        Ok(())
    }

    fn restore_item(&self, item: &mut Value) {
        if item["type"] != "function_call" {
            return;
        }
        if let Some((namespace, name)) = item["name"].as_str().and_then(|name| self.names.get(name))
        {
            let (namespace, name) = (namespace.clone(), name.clone());
            item["name"] = json!(name);
            item["namespace"] = json!(namespace);
        }
    }

    pub fn restore_response(&self, value: &mut Value) {
        if let Some(items) = value.get_mut("output").and_then(Value::as_array_mut) {
            for item in items {
                self.restore_item(item);
            }
        }
    }

    pub fn restore_event(&self, event: String) -> String {
        if self.names.is_empty() {
            return event;
        }
        let Some((prefix, data)) = event.split_once("data: ") else {
            return event;
        };
        let Ok(mut value) = serde_json::from_str::<Value>(data.trim()) else {
            return event;
        };
        if let Some(item) = value.get_mut("item") {
            self.restore_item(item);
        }
        if let Some(response) = value.get_mut("response") {
            self.restore_response(response);
        }
        format!("{prefix}data: {value}\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaces_keep_definitions_history_choice_and_response_identity() {
        let mut request = json!({"tools":[
            {"type":"function","name":"lookup","parameters":{"type":"object"}},
            {"type":"namespace","name":"mcp_server","description":"Group guidance","tools":[
                {"type":"function","name":"lookup","description":"Lookup","strict":true,"parameters":{"type":"object"}}
            ]}],"input":[{"type":"function_call","namespace":"mcp_server","name":"lookup","call_id":"c","arguments":"{}"}],
            "tool_choice":{"type":"function","namespace":"mcp_server","name":"lookup"}});
        let map = ToolNamespaces::prepare(&mut request).unwrap();
        let encoded = request["tools"][1]["name"].clone();
        assert_ne!(encoded, "lookup");
        assert_eq!(request["tools"][1]["strict"], true);
        assert_eq!(request["tools"][1]["description"], "Group guidance\nLookup");
        assert_eq!(request["input"][0]["name"], encoded);
        assert_eq!(request["tool_choice"]["name"], encoded);
        let item = json!({"type":"function_call","name":encoded,"call_id":"c","arguments":"{\"name\":\"unchanged\"}"});
        let mut response = json!({"output":[item.clone()]});
        map.restore_response(&mut response);
        assert_eq!(response["output"][0]["namespace"], "mcp_server");
        assert_eq!(response["output"][0]["name"], "lookup");
        assert_eq!(response["output"][0]["arguments"], item["arguments"]);
        let event = map.restore_event(format!(
            "event: response.output_item.done\ndata: {}\n\n",
            json!({"type":"response.output_item.done","item":item})
        ));
        let value: Value =
            serde_json::from_str(event.split_once("data: ").unwrap().1.trim()).unwrap();
        assert_eq!(value["item"], response["output"][0]);
        let mut followup = json!({"input":response["output"]});
        ToolNamespaces::prepare(&mut followup).unwrap();
        assert_eq!(followup["input"][0]["name"], encoded);
    }

    #[test]
    fn namespaces_reject_unsupported_children_and_plain_identity_collisions() {
        for child in [
            json!({"type":"custom","name":"patch"}),
            json!({"type":"namespace","name":"nested","tools":[]}),
        ] {
            let mut request =
                json!({"tools":[{"type":"namespace","name":"group","tools":[child]}]});
            assert_eq!(
                ToolNamespaces::prepare(&mut request).unwrap_err(),
                "protocol.errors.unsupportedNamespacedTool"
            );
        }
        let mut map = ToolNamespaces::default();
        let encoded = map.encode("group", "lookup").unwrap();
        let namespace = json!({"type":"namespace","name":"group","tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]});
        for mut request in [
            json!({"tools":[namespace, {"type":"function","name":encoded,"parameters":{"type":"object"}}]}),
            json!({"tools":[namespace],"input":[{"type":"function_call","name":encoded,"call_id":"c","arguments":"{}"}]}),
            json!({"tools":[namespace],"tool_choice":{"type":"function","name":encoded}}),
        ] {
            assert_eq!(
                ToolNamespaces::prepare(&mut request).unwrap_err(),
                "protocol.errors.unsupportedToolDefinition"
            );
        }
    }
}
