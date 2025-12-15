use codex_branding::CONFIG_DIR_NAME;
use codex_branding::ENV_VAR_HOME;
use dirs::home_dir;
use std::path::PathBuf;

/// Returns the path to the Azure Codex configuration directory, which can be
/// specified by the `AZURE_CODEX_HOME` environment variable. If not set, defaults to
/// `~/.azure-codex`.
///
/// This is separate from OpenAI Codex's `~/.codex` directory to allow both
/// tools to coexist on the same machine.
///
/// - If `AZURE_CODEX_HOME` is set, the value will be canonicalized and this
///   function will Err if the path does not exist.
/// - If `AZURE_CODEX_HOME` is not set, this function does not verify that the
///   directory exists.
pub(crate) fn find_codex_home() -> std::io::Result<PathBuf> {
    // Honor the `AZURE_CODEX_HOME` environment variable when it is set to allow users
    // (and tests) to override the default location.
    if let Ok(val) = std::env::var(ENV_VAR_HOME)
        && !val.is_empty()
    {
        return PathBuf::from(val).canonicalize();
    }

    let mut p = home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not find home directory",
        )
    })?;
    p.push(CONFIG_DIR_NAME);
    Ok(p)
}
