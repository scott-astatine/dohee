use anyhow::{Context, Result};
use landlock::{
    Access, AccessFs, PathBeneath, PathFd, RestrictionStatus, Ruleset, RulesetAttr,
    RulesetCreatedAttr, ABI,
};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum SandboxPolicy {
    ReadOnly,
    WorkspaceWrite { root: PathBuf },
    DangerFullAccess,
}

pub struct Sandbox;

impl Sandbox {
    pub fn apply(policy: &SandboxPolicy) -> Result<Option<RestrictionStatus>> {
        match policy {
            SandboxPolicy::DangerFullAccess => {
                // No sandboxing
                Ok(None)
            }
            SandboxPolicy::ReadOnly => {
                let abi = ABI::V1;
                let access_all = AccessFs::from_all(abi);
                let access_read = AccessFs::from_read(abi);

                let root_fd = PathFd::new("/").context("Failed to open '/' for sandboxing")?;

                let status = Ruleset::default()
                    .handle_access(access_all)?
                    .create()?
                    .add_rule(PathBeneath::new(root_fd, access_read))?
                    .restrict_self()
                    .context("Failed to enforce Landlock read-only sandbox")?;

                Ok(Some(status))
            }
            SandboxPolicy::WorkspaceWrite { root } => {
                let abi = ABI::V1;
                let access_all = AccessFs::from_all(abi);
                let access_read = AccessFs::from_read(abi);
                let access_write = AccessFs::from_write(abi);

                let root_fd = PathFd::new("/").context("Failed to open '/' for sandboxing")?;
                let workspace_fd = PathFd::new(root).context("Failed to open workspace root for write sandboxing")?;

                let status = Ruleset::default()
                    .handle_access(access_all)?
                    .create()?
                    .add_rule(PathBeneath::new(root_fd, access_read))?
                    .add_rule(PathBeneath::new(workspace_fd, access_write))?
                    .restrict_self()
                    .context("Failed to enforce Landlock workspace-write sandbox")?;

                Ok(Some(status))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_sandbox_policy_workspace_write() -> Result<()> {
        // Skip test if not running on Linux or if Landlock is not supported by kernel
        if !cfg!(target_os = "linux") {
            return Ok(());
        }

        let temp_workspace = tempdir()?;
        let temp_outside = tempdir()?;

        let workspace_root = temp_workspace.path().to_path_buf();
        let outside_dir = temp_outside.path().to_path_buf();

        let policy = SandboxPolicy::WorkspaceWrite { root: workspace_root.clone() };

        // Apply sandbox (this will affect the current test thread/process)
        let status = Sandbox::apply(&policy)?;
        if let Some(status) = status {
            println!("Landlock enforcement status: {:?}", status.ruleset);
        } else {
            println!("Landlock not supported or not enforced.");
            return Ok(()); // Skip validation if landlock is not supported by host kernel
        }

        // 1. Verify writing inside the workspace succeeds
        let inside_file = workspace_root.join("inside.txt");
        fs::write(&inside_file, "inside").context("Write inside workspace failed")?;
        assert_eq!(fs::read_to_string(inside_file)?, "inside");

        // 2. Verify writing outside the workspace fails with PermissionDenied
        let outside_file = outside_dir.join("outside.txt");
        let write_result = fs::write(outside_file, "outside");
        
        assert!(
            write_result.is_err(),
            "Expected write outside workspace to fail, but it succeeded!"
        );

        if let Err(e) = write_result {
            assert_eq!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied,
                "Expected PermissionDenied error, got: {:?}",
                e
            );
        }

        Ok(())
    }
}
