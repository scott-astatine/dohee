//! `dohee-mcp` implements a client for the Model Context Protocol (MCP).
//! It allows the local agent to connect to third-party MCP servers, exposing external databases, tools, or resources as standard agent tools.

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
