//! The Zed side of `satz lsp`: find the `satz` binary and hand Zed the command.
//! Everything the server knows lives in satz; this crate is the two lines Zed
//! needs to start it.

use zed_extension_api::{self as zed, settings::LspSettings, LanguageServerId, Result};

struct SatzExtension;

impl zed::Extension for SatzExtension {
    fn new() -> Self {
        SatzExtension
    }

    fn language_server_command(&mut self, id: &LanguageServerId, worktree: &zed::Worktree) -> Result<zed::Command> {
        // A `binary.path` in Zed's lsp settings wins; else `satz` on the PATH the
        // worktree's shell sees.
        let settings = LspSettings::for_worktree(id.as_ref(), worktree).ok().and_then(|s| s.binary);
        let path = match settings.as_ref().and_then(|b| b.path.clone()) {
            Some(p) => p,
            None => worktree.which("satz").ok_or_else(|| {
                "satz is not on the PATH: install it (https://github.com/tjirsch/satz#installation) \
                 or set `lsp.satz.binary.path` in Zed's settings"
                    .to_string()
            })?,
        };
        let args = settings.and_then(|b| b.arguments).unwrap_or_else(|| vec!["lsp".to_string()]);
        Ok(zed::Command { command: path, args, env: worktree.shell_env() })
    }
}

zed::register_extension!(SatzExtension);
