//! The translation pipeline every routed attempt runs through, whatever led
//! to it (a provider path, a Codex slug, an aggregate member, a Claude Desktop
//! route, the gateway or an account source).
//!
//! Request, in order (`request::prepare`):
//! 1. **target** — upstream URL and credential headers (`target`).
//! 2. **beta** — Claude's 1M-context beta header when the client asked for it.
//! 3. **history** — Messages ingress: the private message history the target
//!    protocol cannot carry is remembered for this conversation.
//! 4. **search** — a hosted WebSearch call is planned and its body normalised.
//! 5. **translate** — the body in the upstream's protocol, or the native
//!    preparation a relay needs (`translate`).
//! 6. **restore** — remembered history is put back into the translated body.
//! 7. **compat** — Messages ingress: reasoning and output limits the target
//!    model takes (`session_compat`).
//! 8. **tools** — tool schemas adapted to the model (`tool_schema`).
//! 9. **vendor** — an account source's own request shape.
//! 10. **meter** — outbound client, credential snapshot, price and usage record.
//!
//! Response (`relay`): an account source's refusal explained, then the stream
//! translated or adapted to the client's protocol while usage is observed.
pub(super) mod relay;
pub(super) mod request;
pub(super) mod target;
pub(super) mod translate;
