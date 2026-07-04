//! `dohee-context` handles token accounting, conversation compaction, and tool output pruning.
//! It manages context windows by replacing old output logs with truncated summaries and running local LLM summarization.

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
