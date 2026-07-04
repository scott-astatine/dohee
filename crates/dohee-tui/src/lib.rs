//! `dohee-tui` provides an interactive terminal user interface built on `ratatui`.
//! It displays session history panels, live token usage meters, and interactive approval menus for agent tool invocations.

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
