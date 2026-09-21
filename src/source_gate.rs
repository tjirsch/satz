//! Reading satz's own source, for the gates that assert over it.
//!
//! Two tests judge the code as TEXT rather than by running it: the stdout gate in
//! `src/mcp.rs` (nothing a tool reaches may print to stdout, because stdout is the
//! protocol) and the binding gate in `src/main.rs` (every dispatch site that runs as an
//! estate's service account is declared in `BINDING_SITES`). Both must read the
//! production code and only the production code — a test module's `println!` is fine,
//! and a test's `configure_estate_impersonation(…)` is not a dispatch site.
//!
//! Both used to do that with `src.split("#[cfg(test)]").next()`, which stops at the FIRST
//! test module instead of skipping each one. In `adopt.rs` that module begins at line 143
//! of 1850, so a region named "src/adopt.rs" was 7% of the file: the resolver a tool
//! actually runs went unchecked while the test reported it covered.

/// `src` with every `#[cfg(test)]` item dropped, and everything else kept.
pub(crate) fn production_only(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("#[cfg(test)]") && !line.starts_with("#[cfg(all(test") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // a `#[cfg(test)] mod … {` runs to its closing brace at column 0; anything else
        // the attribute carries here (a `use`) is the single line that follows it
        match lines.peek() {
            Some(next) if next.trim_end().ends_with('{') => {
                lines.next();
                for l in lines.by_ref() {
                    if l == "}" {
                        break;
                    }
                }
            }
            _ => {
                lines.next();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::production_only;

    #[test]
    fn every_test_module_is_dropped_not_just_the_first() {
        let src = "fn a() {}\n#[cfg(test)]\nmod one {\n    fn t() {}\n}\nfn b() {}\n\
                   #[cfg(all(test, unix))]\nmod two {\n}\nfn c() {}\n";
        let out = production_only(src);
        assert!(out.contains("fn a()") && out.contains("fn b()") && out.contains("fn c()"));
        assert!(!out.contains("mod one") && !out.contains("mod two"));
    }

    #[test]
    fn a_cfg_test_use_takes_only_its_own_line() {
        let out = production_only("#[cfg(test)]\nuse std::path::PathBuf;\nfn a() {}\n");
        assert_eq!(out, "fn a() {}\n");
    }
}
