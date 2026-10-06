//! Native frames keep their bytes while invalid terminal markers are blocked
//! before an SDK can stop reading at [DONE]. This is framing, not retry staging.
use super::Bytes;
use crate::core::protocol::{canonical::TerminalObservation, ModelProtocol};

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct TerminalGate {
    protocol: ModelProtocol,
    pending: Vec<u8>,
    scan_from: usize,
    terminal: TerminalObservation,
    at_start: bool,
}

impl TerminalGate {
    pub fn new(protocol: ModelProtocol) -> Self {
        Self {
            protocol,
            pending: Vec::new(),
            scan_from: 0,
            terminal: Default::default(),
            at_start: true,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Bytes, &'static str> {
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        while let Some(end) =
            separator(&self.pending[self.scan_from..]).map(|end| end + self.scan_from)
        {
            if end > MAX_FRAME_BYTES {
                return Err("invalid_stream_frame");
            }
            let raw: Vec<_> = self.pending.drain(..end).collect();
            self.scan_from = 0;
            self.observe(&raw)?;
            output.extend_from_slice(&raw);
        }
        self.scan_from = self.pending.len().saturating_sub(3);
        if self.pending.len() > MAX_FRAME_BYTES {
            return Err("invalid_stream_frame");
        }
        Ok(Bytes::from(output))
    }

    pub fn flush(&mut self) -> Result<Bytes, &'static str> {
        let raw = std::mem::take(&mut self.pending);
        self.scan_from = 0;
        self.observe(&raw)?;
        // Stream validity is independent of the bounded diagnostic sampler:
        // a large native event can exhaust its budget before a terminal arrives.
        if !self.terminal.received() {
            return Err("upstream_stream_truncated");
        }
        Ok(Bytes::from(raw))
    }

    fn observe(&mut self, raw: &[u8]) -> Result<(), &'static str> {
        let bytes = crate::core::protocol::canonical::strip_stream_bom(raw, &mut self.at_start);
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid_stream_frame")?;
        let data = text
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data:")
                    .map(|data| data.strip_prefix(' ').unwrap_or(data))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if data.trim() == "[DONE]" {
            self.terminal.done();
            if !self.terminal.received() {
                return Err("upstream_stream_truncated");
            }
        } else if let Ok(value) = serde_json::from_str(&data) {
            let name = text
                .lines()
                .find_map(|line| line.strip_prefix("event:"))
                .unwrap_or("")
                .trim();
            self.terminal.observe(self.protocol, &value, name);
        }
        Ok(())
    }
}

fn separator(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(2)
        .position(|part| part == b"\n\n")
        .map(|index| index + 2)
        .into_iter()
        .chain(
            bytes
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .map(|index| index + 4),
        )
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_done_stopping_client_never_sees_an_invalid_marker() {
        let partial = b"data: {\"model\":\"wire\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n";
        for width in [1, 7, 8192] {
            let mut gate = TerminalGate::new(ModelProtocol::ChatCompletions);
            let mut received = Vec::new();
            for chunk in partial.chunks(width) {
                received.extend_from_slice(&gate.push(chunk).unwrap());
            }
            let mut failure = None;
            for chunk in b"data: [DONE]\n\n".chunks(width) {
                match gate.push(chunk) {
                    Ok(bytes) => received.extend_from_slice(&bytes),
                    Err(code) => {
                        failure = Some(code);
                        break;
                    }
                }
                assert!(!received.windows(6).any(|bytes| bytes == b"[DONE]"));
            }
            assert_eq!(failure, Some("upstream_stream_truncated"));
            assert_eq!(received, partial);
        }
    }

    #[test]
    fn native_unknown_frames_and_valid_terminals_keep_exact_bytes() {
        for (protocol, terminal) in [
            (
                ModelProtocol::ChatCompletions,
                "data: {\"choices\":[{\"finish_reason\":\"length\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
            ),
            (
                ModelProtocol::Responses,
                "data: {\"type\":\"response.incomplete\"}\r\n\r\ndata: [DONE]\r\n\r\n",
            ),
            (
                ModelProtocol::AnthropicMessages,
                "data: {\"type\":\"message_stop\"}\r\n\r\n",
            ),
        ] {
            let wire =
                format!("event: vendor.future\r\ndata: {{\"opaque\":true}}\r\n\r\n{terminal}");
            let mut gate = TerminalGate::new(protocol);
            let mut output = Vec::new();
            for byte in wire.as_bytes() {
                output.extend_from_slice(&gate.push(&[*byte]).unwrap());
            }
            output.extend_from_slice(&gate.flush().unwrap());
            assert_eq!(output, wire.as_bytes());
        }
    }

    #[test]
    fn eof_terminal_detection_is_independent_of_diagnostic_budget() {
        let value = serde_json::json!({"choices":[{
            "delta":{"content":"x".repeat(300 * 1024)}, "finish_reason":null
        }]});
        let wire = format!("data: {value}\n\n");
        let mut gate = TerminalGate::new(ModelProtocol::ChatCompletions);
        assert_eq!(gate.push(wire.as_bytes()).unwrap(), wire.as_bytes());
        assert_eq!(gate.flush().unwrap_err(), "upstream_stream_truncated");

        let mut gate = TerminalGate::new(ModelProtocol::ChatCompletions);
        gate.push(wire.as_bytes()).unwrap();
        gate.push(b"data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
            .unwrap();
        assert!(gate.flush().unwrap().is_empty());
    }

    #[test]
    fn encoding_prefixes_cannot_hide_invalid_terminal_markers() {
        for (wire, expected) in [
            (
                b"\xef\xbb\xbfdata: [DONE]\n\n".as_slice(),
                "upstream_stream_truncated",
            ),
            (
                b": \xff\n\ndata: [DONE]\n\n".as_slice(),
                "invalid_stream_frame",
            ),
        ] {
            for width in [1, 7, 8192] {
                let mut gate = TerminalGate::new(ModelProtocol::ChatCompletions);
                let mut output = Vec::new();
                let mut failure = None;
                for chunk in wire.chunks(width) {
                    match gate.push(chunk) {
                        Ok(bytes) => output.extend_from_slice(&bytes),
                        Err(code) => {
                            failure = Some(code);
                            break;
                        }
                    }
                }
                assert_eq!(failure, Some(expected));
                assert!(!output.windows(6).any(|bytes| bytes == b"[DONE]"));
            }
        }
        let wire =
            b"\xef\xbb\xbfdata: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let mut gate = TerminalGate::new(ModelProtocol::ChatCompletions);
        let mut output = Vec::new();
        for byte in wire {
            output.extend_from_slice(&gate.push(&[*byte]).unwrap());
        }
        output.extend_from_slice(&gate.flush().unwrap());
        assert_eq!(output, wire);
    }
}
