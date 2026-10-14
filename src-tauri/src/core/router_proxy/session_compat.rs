//! Request compatibility without inventing or discarding conversation history.

use super::{Bytes, ForwardResult, StatusCode, Value};

/// Apply known capability levels after a Messages request has been translated.
/// A missing declaration leaves intent intact; an explicit empty list removes
/// the effort parameter. Keep raw conversation IDs and all message content.
pub(super) fn prepare_message_translation(
    body: Option<Bytes>,
    protocol: crate::core::router::ModelProtocol,
    metadata: &crate::core::model_metadata::ResolvedMetadata,
) -> ForwardResult<Option<Bytes>> {
    let Some(bytes) = &body else { return Ok(body) };
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(body);
    };
    let is_chat = protocol == crate::core::router::ModelProtocol::ChatCompletions;
    let effort = if is_chat {
        value.get("reasoning_effort")
    } else {
        value.pointer("/reasoning/effort")
    }
    .and_then(Value::as_str);
    let Some(effort) = effort else {
        return Ok(body);
    };
    let levels = metadata.reasoning.levels.value.as_ref();
    if metadata.reasoning.supports != Some(false) && levels.is_none() {
        return Ok(body);
    }
    let supported = |candidate: &str| {
        metadata.reasoning.supports != Some(false)
            && levels.is_some_and(|levels| levels.iter().any(|level| level.effort == candidate))
    };
    if supported(effort) {
        return Ok(body);
    }
    let ranked = [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ];
    let replacement = ranked
        .iter()
        .position(|candidate| *candidate == effort)
        .and_then(|rank| {
            ranked[..=rank]
                .iter()
                .rev()
                .find(|candidate| supported(candidate))
                .copied()
        });
    let object = value
        .as_object_mut()
        .expect("translated request is an object");
    if is_chat {
        if let Some(effort) = replacement {
            object.insert("reasoning_effort".into(), serde_json::json!(effort));
        } else {
            object.remove("reasoning_effort");
        }
    } else if let Some(reasoning) = object.get_mut("reasoning").and_then(Value::as_object_mut) {
        if let Some(effort) = replacement {
            reasoning.insert("effort".into(), serde_json::json!(effort));
        } else {
            reasoning.remove("effort");
        }
        if reasoning.is_empty() {
            object.remove("reasoning");
        }
    }
    serde_json::to_vec(&value)
        .map(|bytes| Some(Bytes::from(bytes)))
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot re-encode request",
            )
        })
}

/// How a Chat upstream switches a model's thinking on or off: OpenRouter and
/// DashScope by their own fields, everyone else (MiMo, GLM, DeepSeek, Kimi)
/// by `thinking.type`.
fn thinking_switch(base_url: &str, on: bool) -> (&'static str, Value) {
    let host = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if host == "openrouter.ai" || host.ends_with(".openrouter.ai") {
        ("reasoning", serde_json::json!({"enabled": on}))
    } else if host.starts_with("dashscope") {
        ("enable_thinking", Value::Bool(on))
    } else {
        ("thinking", serde_json::json!({"type": if on { "enabled" } else { "disabled" }}))
    }
}

/// A Chat request's effort for a model whose thinking is a switch. `none`
/// turns it off unless the model declares `none` as a level of its own; a
/// model with the switch and no levels takes any other effort as on. Declared
/// levels and manual capabilities pass through untouched.
pub(super) fn prepare_thinking_switch(
    body: Option<Bytes>,
    metadata: impl FnOnce() -> crate::core::model_metadata::ResolvedMetadata,
    base_url: &str,
) -> ForwardResult<Option<Bytes>> {
    let Some(bytes) = &body else { return Ok(body) };
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(body);
    };
    let Some(effort) = value.get("reasoning_effort").and_then(Value::as_str).map(str::to_owned) else {
        return Ok(body);
    };
    let metadata = metadata();
    let switch = metadata.reasoning.supports != Some(false)
        && metadata.reasoning.options.iter().any(|option| option.kind == "toggle");
    if !switch || metadata.reasoning.levels.source == crate::core::model_metadata::source::MANUAL {
        return Ok(body);
    }
    let levels = metadata.reasoning.levels.value.unwrap_or_default();
    let on = effort != "none";
    if levels.iter().any(|level| level.effort == effort) || (on && !levels.is_empty()) {
        return Ok(body);
    }
    let object = value.as_object_mut().expect("request object");
    object.remove("reasoning_effort");
    let (field, setting) = thinking_switch(base_url, on);
    object.insert(field.into(), setting);
    serde_json::to_vec(&value)
        .map(|bytes| Some(Bytes::from(bytes)))
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot re-encode request",
            )
        })
}

/// A model switch can leave the previous model's effort (notably `none`) in
/// the request. Remove only an effort excluded by a known, nonempty catalog
/// list. Unknown/empty capabilities are not evidence to change the request.
/// Keep summaries and every other reasoning/history field intact.
///
/// `ultra` is Codex's client alias, never a wire value, even when a catalog
/// lists it for the picker. It resolves the way Codex resolves it: the model's
/// `multi_agent_reasoning_effort`, else `max`, else the highest other level.
///
/// `entry` resolves only the requested slug, so this never builds the whole
/// menu on the request path.
pub(super) fn prepare_reasoning(
    body: Option<Bytes>,
    entry: impl FnOnce() -> Option<Value>,
) -> ForwardResult<Option<Bytes>> {
    let Some(bytes) = &body else { return Ok(body) };
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(body);
    };
    let Some(effort) = value.pointer("/reasoning/effort").and_then(Value::as_str) else {
        return Ok(body);
    };
    // Without a model there is nothing to resolve; leave the body untouched.
    if value.get("model").and_then(Value::as_str).is_none() {
        return Ok(body);
    }
    let entry = entry();
    let preferred = entry
        .as_ref()
        .and_then(|entry| entry.get("multi_agent_reasoning_effort"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let levels = entry
        .and_then(|entry| entry.get("supported_reasoning_levels").cloned())
        .and_then(|levels| levels.as_array().cloned());
    let Some(levels) = levels.filter(|levels| {
        !levels.is_empty()
            && levels.iter().all(|level| {
                level
                    .get("effort")
                    .and_then(Value::as_str)
                    .is_some_and(|effort| !effort.is_empty())
            })
    }) else {
        return Ok(body);
    };
    let declared = |candidate: &str| levels.iter().any(|level| level["effort"] == candidate);
    let replacement = if effort == "ultra" {
        preferred
            .filter(|preferred| preferred != "ultra" && declared(preferred.as_str()))
            .or_else(|| declared("max").then(|| "max".to_owned()))
            .or_else(|| {
                levels
                    .iter()
                    .rev()
                    .filter_map(|level| level["effort"].as_str())
                    .find(|level| *level != "ultra")
                    .map(str::to_owned)
            })
    } else if declared(effort) {
        return Ok(body);
    } else {
        None
    };
    let reasoning = value["reasoning"]
        .as_object_mut()
        .expect("string effort in object");
    match replacement {
        Some(effort) => {
            reasoning.insert("effort".into(), Value::from(effort));
        }
        None => {
            reasoning.remove("effort");
        }
    }
    if reasoning.is_empty() {
        value
            .as_object_mut()
            .expect("request object")
            .remove("reasoning");
    }
    serde_json::to_vec(&value)
        .map(|bytes| Some(Bytes::from(bytes)))
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot re-encode request",
            )
        })
}

/// The translator cannot represent these opaque Responses items. Refuse the lossy
/// conversion, including items replayed from its continuation cache. Native
/// Responses traffic keeps them verbatim; only its upstream can validate them.
pub(super) fn validate_translated_input(input: &[Value]) -> ForwardResult<()> {
    if input.iter().any(|item| {
        matches!(item["type"].as_str(), Some("compaction" | "item_reference"))
            || item
                .get("encrypted_content")
                .is_some_and(|content| !content.is_null())
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            "conversation history cannot be translated without loss; use a native Responses provider or resend the full conversation without provider-specific items",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_thinking_switch_takes_none_as_off_and_any_other_effort_as_on() {
        use crate::core::model_metadata::{ReasoningLevel, ReasoningOptionInfo, ResolvedMetadata};
        let switch_only = || {
            let mut metadata = ResolvedMetadata::default();
            metadata.reasoning.supports = Some(true);
            metadata.reasoning.levels.value = Some(Vec::new());
            metadata.reasoning.levels.source = "catalogExact".into();
            metadata.reasoning.options = vec![ReasoningOptionInfo { kind: "toggle".into(), ..Default::default() }];
            metadata
        };
        let send = |effort: &str, base: &str, metadata: ResolvedMetadata| -> Value {
            let body = Bytes::from(json!({"model":"m","messages":[],"reasoning_effort":effort}).to_string());
            serde_json::from_slice(&prepare_thinking_switch(Some(body), || metadata, base).unwrap().unwrap()).unwrap()
        };
        let off = send("none", "https://api.xiaomimimo.com/v1", switch_only());
        assert_eq!(off["thinking"], json!({"type":"disabled"}));
        assert!(off.get("reasoning_effort").is_none());
        assert_eq!(send("high", "https://api.xiaomimimo.com/v1", switch_only())["thinking"], json!({"type":"enabled"}));
        assert_eq!(send("none", "https://openrouter.ai/api/v1", switch_only())["reasoning"], json!({"enabled":false}));
        assert_eq!(send("none", "https://dashscope.aliyuncs.com/compatible-mode/v1", switch_only())["enable_thinking"], false);
        // A model with levels of its own keeps them; only an undeclared none is the switch.
        let mut leveled = switch_only();
        leveled.reasoning.levels.value = Some(["low", "high"].map(|effort| ReasoningLevel { effort: effort.into(), description: String::new() }).into());
        assert_eq!(send("high", "https://glm.example/v1", leveled.clone())["reasoning_effort"], "high");
        assert_eq!(send("none", "https://glm.example/v1", leveled.clone())["thinking"], json!({"type":"disabled"}));
        // Levels the user set by hand, and models without a switch, are left alone.
        let mut manual = switch_only();
        manual.reasoning.levels.source = crate::core::model_metadata::source::MANUAL.into();
        assert_eq!(send("none", "https://glm.example/v1", manual)["reasoning_effort"], "none");
        assert_eq!(send("none", "https://glm.example/v1", ResolvedMetadata::default())["reasoning_effort"], "none");
        // A request without an effort is not even resolved.
        let untouched = Bytes::from_static(br#"{"model":"m","messages":[]}"#);
        assert_eq!(
            prepare_thinking_switch(Some(untouched.clone()), || unreachable!("no effort, no lookup"), "https://x.example").unwrap(),
            Some(untouched)
        );
    }

    #[test]
    fn translated_messages_effort_uses_known_levels_without_changing_history() {
        use crate::core::{
            model_metadata::{ReasoningLevel, ResolvedMetadata},
            router::ModelProtocol,
        };
        let mut metadata = ResolvedMetadata::default();
        metadata.reasoning.levels.value = Some(
            ["low", "medium", "high"]
                .iter()
                .map(|level| ReasoningLevel {
                    effort: (*level).into(),
                    description: String::new(),
                })
                .collect(),
        );
        for target in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
            let effort_key = if target == ModelProtocol::Responses {
                "reasoning"
            } else {
                "reasoning_effort"
            };
            let mut source = json!({"model":"raw-high","messages":[{"role":"system","content":"keep"}],"input":[{"role":"system","content":"keep"}]});
            source[effort_key] = if target == ModelProtocol::Responses {
                json!({"effort":"max","summary":"auto"})
            } else {
                json!("max")
            };
            let converted = prepare_message_translation(
                Some(Bytes::from(source.to_string())),
                target,
                &metadata,
            )
            .unwrap()
            .unwrap();
            let converted: Value = serde_json::from_slice(&converted).unwrap();
            assert_eq!(converted["model"], source["model"]);
            assert_eq!(converted["input"], source["input"]);
            assert_eq!(converted["messages"], source["messages"]);
            if target == ModelProtocol::Responses {
                assert_eq!(
                    converted["reasoning"],
                    json!({"effort":"high","summary":"auto"})
                );
            } else {
                assert_eq!(converted["reasoning_effort"], "high");
            }
        }
        metadata.reasoning.levels.value = Some(vec![]);
        let body = prepare_message_translation(
            Some(Bytes::from_static(
                br#"{"model":"m","reasoning":{"effort":"high"}}"#,
            )),
            ModelProtocol::Responses,
            &metadata,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"model":"m"})
        );
    }

    #[test]
    fn translated_effort_preserves_unknown_capabilities_and_never_raises_intent() {
        use crate::core::{
            model_metadata::{ReasoningLevel, ResolvedMetadata},
            router::ModelProtocol,
        };
        for target in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
            for (supports, levels, requested, expected) in [
                (None, None, "max", Some("max")),
                (Some(true), None, "high", Some("high")),
                (Some(false), None, "high", None),
                (Some(false), Some(vec!["high"]), "high", None),
                (Some(true), Some(vec![]), "high", None),
                (Some(true), Some(vec!["low", "high"]), "medium", Some("low")),
                (Some(true), Some(vec!["high"]), "low", None),
                (Some(true), Some(vec!["low", "high"]), "none", None),
                (Some(true), Some(vec!["none", "high"]), "none", Some("none")),
                (Some(true), Some(vec!["xhigh", "max"]), "max", Some("max")),
            ] {
                let mut metadata = ResolvedMetadata::default();
                metadata.reasoning.supports = supports;
                metadata.reasoning.levels.value = levels.map(|levels| {
                    levels
                        .into_iter()
                        .map(|effort| ReasoningLevel {
                            effort: effort.into(),
                            description: String::new(),
                        })
                        .collect()
                });
                let source = if target == ModelProtocol::Responses {
                    json!({"model":"unchanged","input":"keep","reasoning":{"effort":requested,"summary":"auto"}})
                } else {
                    json!({"model":"unchanged","messages":[],"reasoning_effort":requested})
                };
                let bytes = prepare_message_translation(
                    Some(Bytes::from(source.to_string())),
                    target,
                    &metadata,
                )
                .unwrap()
                .unwrap();
                let out: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(out["model"], source["model"]);
                if target == ModelProtocol::Responses {
                    assert_eq!(out["reasoning"]["effort"].as_str(), expected);
                    assert_eq!(out["reasoning"]["summary"], "auto");
                    assert_eq!(out["input"], source["input"]);
                } else {
                    assert_eq!(out["reasoning_effort"].as_str(), expected);
                    assert_eq!(out["messages"], source["messages"]);
                }
            }
        }
    }

    #[test]
    fn unsupported_effort_is_removed_without_changing_history_or_summary() {
        let input = json!({
            "model": "relay::model", "reasoning": {"effort": "none", "summary": "auto"},
            "previous_response_id": "provider-response",
            "input": [
                {"type":"reasoning", "summary":[{"type":"summary_text","text":"keep me"}], "encrypted_content":"opaque"},
                {"type":"compaction", "encrypted_content":"compressed-history"}
            ]
        });
        let body = prepare_reasoning(Some(Bytes::from(input.to_string())), || {
            Some(
                json!({"slug": "relay::model", "supported_reasoning_levels": [{"effort": "high"}]}),
            )
        })
        .unwrap()
        .unwrap();
        let mut expected = input;
        expected["reasoning"]
            .as_object_mut()
            .unwrap()
            .remove("effort");
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected);

        let body = prepare_reasoning(
            Some(Bytes::from_static(
                br#"{"model":"relay::model","reasoning":{"effort":"none"}}"#,
            )),
            || Some(json!({"slug": "relay::model", "supported_reasoning_levels": [{"effort": "high"}]})),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"model":"relay::model"})
        );
    }

    #[test]
    fn ultra_resolves_like_codex_and_never_reaches_the_wire() {
        let ultra = json!({"model":"relay::model","reasoning":{"effort":"ultra","summary":"auto"}});
        let level = |effort: &str| json!({"effort": effort});
        for (entry, expected) in [
            (
                json!({"supported_reasoning_levels": [level("high"), level("xhigh"), level("max"), level("ultra")],
                    "multi_agent_reasoning_effort": "xhigh"}),
                Some("xhigh"),
            ),
            (
                json!({"supported_reasoning_levels": [level("high"), level("max"), level("ultra")],
                    "multi_agent_reasoning_effort": "xhigh"}),
                Some("max"),
            ),
            (
                json!({"supported_reasoning_levels": [level("low"), level("xhigh"), level("ultra")]}),
                Some("xhigh"),
            ),
            (json!({"supported_reasoning_levels": [level("ultra")]}), None),
        ] {
            let body = prepare_reasoning(Some(Bytes::from(ultra.to_string())), || Some(entry))
                .unwrap()
                .unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["reasoning"]["summary"], "auto");
            assert_eq!(body["reasoning"]["effort"].as_str(), expected);
        }
    }

    #[test]
    fn supported_unknown_or_malformed_capabilities_leave_bytes_unchanged() {
        let body =
            Bytes::from_static(br#"{ "model":"relay::model", "reasoning":{"effort":"none"} }"#);
        for levels in [
            json!([{"effort":"none"}]),
            json!([]),
            Value::Null,
            json!("high"),
            json!([{"effort":"high"},{}]),
        ] {
            assert_eq!(
                prepare_reasoning(Some(body.clone()), || {
                    Some(json!({"slug": "relay::model", "supported_reasoning_levels": levels}))
                })
                .unwrap()
                .unwrap(),
                body
            );
        }
        assert_eq!(
            prepare_reasoning(Some(body.clone()), || None)
                .unwrap()
                .unwrap(),
            body
        );
        let body = Bytes::from_static(br#"{"model":"relay::model","input":"hi"}"#);
        assert_eq!(
            prepare_reasoning(Some(body.clone()), || panic!("no effort: no catalog read"))
                .unwrap()
                .unwrap(),
            body
        );
    }
}
