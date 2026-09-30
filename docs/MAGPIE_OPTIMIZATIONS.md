# Magpie-inspired improvements

AiMaMi's implementation follows the local Magpie snapshot
7a3337dd25bd759b1a73a051cdefeab6033b63f6. The existing capability-matching
and thread UI changes remain in place.

## Routing diagnostics

The provider settings page now exposes a recent request trace. Each trace
records the requested model, the normalized capability identity, the selected
provider, the protocol, the reasoning and output-budget values actually sent,
each route attempt, the failure stage, duration, and time to first token.
Request bodies, prompts, headers, URLs, API keys, and remote response bodies are
excluded. The registry is bounded to the most recent 60 requests and is kept
in memory.

## Unified connectivity probes

The streaming probe engine is shared by the provider UI and legacy test
commands. It starts with a 64-token output budget and preserves HTTP, JSON, and
SSE error details after redacting credentials. A single retry is allowed only
for an explicit HTTP 400/422 minimum-output-budget error, capped at 4096
tokens. Once streaming has started, no retry is attempted.

## Capability catalog and preview

Capability lookup is based on the model identity, independently of the
channel. Display aliases, channel prefixes, and the upstream request ID remain
unchanged. The preview shows:

- display name, upstream model ID, and normalized capability identity;
- reasoning levels and default, context window, and output limit;
- a source and update time for each field (local, catalog, upstream, manual,
  contextLimit, or unknown).

The catalog loader validates bounded input before publishing it. A malformed,
empty, truncated, or unavailable refresh retains the previous valid snapshot,
including across restart through a private last-good backup. The router and
preview use the same loader. The router performs a bounded local refresh while
running; the UI marks stale data and shows the retained error.

## Indexed history details

Thread detail scanning builds a byte-offset index while collecting the existing
statistics. Later pages seek directly to the indexed records instead of
rescanning from byte zero. The cache stores offsets only, with limits of eight
files and 32 MiB of index data. A snapshot token and file fingerprint detect
truncation, replacement, append, same-size edits, and timestamp-preserving
changes; changed files require a fresh detail snapshot rather than combining
pages from different versions.

## Same-model channel failover

Failover is opt-in and disabled by default. It requires a recognized stable
session header and an identified new stateless turn. The alternate channel must
match the exact upstream model ID and protocol. At most one alternate attempt
is made, and only for a non-timeout connection failure or HTTP 429 before any
output is exposed.

Timeouts, 5xx responses, started streams, tool calls, continuation turns, and
requests without a stable session ID are never replayed. Once a session is
bound to a provider, subsequent turns keep that provider; a disabled or
changed bound provider fails closed. Bindings are bounded in memory and are not
restored after an application restart.

## Verification

- 442 Rust unit tests passed; 4 tests remain ignored, including live-auth
  checks.
- 9 local workflow integration tests passed.
- npm run build passed.
- The frontend regression suite passed, including the routing diagnostics,
  capability preview, stale-cache warning, and mobile layout checks.
- A synthetic 20.8 MB history with 10,000 messages completed 20 late-page
  reads in 186 ms after index construction, versus 22,126 ms for 20 full
  rescans on the same debug build. This is a local synthetic benchmark, not a
  general application latency guarantee.
