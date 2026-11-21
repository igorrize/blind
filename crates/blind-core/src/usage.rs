#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    #[must_use]
    pub fn plus(self, other: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_adds_up_per_direction() {
        let first = Usage {
            input_tokens: 7,
            output_tokens: 3,
        };
        let second = Usage {
            input_tokens: 10,
            output_tokens: 20,
        };
        assert_eq!(
            first.plus(second),
            Usage {
                input_tokens: 17,
                output_tokens: 23
            }
        );
    }
}
