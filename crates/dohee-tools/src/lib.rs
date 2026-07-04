//! `dohee-tools` implements the core action set accessible to the AI agent.
//! This includes filesystem directory listing, file reading, file editing (diff/regex patching), grep searching, and local shell execution.

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
