//! `dohee-config` manages the configuration schemas and loading hierarchy for the project.
//! It merges built-in settings with global config files (~/.config/dohee/config.toml) and project-specific files (.dohee.toml).

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
