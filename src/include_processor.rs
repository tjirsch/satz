use std::path::{Path, PathBuf};

/// Key prefix used to rename `variables:` blocks from Form A included files.
/// Prevents duplicate top-level key errors when both the parent and included
/// file define a `variables:` block. The variable extractor recognises this prefix.
pub const INCLUDE_VARS_PREFIX: &str = "_satz_include_vars_";

/// Key prefix used to rename colliding top-level *resource-type* keys so the merged
/// document parses; `crate::merge_renamed_resource_keys` folds them back afterwards.
/// Purely a parser-evasion encoding — the merge semantics live in the fold.
pub const MERGE_KEY_PREFIX: &str = "_satz_merge_";

/// Resource-type names from the provider schemas, set once per process by the command
/// arms that know `schema_dir` (see `crate::init_resource_merge`). Deliberately NOT a
/// name-shape heuristic: when this is unset or empty, no cross-file merging happens and
/// duplicate keys keep their strict error. Merge behavior thus depends on exactly one
/// observable fact — schemas fetched — never on how a key is spelled.
static RESOURCE_TYPES: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();

/// Install the schema-derived resource-type set. First call wins; later calls are
/// no-ops (the set is identical for one config anyway).
pub fn set_resource_types(types: std::collections::HashSet<String>) {
    let _ = RESOURCE_TYPES.set(types);
}

/// Structural keys that must never merge across files even though the registry may
/// resolve them (`folder` -> google_folder): merging recursive hierarchy trees is a
/// different feature with different rules.
// `project` maps DO merge: their entries are named projects, so two `project`
// blocks under one folder union id-by-id (a preset bringing its own project can
// land in a folder that already has one). Same id + different content errors.
// `folder` stays strict — merging hierarchy trees is a different feature.
const NEVER_MERGE: &[&str] = &["folder", "variables", "terraform", "providers"];

fn is_mergeable_resource_key(key: &str) -> bool {
    let Some(types) = RESOURCE_TYPES.get() else {
        return false;
    };
    types.contains(key) || types.contains(&format!("google_{}", key))
}

/// The operation an include line requests. `!include` only inlines the file;
/// `!import-include` additionally marks the file so that `transpile` imports the resources
/// it contributes into state (see `crate::classify_import_binding` for the dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncludeOp {
    /// `!include` — inline only, no import side effects.
    Plain,
    /// `!import-include` — inline AND, at transpile time, look the included resources up
    /// live and `tofu import` the ones that already exist into state.
    Import,
}

/// A resolved `!import-include` discovered while expanding a config.
#[derive(Debug, Clone)]
/// `op`/`key` are read by the R1 material in `classify_import_binding`.
#[allow(dead_code)]
pub struct IncludeBinding {
    pub op: IncludeOp,
    pub path: PathBuf,
    /// The YAML key the included content is nested under (Form B). A nested Form A include
    /// inherits it from the enclosing include, because its content lands under that same
    /// key. `None` only for a bare Form A include at the top level of the main config.
    /// This is what tells `transpile` which kind of resource the preset contributes.
    pub key: Option<String>,
}

/// Expand all includes in `file_path`, returning the merged YAML text.
/// (Operation-tagged includes are inlined just like `!include`; use
/// `process_includes_with_ops` if you also need the operations manifest.)
pub fn process_includes(file_path: &Path, include_paths: &[PathBuf]) -> Result<String, Box<dyn std::error::Error>> {
    Ok(process_includes_with_ops(file_path, include_paths)?.0)
}

/// Like `process_includes`, but also returns the manifest of `!import-include` directives
/// encountered, in document order.
pub fn process_includes_with_ops(
    file_path: &Path,
    include_paths: &[PathBuf],
) -> Result<(String, Vec<IncludeBinding>), Box<dyn std::error::Error>> {
    let mut counter = 0usize;
    let mut bindings = Vec::new();
    let mut stack = Vec::new();
    let mut seen_anchors = std::collections::HashSet::new();
    let text = process_includes_inner(file_path, include_paths, &mut counter, &mut bindings, &mut stack, None, &mut seen_anchors)?;
    Ok((rename_duplicate_resource_keys(&text, &is_mergeable_resource_key), bindings))
}

/// Post-pass over the fully merged document: the second and later occurrence of a
/// resource-type key *within the same mapping* is renamed to `_satz_merge_<n>_<key>`
/// so the parse succeeds; the fold then merges the entries back id-by-id at that level.
/// Depth-aware: two presets included inside the same folder block collide one level
/// down, not at the document top. Runs on the final text so main-file keys and any mix
/// of includes are handled uniformly, in document order.
fn rename_duplicate_resource_keys(text: &str, is_resource: &dyn Fn(&str) -> bool) -> String {
    // Open mapping keys above the current line, innermost last — the parent path.
    let mut path: Vec<(usize, String)> = Vec::new();
    // (parent path, indent, key) triples already seen.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut n = 0usize;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some((indent, key)) = bare_mapping_key(line) {
            while path.last().is_some_and(|(i, _)| *i >= indent) {
                path.pop();
            }
            let parent: Vec<&str> = path.iter().map(|(_, k)| k.as_str()).collect();
            let slot = format!("{}\u{0}{}\u{0}{}", parent.join("."), indent, key);
            // Structural keys never merge regardless of what the caller's check says —
            // the registry resolves `folder`/`project` to real resource types, but
            // merging hierarchy trees is a different feature.
            let never = key.starts_with("_satz_") || NEVER_MERGE.contains(&key.as_str());
            if !seen.insert(slot) && !never && is_resource(&key) {
                let renamed = format!("{}{}_{}", MERGE_KEY_PREFIX, n, key);
                out.push(format!("{}{}:", " ".repeat(indent), renamed));
                n += 1;
                path.push((indent, renamed));
                continue;
            }
            path.push((indent, key));
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

/// Record every `&name` anchor a line defines, so later lines can ask "was this anchor
/// already defined above me?". Comment lines are prose, not YAML.
pub(crate) fn register_anchors(line: &str, seen: &mut std::collections::HashSet<String>) {
    if line.trim_start().starts_with('#') {
        return;
    }
    for (i, _) in line.match_indices('&') {
        let rest = &line[i + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
            .unwrap_or(rest.len());
        if end > 0
            && rest[..end]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            seen.insert(rest[..end].to_string());
        }
    }
}

/// A `variables:` entry line: `  <key>: &<anchor> <value>` → (indent, key, anchor, rest).
pub(crate) fn parse_vars_entry(line: &str) -> Option<(usize, &str, &str, &str)> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if indent == 0 || trimmed.starts_with('#') {
        return None;
    }
    let colon = trimmed.find(':')?;
    let key = trimmed[..colon].trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return None;
    }
    let after = trimmed[colon + 1..].trim_start();
    let anchor_rest = after.strip_prefix('&')?;
    let end = anchor_rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .unwrap_or(anchor_rest.len());
    if end == 0 {
        return None;
    }
    Some((indent, key, &anchor_rest[..end], &anchor_rest[end..]))
}

fn process_includes_inner(
    file_path: &Path,
    include_paths: &[PathBuf],
    counter: &mut usize,
    bindings: &mut Vec<IncludeBinding>,
    stack: &mut Vec<PathBuf>,
    // The key `file_path`'s own content is nested under, so that a Form A include inside
    // it reports the key its content actually ends up under rather than `None`.
    enclosing_key: Option<&str>,
    // Anchors defined so far in the merged document, in emission order. Drives two
    // things: `!include-if <anchor> <file>` (include only if defined), and
    // first-definition-wins for preset defaults (see the variables handling below).
    seen_anchors: &mut std::collections::HashSet<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    // Identify each file by its canonical path so `./a.yaml` and `a.yaml` are the same
    // node, then refuse to re-enter a file already open further up the chain. Without
    // this a file that includes itself recurses until the stack overflows, which aborts
    // the process instead of producing an error the caller can report.
    // Canonicalisation can fail (broken symlink, permissions); fall back to the path as
    // given and let the read below surface the real reason.
    let key = std::fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
    if let Some(start) = stack.iter().position(|open| open == &key) {
        let mut chain: Vec<String> = stack[start..].iter().map(|p| p.display().to_string()).collect();
        chain.push(key.display().to_string());
        return Err(format!("Include cycle detected: {}", chain.join(" -> ")).into());
    }
    stack.push(key);

    let content = crate::fsx::read_to_string(file_path)?;
    let mut result = Vec::new();
    let parent_dir = file_path.parent().unwrap_or(Path::new("."));
    // Open mapping keys above the current line, innermost last. Lets a Form A include
    // written *under* a key (`org_policy_policy:` on its own line, directive indented
    // beneath it) report that key, which is how the directive is most naturally written.
    let mut key_stack: Vec<(usize, String)> = Vec::new();
    // Only included files get their variables-block defaults overridden; the root file
    // is the overrider, never the overridden.
    let is_included_file = stack.len() > 1;
    let mut in_vars_block = false;

    for line in content.lines() {
        if let Some(caps) = find_include(line) {
            let (indent, key, include_file, op, condition) = caps;

            // `!include-if <anchor> <file>`: only include when the anchor is already
            // defined above this line. The file does not even need to exist otherwise —
            // a template can carry optional parts its consumers never resolve.
            if let Some(cond) = condition {
                if !seen_anchors.contains(cond) {
                    result.push(format!(
                        "{}# satz:skipped: !include-if {} — anchor not defined: {}",
                        " ".repeat(indent),
                        cond,
                        include_file
                    ));
                    continue;
                }
            }

            let resolved_path = resolve_include_path(parent_dir, include_file, include_paths)
                .ok_or_else(|| format!("Could not resolve include file: {}", include_file))?;

            // Form B names the key on the same line; otherwise the content lands under the
            // nearest enclosing key of this file, or failing that under whatever key this
            // whole file was included beneath.
            let effective_key = key
                .map(|k| k.to_string())
                .or_else(|| {
                    key_stack
                        .iter()
                        .rev()
                        .find(|(i, _)| *i < indent)
                        .map(|(_, k)| k.clone())
                })
                .or_else(|| enclosing_key.map(|k| k.to_string()));

            // Every include is recorded — consumers filter by op. Plain entries give
            // commands like check-presets the resolved path of each included file.
            bindings.push(IncludeBinding {
                op,
                path: resolved_path.clone(),
                key: effective_key.clone(),
            });

            let included_content = process_includes_inner(
                &resolved_path,
                include_paths,
                counter,
                bindings,
                stack,
                effective_key.as_deref(),
                seen_anchors,
            )?;

            let content_indent = if key.is_some() { indent + 2 } else { indent };
            let prefix = " ".repeat(content_indent);

            if let Some(key_str) = key {
                // Form B: content is indented under a key — no top-level key conflicts possible
                result.push(format!("{}{}:", " ".repeat(indent), key_str));
                for inc_line in included_content.lines() {
                    if inc_line.trim().is_empty() {
                        result.push(String::new());
                    } else {
                        result.push(format!("{}{}", prefix, inc_line));
                    }
                }
            } else {
                // Form A: content is inserted at the same indent level as the parent.
                // Rename any top-level `variables:` block in the included file to a unique
                // internal key to prevent duplicate-key errors when both files define variables.
                let idx = *counter;
                *counter += 1;
                let renamed = rename_top_level_variables(&included_content, idx);

                // Source annotation is visible in YAML error context output
                result.push(format!("# satz:source: {}", resolved_path.display()));
                for inc_line in renamed.lines() {
                    if inc_line.trim().is_empty() {
                        result.push(String::new());
                    } else {
                        result.push(format!("{}{}", prefix, inc_line));
                    }
                }
                result.push(format!("# satz:source-end: {}", resolved_path.display()));
            }
        } else {
            if let Some((indent, key)) = bare_mapping_key(line) {
                while key_stack.last().is_some_and(|(i, _)| *i >= indent) {
                    key_stack.pop();
                }
                key_stack.push((indent, key));
            }

            // Track whether we are inside this file's top-level `variables:` block.
            let trimmed = line.trim_start();
            let at_top_level = !line.starts_with(' ') && !trimmed.is_empty() && !trimmed.starts_with('#');
            if at_top_level {
                in_vars_block = trimmed == "variables:";
            }

            // First definition wins: a preset's variables block provides *defaults*. If
            // the including document already defined the anchor, strip the preset's
            // redefinition so the preset's own aliases resolve to the outer value —
            // otherwise YAML's nearest-preceding-anchor rule makes defaults unoverridable.
            if is_included_file && in_vars_block && !at_top_level {
                if let Some((indent, key, anchor, rest)) = parse_vars_entry(line) {
                    if seen_anchors.contains(anchor) {
                        result.push(format!("{}{}:{}", " ".repeat(indent), key, rest));
                        continue;
                    }
                }
            }

            register_anchors(line, seen_anchors);
            result.push(line.to_string());
        }
    }

    stack.pop();
    Ok(result.join("\n"))
}

/// Renames the top-level `variables:` key in an included file's content to a
/// unique internal key so it can coexist with the parent file's `variables:` block.
fn rename_top_level_variables(content: &str, idx: usize) -> String {
    let new_key = format!("{}{}", INCLUDE_VARS_PREFIX, idx);
    content.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if indent == 0 && (trimmed == "variables:" || trimmed.starts_with("variables: ")) {
                let rest = &trimmed["variables".len()..];
                format!("{}{}", new_key, rest)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Recognized include directives. `!include` and `!import-include` are distinct prefixes,
/// so match order does not matter.
const DIRECTIVES: &[(&str, IncludeOp)] = &[
    ("!include ", IncludeOp::Plain),
    ("!import-include ", IncludeOp::Import),
];

fn match_directive(s: &str) -> Option<(IncludeOp, &str)> {
    for (token, op) in DIRECTIVES {
        if let Some(rest) = s.strip_prefix(token) {
            let filename = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            return Some((*op, filename));
        }
    }
    None
}

/// `!include-if <anchor> <file>` — a plain include gated on the anchor being defined
/// earlier in the document. The anchor may be written bare or as `*name`/`&name`.
fn match_conditional(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("!include-if ")?.trim();
    let mut parts = rest.splitn(2, char::is_whitespace);
    let cond = parts.next()?.trim_start_matches(['*', '&']);
    let file = parts.next()?.trim().trim_matches(|c| c == '"' || c == '\'');
    if cond.is_empty() || file.is_empty() {
        return None;
    }
    Some((cond, file))
}

/// (indent, Form B key, file, op, include-if condition)
type FoundInclude<'a> = (usize, Option<&'a str>, &'a str, IncludeOp, Option<&'a str>);

fn find_include(line: &str) -> Option<FoundInclude<'_>> {
    let trimmed = line.trim_start();
    // A commented-out or documented include is not an include. Without this, a preset whose
    // header shows how to include it inlines *itself* and is reported as a cycle.
    if trimmed.starts_with('#') {
        return None;
    }
    let indent = line.len() - trimmed.len();

    // Form A: <directive> file.yaml   (`!include-if` first: "!include " is not its prefix,
    // but checking it first keeps the intent obvious)
    if let Some((cond, filename)) = match_conditional(trimmed) {
        return Some((indent, None, filename, IncludeOp::Plain, Some(cond)));
    }
    if let Some((op, filename)) = match_directive(trimmed) {
        return Some((indent, None, filename, op, None));
    }

    // Form B: key: <directive> file.yaml
    if let Some(colon_pos) = trimmed.find(':') {
        let key = trimmed[..colon_pos].trim();
        let rest = trimmed[colon_pos + 1..].trim();
        if let Some((cond, filename)) = match_conditional(rest) {
            return Some((indent, Some(key), filename, IncludeOp::Plain, Some(cond)));
        }
        if let Some((op, filename)) = match_directive(rest) {
            return Some((indent, Some(key), filename, op, None));
        }
    }

    None
}

/// A line that opens a nested mapping — `some_key:` and nothing else. Deliberately strict:
/// sequence items, inline values and comments are not keys anything gets nested under, and
/// a wrong guess here would misroute an import rather than merely fail to help.
fn bare_mapping_key(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let indent = line.len() - trimmed.len();
    let key = trimmed.strip_suffix(':')?.trim_end();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return None;
    }
    Some((indent, key.to_string()))
}

fn resolve_include_path(current_dir: &Path, include_file: &str, search_paths: &[PathBuf]) -> Option<PathBuf> {
    // 1. Try relative to current file
    let rel_path = current_dir.join(include_file);
    if rel_path.exists() {
        return Some(rel_path);
    }

    // 2. Try search paths
    for path in search_paths {
        let abs_path = path.join(include_file);
        if abs_path.exists() {
            return Some(abs_path);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_import_directives() {
        assert_eq!(find_include("!include a.yaml").unwrap().3, IncludeOp::Plain);
        assert_eq!(find_include("!import-include a.yaml").unwrap().3, IncludeOp::Import);
        assert_eq!(
            find_include("  !import-include a.yaml").unwrap().3,
            IncludeOp::Import
        );
        let (_indent, key, file, op, cond) = find_include("key: !import-include \"p.yaml\"").unwrap();
        assert_eq!(key, Some("key"));
        assert_eq!(file, "p.yaml");
        assert_eq!(op, IncludeOp::Import);
        assert_eq!(cond, None);
    }

    #[test]
    fn parses_conditional_includes() {
        let (_i, key, file, op, cond) = find_include("!include-if logsink-project-name logsink.yaml").unwrap();
        assert_eq!((key, file, op, cond), (None, "logsink.yaml", IncludeOp::Plain, Some("logsink-project-name")));

        // Anchor sigils are tolerated, Form B works, and a plain !include has no condition.
        let (_i, _k, _f, _op, cond) = find_include("!include-if *logsink-project-name logsink.yaml").unwrap();
        assert_eq!(cond, Some("logsink-project-name"));
        let (_i, key, file, _op, cond) = find_include("audit: !include-if logsink-name \"l.yaml\"").unwrap();
        assert_eq!((key, file, cond), (Some("audit"), "l.yaml", Some("logsink-name")));
        assert_eq!(find_include("!include a.yaml").unwrap().4, None);
    }

    #[test]
    fn non_include_lines_ignored() {
        assert!(find_include("name: foo").is_none());
        assert!(find_include("# just a comment").is_none());
    }

    #[test]
    fn commented_out_includes_are_not_includes() {
        // Presets document their own usage in a header comment; treating that as a real
        // include makes the file include itself.
        assert!(find_include("# !include other.yaml").is_none());
        assert!(find_include("#   cloud_identity_group: !include self.yaml").is_none());
        assert!(find_include("  # org_policy_policy: !import-include self.yaml").is_none());
    }

    /// Scratch directory unique to this process and test name, so tests that write
    /// real files stay safe under `cargo test`'s parallel harness.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-inc-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn self_include_is_reported_as_cycle() {
        let dir = scratch("self");
        let a = dir.join("a.yaml");
        std::fs::write(&a, "!include a.yaml\n").unwrap();

        let err = process_includes(&a, &[]).expect_err("a self-include must not recurse forever");
        let msg = err.to_string();
        assert!(msg.contains("Include cycle detected"), "unexpected error: {msg}");
        assert!(msg.contains("a.yaml"), "cycle should name the file: {msg}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_file_cycle_is_reported() {
        let dir = scratch("two");
        let a = dir.join("a.yaml");
        let b = dir.join("b.yaml");
        std::fs::write(&a, "!include b.yaml\n").unwrap();
        std::fs::write(&b, "!include a.yaml\n").unwrap();

        let err = process_includes(&a, &[]).expect_err("an a -> b -> a cycle must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("Include cycle detected"), "unexpected error: {msg}");
        assert!(msg.contains("a.yaml") && msg.contains("b.yaml"), "cycle should name both files: {msg}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The main file's value must win over a preset's default: YAML's own rule is
    /// nearest-preceding-anchor, which would make preset defaults unoverridable.
    #[test]
    fn main_file_anchor_overrides_included_default() {
        let dir = scratch("override");
        let main = dir.join("main.yaml");
        let preset = dir.join("preset.yaml");
        std::fs::write(
            &preset,
            "variables:\n  size: &size \"default\"\n  color: &color \"blue\"\nthing:\n  a: *size\n  b: *color\n",
        )
        .unwrap();
        std::fs::write(
            &main,
            "variables:\n  size: &size \"overridden\"\n!include preset.yaml\n",
        )
        .unwrap();

        let out = process_includes(&main, &[]).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let parsed: serde_yaml::Value = serde_yaml::from_str(&out).expect("merged doc parses");
        // `size` was defined by main first -> preset's alias resolves to main's value;
        // `color` only exists in the preset -> its default survives.
        assert_eq!(parsed["thing"]["a"].as_str(), Some("overridden"), "merged:\n{out}");
        assert_eq!(parsed["thing"]["b"].as_str(), Some("blue"), "merged:\n{out}");
    }

    /// The root file is the overrider, never the overridden: its own variables keep
    /// their anchors even if an earlier include defined the same name.
    #[test]
    fn root_file_variables_are_never_stripped() {
        let dir = scratch("rootkeep");
        let main = dir.join("main.yaml");
        std::fs::write(dir.join("first.yaml"), "variables:\n  x: &x \"from-include\"\n").unwrap();
        std::fs::write(
            &main,
            "!include first.yaml\nvariables:\n  x: &x \"from-main\"\nuse: *x\n",
        )
        .unwrap();
        let out = process_includes(&main, &[]).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let parsed: serde_yaml::Value = serde_yaml::from_str(&out).expect("parses");
        assert_eq!(parsed["use"].as_str(), Some("from-main"), "merged:\n{out}");
    }

    #[test]
    fn include_if_skips_when_anchor_undefined_and_includes_when_defined() {
        let dir = scratch("condinc");
        let main_on = dir.join("on.yaml");
        let main_off = dir.join("off.yaml");
        std::fs::write(dir.join("optional.yaml"), "extra: present\n").unwrap();
        std::fs::write(
            &main_on,
            "variables:\n  logsink-project: &logsink-project \"p\"\n!include-if logsink-project optional.yaml\n",
        )
        .unwrap();
        std::fs::write(&main_off, "base: yes\n!include-if logsink-project optional.yaml\n").unwrap();

        let on = process_includes(&main_on, &[]).unwrap();
        let off = process_includes(&main_off, &[]).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(on.contains("extra: present"), "defined anchor includes:\n{on}");
        assert!(!off.contains("extra: present"), "undefined anchor skips:\n{off}");
        assert!(off.contains("satz:skipped"), "skip leaves a trace:\n{off}");
    }

    /// A skipped conditional include must not require the file to exist — templates may
    /// carry optional parts their consumers never resolve.
    #[test]
    fn skipped_include_if_tolerates_missing_file() {
        let dir = scratch("condmissing");
        let main = dir.join("main.yaml");
        std::fs::write(&main, "base: yes\n!include-if not-defined nowhere.yaml\n").unwrap();
        let out = process_includes(&main, &[]).expect("missing file is fine when skipped");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.contains("base: yes"));
    }

    #[test]
    fn duplicate_resource_keys_are_renamed_in_document_order() {
        let is_resource = |k: &str| k == "google_logging_organization_sink";
        let text = "a: 1\ngoogle_logging_organization_sink:\n  s1:\n    name: x\ngoogle_logging_organization_sink:\n  s2:\n    name: y\n";
        let out = rename_duplicate_resource_keys(text, &is_resource);
        assert!(out.contains("_satz_merge_0_google_logging_organization_sink:"), "{out}");
        // exactly the first occurrence keeps its name; result parses
        assert_eq!(out.matches("\ngoogle_logging_organization_sink:").count(), 1);
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&out).is_ok(), "{out}");
    }

    #[test]
    fn non_resource_and_structural_keys_are_never_renamed() {
        let is_resource = |_: &str| true; // even if the registry resolved them
        for key in ["folder", "terraform", "providers"] {
            let text = format!("{key}:\n  a: 1\n{key}:\n  b: 2\n");
            let out = rename_duplicate_resource_keys(&text, &is_resource);
            assert!(!out.contains(MERGE_KEY_PREFIX), "{key} must stay strict: {out}");
        }
        // unknown key without registry backing stays strict too
        let out = rename_duplicate_resource_keys("x_y:\n  a: 1\nx_y:\n  b: 2\n", &|_| false);
        assert!(!out.contains(MERGE_KEY_PREFIX), "{out}");
    }

    #[test]
    fn sibling_project_maps_in_a_folder_are_renamed_for_the_fold() {
        // a preset included inside a folder brings its own `project` block; it must
        // union with the folder's existing one (distinct ids) via the merge fold
        let is_resource = |k: &str| k == "project";
        let text = "folder:\n  f:\n    project:\n      infra:\n        a: 1\n    project:\n      logsink:\n        b: 2\n";
        let out = rename_duplicate_resource_keys(text, &is_resource);
        assert!(out.contains("_satz_merge_0_project:"), "{out}");
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&out).is_ok(), "{out}");
    }

    #[test]
    fn form_b_key_recorded_in_binding() {
        // The wrapper key is what tells `transpile` which importer to run, so it has to
        // survive into the manifest rather than being consumed by the inliner.
        let dir = scratch("formb");
        let main = dir.join("main.yaml");
        std::fs::write(&main, "cloud_identity_group: !import-include groups.yaml\n").unwrap();
        std::fs::write(dir.join("groups.yaml"), "admins:\n  display_name: A\n").unwrap();

        let (_text, bindings) = process_includes_with_ops(&main, &[]).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].key.as_deref(), Some("cloud_identity_group"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_form_a_inherits_enclosing_key() {
        // A Form A include nested inside a Form B one has no key of its own, but its
        // content still lands under the outer key — reporting `None` would misroute it.
        let dir = scratch("nested");
        let main = dir.join("main.yaml");
        std::fs::write(&main, "cloud_identity_group: !include outer.yaml\n").unwrap();
        std::fs::write(dir.join("outer.yaml"), "!import-include inner.yaml\n").unwrap();
        std::fs::write(dir.join("inner.yaml"), "admins:\n  display_name: A\n").unwrap();

        let (_text, bindings) = process_includes_with_ops(&main, &[]).unwrap();
        let imports: Vec<_> = bindings.iter().filter(|b| b.op == IncludeOp::Import).collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].key.as_deref(), Some("cloud_identity_group"));
        // The plain outer include is recorded too, with its resolved path.
        assert_eq!(bindings.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn form_a_indented_under_a_key_reports_that_key() {
        // The directive is most naturally written on its own line beneath the key rather
        // than inline after it; both spellings must route to the same importer.
        let dir = scratch("underkey");
        let main = dir.join("main.yaml");
        std::fs::write(&main, "cloud_identity_group:\n  !import-include groups.yaml\n").unwrap();
        std::fs::write(dir.join("groups.yaml"), "admins:\n  display_name: A\n").unwrap();

        let (_text, bindings) = process_includes_with_ops(&main, &[]).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].key.as_deref(), Some("cloud_identity_group"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sibling_key_above_does_not_capture_a_top_level_include() {
        // A key that closed before the include starts must not claim it — the include is
        // its sibling, not its child.
        let dir = scratch("sibling");
        let main = dir.join("main.yaml");
        std::fs::write(&main, "org_policy_policy:\n  foo:\n    name: x\n!import-include p.yaml\n").unwrap();
        std::fs::write(dir.join("p.yaml"), "bar: baz\n").unwrap();

        let (_text, bindings) = process_includes_with_ops(&main, &[]).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].key, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn top_level_form_a_has_no_key() {
        let dir = scratch("forma");
        let main = dir.join("main.yaml");
        std::fs::write(&main, "!import-include p.yaml\n").unwrap();
        std::fs::write(dir.join("p.yaml"), "foo: bar\n").unwrap();

        let (_text, bindings) = process_includes_with_ops(&main, &[]).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].key, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_file_included_twice_is_not_a_cycle() {
        // Guards the cycle check against over-reach: two sibling includes of one file
        // are legitimate, only re-entering a file still open above is a cycle.
        let dir = scratch("diamond");
        let main = dir.join("main.yaml");
        let leaf = dir.join("leaf.yaml");
        std::fs::write(&main, "!include leaf.yaml\n!include leaf.yaml\n").unwrap();
        std::fs::write(&leaf, "foo: bar\n").unwrap();

        let out = process_includes(&main, &[]).expect("sibling includes are not a cycle");
        assert_eq!(out.matches("foo: bar").count(), 2, "both includes should be inlined:\n{out}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
