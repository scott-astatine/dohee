//! `dohee-core` manages the main agentic REPL loop and the state machine of the coding session.
//! It coordinates model prompts, parsed tool invocations, user approval gates, and the feedback loop.

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
