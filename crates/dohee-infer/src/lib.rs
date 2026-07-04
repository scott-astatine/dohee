//! `dohee-infer` encapsulates the in-process llama.cpp bindings (via `llama-cpp-2`).
//! It is responsible for model loading, VRAM/RAM allocation, context updates, KV cache management, and token stream generation.

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
