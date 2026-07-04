//! `dohee-sandbox` provides process-level sandboxing on Linux using Landlock LSM.
//! It limits the capabilities of spawned shell tools to prevent access to the host outside the project directory and block unauthorized network requests.

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
