use std::path::Path;

pub fn suggest_verification_command(root: &Path) -> Option<&'static str> {
    if root.join("Cargo.toml").exists() {
        return Some("cargo check");
    }
    if root.join("package.json").exists() {
        return Some("npm exec -y tsc --noEmit");
    }
    if root.join("pyproject.toml").exists() || root.join("pytest.ini").exists() {
        return Some("pytest -q");
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::suggest_verification_command;

    #[test]
    fn unknown_root_has_no_verifier() {
        assert_eq!(suggest_verification_command(Path::new("Z:\\unlikely")), None);
    }
}
