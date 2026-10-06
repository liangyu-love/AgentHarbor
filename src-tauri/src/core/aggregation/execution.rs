//! Immutable request requirements and attempt boundaries, independent of HTTP
//! transport and client configuration writers.
use super::{affinity::Owner, projection::ResolvedMember, selector::Candidate};
use crate::core::{
    protocol::{self, ModelProtocol},
    router,
};
use serde_json::Value;
use std::time::{Duration, Instant};

pub const TOTAL_DEADLINE: Duration = Duration::from_secs(900);
#[derive(Clone, Debug)]
pub struct Requirements {
    pub ingress: ModelProtocol,
    pub one_m: bool,
    pub output_tokens: Option<u64>,
    pub images: bool,
    pub tools: bool,
    pub search: bool,
    pub private_history: bool,
    pub continuation: bool,
    pub portable_fresh: bool,
}
impl Requirements {
    pub fn parse(ingress: ModelProtocol, value: &Value, one_m: bool) -> Result<Self, &'static str> {
        if !value.is_object() {
            return Err("invalid_request");
        }
        let tools = value["tools"].as_array();
        let has_tools = tools.is_some_and(|tools| !tools.is_empty());
        let search = tools.is_some_and(|tools| {
            tools.iter().any(|tool| {
                protocol::web_search::is_search(tool)
                    || tool["type"]
                        .as_str()
                        .is_some_and(|kind| kind.starts_with("web_search"))
            })
        });
        let mut images = false;
        let mut private_history = false;
        let mut history = false;
        // Inspect conversation items only. Tool schemas and function arguments
        // may legitimately contain keys named signature, role or image.
        let mut pending: Vec<_> = ["input", "messages", "system"]
            .iter()
            .filter_map(|key| value.get(*key))
            .collect();
        while let Some(item) = pending.pop() {
            match item {
                Value::Array(items) => pending.extend(items),
                Value::Object(object) => {
                    let kind = object
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    images |= matches!(kind, "image" | "image_url" | "input_image");
                    private_history |= matches!(kind, "thinking" | "redacted_thinking")
                        || object.get("signature").is_some()
                        || object.get("encrypted_content").is_some();
                    history |= matches!(
                        object.get("role").and_then(Value::as_str),
                        Some("assistant" | "tool")
                    ) || matches!(
                        kind,
                        "function_call"
                            | "function_call_output"
                            | "tool_use"
                            | "tool_result"
                            | "server_tool_use"
                            | "web_search_tool_result"
                    );
                    if let Some(content) = object.get("content") {
                        pending.push(content);
                    }
                }
                _ => {}
            }
        }
        let resource = value
            .get("previous_response_id")
            .is_some_and(|id| !id.is_null())
            || value.get("conversation").is_some_and(|id| !id.is_null())
            || value["background"] == true;
        Ok(Self {
            ingress,
            one_m,
            images,
            tools: has_tools,
            search,
            private_history,
            output_tokens: ["max_output_tokens", "max_tokens", "max_completion_tokens"]
                .iter()
                .filter_map(|key| value.get(*key).and_then(Value::as_u64))
                .max(),
            continuation: history || resource,
            portable_fresh: !history && !resource && !private_history && !has_tools,
        })
    }

    pub fn admit(
        &self,
        member: &ResolvedMember,
        effective: ModelProtocol,
    ) -> Result<(), &'static str> {
        if !protocol::Translation::can_serve(self.ingress, effective) {
            return Err("protocol_unsupported");
        }
        if self.images && member.vision != Some(true) {
            return Err("vision_unavailable");
        }
        if self.tools && member.tools == Some(false) {
            return Err("tools_unavailable");
        }
        if self.one_m
            && (member.projection.context_is_fallback
                || member.projection.context_window < 1_000_000)
        {
            return Err("context_1m_unavailable");
        }
        if self.output_tokens.is_some_and(|tokens| {
            tokens
                > member
                    .projection
                    .output_limit
                    .unwrap_or(crate::core::model_metadata::FALLBACK_MAX_OUTPUT_TOKENS)
        }) {
            return Err("output_limit_exceeded");
        }
        if self.private_history && self.ingress != effective {
            return Err("private_history_protocol");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct PlannedMember {
    pub resolved: ResolvedMember,
    pub effective_protocol: ModelProtocol,
    pub compatibility_plan: Option<&'static str>,
    pub effective_target_key: String,
}
impl PlannedMember {
    pub fn candidate(&self) -> Candidate {
        let member = &self.resolved.member;
        Candidate {
            member_id: member.id.clone(),
            provider_id: member.provider_id.clone(),
            actual_model_id: member.actual_model_id.clone(),
            priority: member.priority,
            weight: member.weight,
            target_key: self.effective_target_key.clone(),
        }
    }
    pub fn owner(&self, aggregate_id: &str, native_resource: bool) -> Owner {
        Owner {
            aggregate_id: aggregate_id.into(),
            member_id: self.resolved.member.id.clone(),
            provider_id: self.resolved.member.provider_id.clone(),
            actual_model_id: self.resolved.member.actual_model_id.clone(),
            connection_key: self.resolved.target_key.clone(),
            configured_protocol: self.resolved.protocol,
            effective_protocol: self.effective_protocol,
            native_resource,
        }
    }
    pub fn agrees(&self, owner: &Owner, endpoint_state: bool) -> bool {
        self.resolved.member.id == owner.member_id
            && self.resolved.member.provider_id == owner.provider_id
            && self.resolved.member.actual_model_id == owner.actual_model_id
            && self.resolved.target_key == owner.connection_key
            && (!endpoint_state || self.effective_protocol == owner.effective_protocol)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    ConnectBeforeSend,
    RateLimited,
    Timeout,
    Transport,
    Http,
    SideEffect,
}
pub struct Attempts {
    deadline: Instant,
    max: u8,
    used: u8,
    safe: bool,
    committed: bool,
}
impl Attempts {
    pub fn new(max: u8, requirements: &Requirements, bound: bool) -> Self {
        Self {
            deadline: Instant::now() + TOTAL_DEADLINE,
            max: max.clamp(1, 2),
            used: 0,
            safe: requirements.portable_fresh && !bound,
            committed: false,
        }
    }
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
    pub fn begin(&mut self) -> Result<(), &'static str> {
        if self.remaining().is_zero() {
            return Err("request_deadline");
        }
        if self.used >= self.max {
            return Err("attempt_budget");
        }
        self.used += 1;
        Ok(())
    }
    pub fn commit(&mut self) {
        self.committed = true;
    }
    pub fn may_retry(&self, failure: Failure) -> bool {
        self.safe
            && !self.committed
            && self.used < self.max
            && !self.remaining().is_zero()
            && matches!(failure, Failure::ConnectBeforeSend | Failure::RateLimited)
    }
}

/// Revalidation can veto a frozen attempt but must never replace its key, URL
/// or model from a newer generation of configuration.
pub fn still_authorized(
    store: &router::Store,
    aggregate_id: &str,
    planned: &PlannedMember,
) -> bool {
    let Some(aggregate) = store
        .aggregates
        .iter()
        .find(|aggregate| aggregate.id == aggregate_id && aggregate.enabled)
    else {
        return false;
    };
    let Some(member) = aggregate
        .members
        .iter()
        .find(|member| member.id == planned.resolved.member.id && member.enabled)
    else {
        return false;
    };
    let Some(provider) = store
        .providers
        .iter()
        .find(|provider| provider.id == member.provider_id && provider.enabled)
    else {
        return false;
    };
    member.actual_model_id == planned.resolved.member.actual_model_id
        && provider.can_route_model(&member.actual_model_id)
        && provider
            .models
            .iter()
            .find(|model| model.id == member.actual_model_id)
            .is_none_or(|model| model.included)
        && super::target_key(
            provider,
            &member.actual_model_id,
            provider.protocol_for_model(&member.actual_model_id),
        ) == planned.resolved.target_key
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tools_private_state_and_deadline_disable_whole_request_replay() {
        let plain = Requirements::parse(ModelProtocol::Responses, &json!({"input":"hello"}), false)
            .unwrap();
        let mut attempts = Attempts::new(2, &plain, false);
        attempts.begin().unwrap();
        assert!(attempts.may_retry(Failure::ConnectBeforeSend));
        assert!(attempts.may_retry(Failure::RateLimited));
        assert!(!attempts.may_retry(Failure::Timeout));
        attempts.commit();
        assert!(!attempts.may_retry(Failure::RateLimited));
        let tools = Requirements::parse(ModelProtocol::AnthropicMessages, &json!({"tools":[{"type":"web_search_20250305","name":"web_search"}],"messages":[{"role":"user","content":"search"}]}), false).unwrap();
        let mut attempts = Attempts::new(2, &tools, false);
        attempts.begin().unwrap();
        assert!(tools.search);
        assert!(!attempts.may_retry(Failure::RateLimited));
        let private = Requirements::parse(
            ModelProtocol::Responses,
            &json!({"input":[{"type":"reasoning","encrypted_content":"opaque"}]}),
            false,
        )
        .unwrap();
        assert!(private.private_history);
        assert!(!private.portable_fresh);
    }

    #[test]
    fn tool_schema_property_names_are_not_conversation_capabilities() {
        let request = json!({"input":"hello", "tools":[{"type":"function","name":"sign",
            "parameters":{"type":"object", "properties":{"signature":{"type":"string"},
                "preview":{"type":"image"}, "role":{"const":"assistant"}}}}]});
        let requirements = Requirements::parse(ModelProtocol::Responses, &request, false).unwrap();
        assert!(requirements.tools);
        assert!(!requirements.images);
        assert!(!requirements.private_history);
        assert!(!requirements.continuation);
        let actual = json!({"input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://example.invalid/a.png"}]}]});
        assert!(
            Requirements::parse(ModelProtocol::Responses, &actual, false)
                .unwrap()
                .images
        );
    }
}
