//! `satz mcp-config` — the MCP client configuration this estate needs, printed.
//!
//! `satz mcp` is a server a client starts. The other half of that is the block the
//! client reads to start it: which binary, which directory it may work under, and
//! which capability ceiling. All three are satz's own knowledge — the root is the
//! estate's directory, the ceiling is a grant `satz mcp` defines and parses — so a
//! front end that assembled the block itself would be a second implementation of
//! both, drifting the day a flag changes.
//!
//! It prints, like `prowler`. `--write` is the one thing it does to disk, and it
//! owns exactly one key of the file it writes: every other server stays as it is,
//! and a satz key already there with other arguments is a refusal until `--force`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::mcp::Level;

/// The clients satz writes a configuration for. Each reads a different file and
/// names its servers differently; the server itself is the same process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Client {
    /// `.mcp.json`, read from the directory Claude Code starts in
    ClaudeCode,
    /// the `mcpServers` block of Claude Desktop's own configuration file
    ClaudeDesktop,
}

impl Client {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Client::ClaudeCode => "claude-code",
            Client::ClaudeDesktop => "claude-desktop",
        }
    }

    /// stdio is the only transport satz serves. Claude Code names the transport in
    /// its entry; Claude Desktop's file carries command and args alone.
    fn transport(self) -> Option<&'static str> {
        match self {
            Client::ClaudeCode => Some("stdio"),
            Client::ClaudeDesktop => None,
        }
    }
}

/// One server entry, written in the order a reader wants to see it: what it is,
/// what runs, what it is given.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Server {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    transport: Option<&'static str>,
    command: String,
    args: Vec<String>,
}

/// The whole block, which is what both files hold at their top level.
#[derive(Debug, Serialize)]
struct Block {
    #[serde(rename = "mcpServers")]
    servers: BTreeMap<String, Server>,
}

/// The configuration one client needs for one estate.
pub(crate) struct ClientConfig {
    pub client: Client,
    /// The key the server is written under. `satz` for Claude Code, whose file
    /// belongs to one project; `satz-<estate>` for Claude Desktop, whose single
    /// file holds every server a person has.
    pub key: String,
    /// The estate as the caller named it, for the lines that say what to open.
    pub estate: String,
    /// The directory the server may work under: the estate's own, absolute.
    pub root: String,
    /// The ceiling, always written out.
    pub allow: String,
    /// The binary the client starts: the satz that produced this configuration.
    pub command: String,
    /// Where `--write` puts it.
    pub file: PathBuf,
    server: Server,
}

impl ClientConfig {
    /// The block as it is printed and as it is written: one server, pretty JSON.
    pub(crate) fn block(&self) -> String {
        let mut servers = BTreeMap::new();
        servers.insert(self.key.clone(), self.server.clone());
        let json = serde_json::to_string_pretty(&Block { servers })
            .expect("a server entry of strings serializes");
        format!("{json}\n")
    }
}

/// Where Claude Desktop reads its servers from, per platform.
///
/// One file for every server a person has, which is why satz's key names the
/// estate: a second estate's server is a second key in the same object.
fn desktop_config_path() -> Result<PathBuf, String> {
    let home = crate::settings::home_dir()
        .ok_or("no home directory: set HOME (USERPROFILE on Windows), or name the file with --file")?;
    let path = if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("Claude")
    } else if cfg!(windows) {
        match std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
            Some(appdata) => PathBuf::from(appdata).join("Claude"),
            None => home.join("AppData").join("Roaming").join("Claude"),
        }
    } else {
        home.join(".config").join("Claude")
    };
    Ok(path.join("claude_desktop_config.json"))
}

/// The key one estate's server is written under in a file that holds many.
///
/// The estate as an operator names it — `C0example.satz`, or `pk/C0example.satz` for
/// one in a subdirectory — with the extension dropped and anything a server name may
/// not carry replaced by a dash. Two estates of the same name under two roots are the
/// one collision this cannot see; the write refuses on the key that is already there
/// and `--name` is the way past it.
fn derive_key(client: Client, typed: &str) -> Result<String, String> {
    if client == Client::ClaudeCode {
        return Ok("satz".to_string());
    }
    let stem = typed.strip_suffix(".satz").unwrap_or(typed);
    let mut slug = String::new();
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        return Err(format!(
            "mcp-config: {typed} gives no server name Claude Desktop can hold — name one with --name"
        ));
    }
    Ok(format!("satz-{slug}"))
}

/// Work out the configuration: the binary, the root, the ceiling, the key, the file.
///
/// `command` is the satz that is running. A client starts a process, not a shell, so
/// the absolute path is what makes the server start at all where the client's `PATH`
/// is not the terminal's — which is every desktop application on macOS.
pub(crate) fn plan(
    client: Client,
    estate: &str,
    root: &Path,
    allow: Level,
    command: &Path,
    name: Option<&str>,
    file: Option<&Path>,
) -> Result<ClientConfig, String> {
    let key = match name {
        Some(n) if n.trim().is_empty() => return Err("--name is empty".into()),
        Some(n) => n.to_string(),
        None => derive_key(client, estate)?,
    };
    let root = crate::fsx::slash(root);
    let allow = allow.describe();
    let args = vec![
        "mcp".to_string(),
        "--root".to_string(),
        root.clone(),
        "--allow".to_string(),
        allow.clone(),
    ];
    let command = crate::fsx::slash(command);
    let file = match file {
        Some(f) => f.to_path_buf(),
        None => match client {
            Client::ClaudeCode => Path::new(&root).join(".mcp.json"),
            Client::ClaudeDesktop => desktop_config_path()?,
        },
    };
    Ok(ClientConfig {
        client,
        key,
        estate: estate.to_string(),
        root,
        allow,
        command: command.clone(),
        server: Server { transport: client.transport(), command, args },
        file,
    })
}

/// What the block cannot say, for the caller who is reading it rather than writing it.
pub(crate) fn notes(cfg: &ClientConfig) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "note: the client starts this binary by absolute path, so it needs no PATH of its own: {}\n",
        cfg.command
    ));
    out.push_str(&format!(
        "note: the server may work under {} and holds no estate until a call opens one — `satz_open` with {}\n",
        cfg.root, cfg.estate
    ));
    if cfg.client == Client::ClaudeCode {
        out.push_str(
            "note: Claude Code reads .mcp.json from the directory it starts in, and asks before it starts a server it has not seen\n",
        );
    }
    out.push_str(&format!("then: satz mcp-config {} --client {} --write   # {}\n", cfg.estate, cfg.client.as_str(), match cfg.client {
        Client::ClaudeCode => format!("writes {}", cfg.file.display()),
        Client::ClaudeDesktop => format!("merges the key into {}", cfg.file.display()),
    }));
    out
}

/// What `--write` did.
#[derive(Debug, PartialEq)]
pub(crate) enum Wrote {
    /// The file did not exist; satz created it holding its one server.
    Created,
    /// satz's key was added or replaced beside the servers already there.
    Merged { others: usize, replaced: bool },
    /// The key was already exactly this, so nothing was written.
    Unchanged,
}

/// A JSON document as the file holds it: every object keeps its keys in the order
/// they were written. `serde_json::Value` sorts them (the crate's `preserve_order`
/// feature would change that for every JSON satz emits), and a file a person keeps
/// by hand is written back in their order, not alphabetised.
#[derive(Debug, Clone, PartialEq)]
enum Doc {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Doc>),
    Object(Vec<(String, Doc)>),
}

impl Doc {
    fn get_mut(&mut self, key: &str) -> Option<&mut Doc> {
        match self {
            Doc::Object(kv) => kv.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Set `key`, in place where it is already there, last where it is not.
    fn set(&mut self, key: &str, value: Doc) {
        if let Doc::Object(kv) = self {
            match kv.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => kv.push((key.to_string(), value)),
            }
        }
    }

    /// The same value as `serde_json` reads it, to compare with an entry satz builds.
    fn value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a parsed document serializes")
    }
}

impl Serialize for Doc {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            Doc::Null => s.serialize_unit(),
            Doc::Bool(b) => s.serialize_bool(*b),
            Doc::Number(n) => n.serialize(s),
            Doc::String(t) => s.serialize_str(t),
            Doc::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for i in items {
                    seq.serialize_element(i)?;
                }
                seq.end()
            }
            Doc::Object(kv) => {
                let mut map = s.serialize_map(Some(kv.len()))?;
                for (k, v) in kv {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for Doc {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Doc, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = Doc;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E>(self) -> Result<Doc, E> {
                Ok(Doc::Null)
            }
            fn visit_bool<E>(self, b: bool) -> Result<Doc, E> {
                Ok(Doc::Bool(b))
            }
            fn visit_i64<E>(self, n: i64) -> Result<Doc, E> {
                Ok(Doc::Number(n.into()))
            }
            fn visit_u64<E>(self, n: u64) -> Result<Doc, E> {
                Ok(Doc::Number(n.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, n: f64) -> Result<Doc, E> {
                serde_json::Number::from_f64(n).map(Doc::Number).ok_or_else(|| E::custom("a number JSON cannot hold"))
            }
            fn visit_str<E>(self, t: &str) -> Result<Doc, E> {
                Ok(Doc::String(t.to_string()))
            }
            fn visit_string<E>(self, t: String) -> Result<Doc, E> {
                Ok(Doc::String(t))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Doc, A::Error> {
                let mut items = Vec::new();
                while let Some(i) = a.next_element()? {
                    items.push(i);
                }
                Ok(Doc::Array(items))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Doc, A::Error> {
                let mut obj = Doc::Object(Vec::new());
                // a repeated key keeps its first place and its last value, as a JSON reader does
                while let Some((k, v)) = a.next_entry::<String, Doc>()? {
                    obj.set(&k, v);
                }
                Ok(obj)
            }
        }
        d.deserialize_any(Visit)
    }
}

/// Write satz's key into the client's file, and nothing else.
///
/// satz owns one key. Every other key in the file — the other servers, and every
/// setting beside `mcpServers` — is read and written back as it was, in the order it
/// was written; satz's key keeps its place when it is replaced and goes last when it
/// is new. A file that is not JSON is refused rather than replaced — satz cannot
/// merge into what it cannot read, and `--force` is for a satz key that differs, not
/// for discarding a file whose contents are unknown.
pub(crate) fn write(cfg: &ClientConfig, force: bool) -> Result<Wrote, String> {
    let unreadable = |what: String| {
        format!(
            "{}: {what} — satz merges its own server into this file and does not replace one it cannot read. \
             Fix the file, or move it aside and run this again.",
            cfg.file.display()
        )
    };
    let entry = serde_json::to_value(&cfg.server).expect("a server entry of strings serializes");
    if !cfg.file.exists() {
        if let Some(parent) = cfg.file.parent() {
            crate::fsx::create_dir_all(parent).map_err(|e| format!("{}: {}", parent.display(), e))?;
        }
        crate::fsx::write(&cfg.file, cfg.block()).map_err(|e| format!("{}: {}", cfg.file.display(), e))?;
        return Ok(Wrote::Created);
    }
    let text = crate::fsx::read_to_string(&cfg.file).map_err(|e| format!("{}: {}", cfg.file.display(), e))?;
    let mut doc: Doc = serde_json::from_str(&text).map_err(|e| unreadable(format!("not valid JSON ({e})")))?;
    if !matches!(doc, Doc::Object(_)) {
        return Err(unreadable("its top level is not a JSON object".to_string()));
    }
    if doc.get_mut("mcpServers").is_none() {
        doc.set("mcpServers", Doc::Object(Vec::new()));
    }
    let servers = doc.get_mut("mcpServers").expect("set above");
    let Doc::Object(held) = &*servers else {
        return Err(unreadable("its `mcpServers` is not a JSON object".to_string()));
    };
    let held = held.len();
    let existing = servers.get_mut(&cfg.key).map(|v| v.value());
    let replaced = match existing {
        Some(existing) if existing == entry => return Ok(Wrote::Unchanged),
        Some(existing) => {
            if !force {
                return Err(format!(
                    "{} already holds the server \"{}\", with other arguments:\n\n{}\n\n\
                     --force replaces it. Every other server in the file is untouched either way.",
                    cfg.file.display(),
                    cfg.key,
                    serde_json::to_string_pretty(&existing).unwrap_or_else(|_| existing.to_string()),
                ));
            }
            true
        }
        None => false,
    };
    let others = held - usize::from(replaced);
    // through the text, not `entry`: a `serde_json::Value` has sorted the entry's keys
    let text = serde_json::to_string(&cfg.server).expect("a server entry of strings serializes");
    servers.set(&cfg.key, serde_json::from_str(&text).expect("a server entry reads back"));
    let json = serde_json::to_string_pretty(&doc).map_err(|e| format!("{}: {}", cfg.file.display(), e))?;
    crate::fsx::write(&cfg.file, format!("{json}\n")).map_err(|e| format!("{}: {}", cfg.file.display(), e))?;
    Ok(Wrote::Merged { others, replaced })
}

/// The one line a write leaves on the console: which key, which file, what else is there.
pub(crate) fn wrote(cfg: &ClientConfig, what: &Wrote) -> String {
    let where_ = format!("{} in {}", cfg.key, cfg.file.display());
    match what {
        Wrote::Created => format!("wrote {where_}: satz mcp --root {} --allow {}\n", cfg.root, cfg.allow),
        Wrote::Merged { others, replaced } => format!(
            "{} {where_}: satz mcp --root {} --allow {} ({} other server(s) untouched)\n",
            if *replaced { "replaced" } else { "added" },
            cfg.root,
            cfg.allow,
            others
        ),
        Wrote::Unchanged => format!("unchanged {where_}: it already runs satz mcp --allow {}\n", cfg.allow),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(spec: &str) -> Level {
        Level::parse(spec).expect("a level")
    }

    fn cfg(client: Client, root: &Path, file: Option<&Path>) -> ClientConfig {
        plan(
            client,
            "C0example.satz",
            root,
            level("read"),
            Path::new("/opt/bin/satz"),
            None,
            file,
        )
        .expect("a configuration")
    }

    fn block_of(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("the block is JSON")
    }

    #[test]
    fn the_claude_code_shape_is_the_mcp_json_claude_code_reads() {
        let c = cfg(Client::ClaudeCode, Path::new("/estates/acme"), None);
        assert_eq!(c.key, "satz");
        assert_eq!(c.file, Path::new("/estates/acme/.mcp.json"));
        let v = block_of(&c.block());
        let server = &v["mcpServers"]["satz"];
        assert_eq!(server["type"], "stdio");
        assert_eq!(server["command"], "/opt/bin/satz");
        assert_eq!(
            server["args"],
            serde_json::json!(["mcp", "--root", "/estates/acme", "--allow", "read"])
        );
    }

    #[test]
    fn the_claude_desktop_shape_is_keyed_per_estate_and_names_no_transport() {
        let c = cfg(Client::ClaudeDesktop, Path::new("/estates/acme"), Path::new("/tmp/desktop.json").into());
        assert_eq!(c.key, "satz-C0example");
        let v = block_of(&c.block());
        let server = &v["mcpServers"]["satz-C0example"];
        assert!(server.get("type").is_none(), "Claude Desktop's file carries command and args: {server}");
        assert_eq!(server["command"], "/opt/bin/satz");
    }

    /// The default is `read`, and a configuration that leaves it out hands an agent a
    /// read-only server without saying so — the next reader cannot tell it was chosen.
    #[test]
    fn the_ceiling_is_always_written_out() {
        for spec in ["read", "read,write", "read,write,exec", "exec"] {
            for client in [Client::ClaudeCode, Client::ClaudeDesktop] {
                let c = plan(
                    client,
                    "C0example.satz",
                    Path::new("/estates/acme"),
                    level(spec),
                    Path::new("/opt/bin/satz"),
                    None,
                    Path::new("/tmp/f.json").into(),
                )
                .expect("a configuration");
                let v = block_of(&c.block());
                let args = v["mcpServers"][&c.key]["args"].as_array().expect("args").clone();
                let args: Vec<&str> = args.iter().map(|a| a.as_str().expect("a string")).collect();
                let at = args.iter().position(|a| *a == "--allow").expect("--allow is written out");
                assert_eq!(args[at + 1], Level::parse(spec).expect("a level").describe());
            }
        }
    }

    /// A path with a space in it is why the args are a list rather than a command line.
    #[test]
    fn a_path_with_a_space_survives_the_round_trip() {
        let root = Path::new("/Users/someone/Cloud Estates/acme");
        let c = plan(
            Client::ClaudeCode,
            "C0example.satz",
            root,
            level("read"),
            Path::new("/Applications/satz tools/satz"),
            None,
            None,
        )
        .expect("a configuration");
        let v = block_of(&c.block());
        let server = &v["mcpServers"]["satz"];
        assert_eq!(server["command"], "/Applications/satz tools/satz");
        assert_eq!(server["args"][2], "/Users/someone/Cloud Estates/acme");
        assert_eq!(c.file, root.join(".mcp.json"));
    }

    #[test]
    fn a_name_of_its_own_wins_over_the_derived_one() {
        let c = plan(
            Client::ClaudeDesktop,
            "C0example.satz",
            Path::new("/estates/acme"),
            level("read"),
            Path::new("/opt/bin/satz"),
            Some("satz-second-org"),
            Path::new("/tmp/f.json").into(),
        )
        .expect("a configuration");
        assert_eq!(c.key, "satz-second-org");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-mcp-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    #[test]
    fn write_creates_the_file_and_is_idempotent() {
        let dir = scratch("create");
        let c = cfg(Client::ClaudeCode, &dir, None);
        assert_eq!(write(&c, false), Ok(Wrote::Created));
        let first = std::fs::read_to_string(&c.file).expect("the file");
        assert_eq!(block_of(&first), block_of(&c.block()));
        assert_eq!(write(&c, false), Ok(Wrote::Unchanged));
        assert_eq!(std::fs::read_to_string(&c.file).expect("the file"), first);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_satz_key_with_other_arguments_is_refused_until_force() {
        let dir = scratch("refuse");
        let read_only = cfg(Client::ClaudeCode, &dir, None);
        write(&read_only, false).expect("the first write");
        let writable = plan(
            Client::ClaudeCode,
            "C0example.satz",
            &dir,
            level("read,write"),
            Path::new("/opt/bin/satz"),
            None,
            None,
        )
        .expect("a configuration");
        let refusal = write(&writable, false).expect_err("a differing key is refused");
        assert!(refusal.contains(&writable.file.display().to_string()), "{refusal}");
        assert!(refusal.contains("\"satz\""), "{refusal}");
        assert!(refusal.contains("--force"), "{refusal}");
        // refused means written nowhere
        let on_disk = block_of(&std::fs::read_to_string(&writable.file).expect("the file"));
        assert_eq!(on_disk["mcpServers"]["satz"]["args"][4], "read");

        assert_eq!(write(&writable, true), Ok(Wrote::Merged { others: 0, replaced: true }));
        let on_disk = block_of(&std::fs::read_to_string(&writable.file).expect("the file"));
        assert_eq!(on_disk["mcpServers"]["satz"]["args"][4], "read,write");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// The Claude Desktop file holds every server a person has. satz owns one key of it.
    #[test]
    fn every_other_server_is_left_alone() {
        let dir = scratch("merge");
        let file = dir.join("claude_desktop_config.json");
        // keys in no sorted order, at every level: the file is written back in this order
        std::fs::write(
            &file,
            r#"{"theme":"dark","mcpServers":{"zeta":{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem","/tmp"],"env":{"Z":"1","A":"2"}},"alpha":{"command":"uvx","args":[],"port":8080.5}},"globalShortcut":"Alt+Space"}"#,
        )
        .expect("a desktop configuration");
        let c = cfg(Client::ClaudeDesktop, Path::new("/estates/acme"), Some(&file));
        assert_eq!(write(&c, false), Ok(Wrote::Merged { others: 2, replaced: false }));
        let expected = r#"{
  "theme": "dark",
  "mcpServers": {
    "zeta": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "/tmp"
      ],
      "env": {
        "Z": "1",
        "A": "2"
      }
    },
    "alpha": {
      "command": "uvx",
      "args": [],
      "port": 8080.5
    },
    "satz-C0example": {
      "command": "/opt/bin/satz",
      "args": [
        "mcp",
        "--root",
        "/estates/acme",
        "--allow",
        "read"
      ]
    }
  },
  "globalShortcut": "Alt+Space"
}
"#;
        assert_eq!(std::fs::read_to_string(&file).expect("the file"), expected, "written back in the file's own key order");

        // replaced, satz's key keeps its place
        let wider = plan(
            Client::ClaudeDesktop,
            "C0example.satz",
            Path::new("/estates/acme"),
            level("read,write"),
            Path::new("/opt/bin/satz"),
            None,
            Some(&file),
        )
        .expect("a configuration");
        assert_eq!(write(&wider, true), Ok(Wrote::Merged { others: 2, replaced: true }));
        assert_eq!(
            std::fs::read_to_string(&file).expect("the file"),
            expected.replace("\"read\"\n", "\"read,write\"\n")
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A file satz cannot read is a file satz does not replace: the other servers in
    /// it are unknown, and `--force` is for a satz key that differs.
    #[test]
    fn a_file_that_is_not_json_is_refused_and_force_does_not_help() {
        let dir = scratch("broken");
        let file = dir.join("claude_desktop_config.json");
        std::fs::write(&file, "{\"mcpServers\": {,}\n").expect("a broken file");
        let c = cfg(Client::ClaudeDesktop, Path::new("/estates/acme"), Some(&file));
        for force in [false, true] {
            let refusal = write(&c, force).expect_err("a file that is not JSON is refused");
            assert!(refusal.contains("not valid JSON"), "{refusal}");
            assert!(refusal.contains(&file.display().to_string()), "{refusal}");
        }
        assert_eq!(std::fs::read_to_string(&file).expect("the file"), "{\"mcpServers\": {,}\n");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
