const DATA_PREFIX: &str = "data:";

#[derive(Default)]
pub struct SseLines {
    unfinished: Vec<u8>,
}

impl SseLines {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.unfinished.extend_from_slice(chunk);
        let mut payloads = Vec::new();
        while let Some(line) = self.take_line() {
            payloads.extend(data_payload(&line));
        }
        payloads
    }

    fn take_line(&mut self) -> Option<String> {
        let end = self.unfinished.iter().position(|byte| *byte == b'\n')?;
        let line: Vec<u8> = self.unfinished.drain(..=end).collect();
        Some(String::from_utf8_lossy(&line).trim_end().to_owned())
    }
}

fn data_payload(line: &str) -> Option<String> {
    let payload = line.strip_prefix(DATA_PREFIX)?;
    Some(payload.trim_start().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yields_one_payload_per_data_line() {
        let mut lines = SseLines::default();
        assert_eq!(
            lines.push(b"data: {\"a\":1}\n\ndata: [DONE]\n\n"),
            ["{\"a\":1}", "[DONE]"]
        );
    }

    #[test]
    fn waits_for_the_rest_of_a_line_split_across_chunks() {
        let mut lines = SseLines::default();
        assert!(lines.push(b"data: {\"text\":\"hel").is_empty());
        assert_eq!(lines.push(b"lo\"}\n"), ["{\"text\":\"hello\"}"]);
    }

    #[test]
    fn keeps_a_character_split_across_chunks_intact() {
        let line = "data: привет\n".as_bytes();
        let (head, tail) = line.split_at(9);
        let mut lines = SseLines::default();
        assert!(lines.push(head).is_empty());
        assert_eq!(lines.push(tail), ["привет"]);
    }

    #[test]
    fn ignores_comments_event_names_and_windows_line_endings() {
        let mut lines = SseLines::default();
        assert_eq!(
            lines.push(b": keep-alive\r\nevent: message\r\ndata: ok\r\n\r\n"),
            ["ok"]
        );
    }
}
