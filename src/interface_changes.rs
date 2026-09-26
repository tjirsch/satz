//! `CHANGES.md` beside an interface's README: what the last transpile changed in it, as
//! the todo a project follows. It is the previous `satz/interface.satz` of the interface,
//! read from disk before satz rewrites `interfaces/`, diffed against the new one — so no
//! contract version and no pack provenance is needed: a rename is a removed output and an
//! added one with the same targets and shape, and everything else is what the two files
//! say. One transpile is one step; the estate's history of the file is the guide across
//! several, which is why nothing accumulates here.

use std::collections::BTreeMap;
use std::path::Path;

use satz_core::satz::{InterfaceFile, OfferedLookup, OfferedOutput};

/// Beside an interface's `README.md`, when the last transpile changed it.
pub(crate) const FILE: &str = "CHANGES.md";

/// The interface files under `interfaces_dir` as the last transpile left them, by
/// interface name — every folder carries the same copy of an interface, so the first
/// found is the one. An absent directory is no previous state. A file that does not
/// parse is refused: satz wrote it and rewrites the directory whole, so a hand edit or a
/// file an older satz wrote is removed, not read around.
pub(crate) fn previous(interfaces_dir: &Path) -> Result<BTreeMap<String, InterfaceFile>, String> {
    let mut out = BTreeMap::new();
    if !interfaces_dir.is_dir() {
        return Ok(out);
    }
    let dirs = |d: &Path| -> Result<Vec<std::path::PathBuf>, String> {
        let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(d)
            .map_err(|e| format!("{}: {}", d.display(), e))?
            .map(|e| e.map(|e| e.path()).map_err(|e| format!("{}: {}", d.display(), e)))
            .collect::<Result<_, _>>()?;
        v.retain(|p| p.is_dir());
        v.sort();
        Ok(v)
    };
    for folder in dirs(interfaces_dir)? {
        for m in dirs(&folder)? {
            let name = m.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            let p = m.join(crate::interface::SATZ_DIR).join(crate::interface::SATZ_FILE);
            if name.is_empty() || out.contains_key(&name) || !p.is_file() {
                continue;
            }
            let text = crate::fsx::read_to_string(&p).map_err(|e| e.to_string())?;
            let file = satz_core::satz::parse(&text)
                .map_err(|e| {
                    format!(
                        "{}:{}: {} — satz wrote this file and rewrites `{}` whole; remove the directory to transpile without a CHANGES.md",
                        p.display(),
                        e.line,
                        e.msg,
                        interfaces_dir.display()
                    )
                })?
                .interface_file
                .ok_or_else(|| format!("{}: no interface file — satz wrote this path and rewrites `{}` whole; remove the directory", p.display(), interfaces_dir.display()))?;
            out.insert(name, file);
        }
    }
    Ok(out)
}

/// One line of the file: something a project must do, or something it may want to know.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub todo: bool,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Shape {
    Text,
    List,
    Map,
}

impl Shape {
    fn of(v: &serde_yaml::Value) -> Shape {
        match v {
            serde_yaml::Value::Sequence(_) => Shape::List,
            serde_yaml::Value::Mapping(_) => Shape::Map,
            _ => Shape::Text,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Shape::Text => "single value",
            Shape::List => "list",
            Shape::Map => "map",
        }
    }
}

/// Every string in a value, however deeply nested.
fn strings<'a>(v: &'a serde_yaml::Value, out: &mut Vec<&'a str>) {
    match v {
        serde_yaml::Value::String(s) => out.push(s),
        serde_yaml::Value::Sequence(seq) => seq.iter().for_each(|i| strings(i, out)),
        serde_yaml::Value::Mapping(m) => m.values().for_each(|i| strings(i, out)),
        _ => {}
    }
}

/// The lookups a value reads, in the order they are named.
fn lookups_of<'a>(o: &OfferedOutput, file: &'a InterfaceFile) -> Vec<&'a OfferedLookup> {
    let mut texts = Vec::new();
    strings(&o.value, &mut texts);
    let mut out: Vec<&OfferedLookup> = Vec::new();
    for t in texts {
        for l in &file.lookups {
            if t.contains(&format!("${{{}.", l.address)) && !out.iter().any(|x| x.address == l.address) {
                out.push(l);
            }
        }
    }
    out
}

fn looked_up(o: &OfferedOutput) -> bool {
    let mut texts = Vec::new();
    strings(&o.value, &mut texts);
    texts.iter().any(|t| t.contains("${data."))
}

fn map_keys(v: &serde_yaml::Value) -> Vec<String> {
    match v {
        serde_yaml::Value::Mapping(m) => m.keys().filter_map(|k| k.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

/// How a project reads `name`, in both forms.
fn read_as(name: &str) -> String {
    format!("`module.satz.{}` / `${{{{interface.{}}}}}`", name, name)
}

fn quoted(items: &[String]) -> String {
    items.iter().map(|i| format!("`{}`", i)).collect::<Vec<_>>().join(", ")
}

/// What changed from `old` to `new`, todos first, each in the order the outputs stand
/// in the new file (removed ones after, in the old file's order).
pub(crate) fn changes(old: &InterfaceFile, new: &InterfaceFile) -> Vec<Change> {
    let mut todo: Vec<String> = Vec::new();
    let mut info: Vec<String> = Vec::new();
    let removed: Vec<&OfferedOutput> = old.outputs.iter().filter(|o| !new.outputs.iter().any(|n| n.name == o.name)).collect();
    let mut added: Vec<&OfferedOutput> = new.outputs.iter().filter(|n| !old.outputs.iter().any(|o| o.name == n.name)).collect();
    // a rename: the same resources, the same shape, a new name — one added output serves one removed
    for o in &removed {
        let renamed = (!o.targets.is_empty()).then(|| added.iter().position(|n| n.targets == o.targets && Shape::of(&n.value) == Shape::of(&o.value))).flatten();
        match renamed {
            Some(i) => {
                let n = added.remove(i);
                todo.push(format!("Replace {} with `…{}`: renamed, the same {}.", read_as(&o.name), n.name, quoted(&o.targets)));
            }
            None => {
                let named = if o.targets.is_empty() { String::new() } else { format!(" It named {}.", quoted(&o.targets)) };
                todo.push(format!("`{}` is gone: the interface no longer publishes it, so {} fails at plan.{}", o.name, read_as(&o.name), named));
            }
        }
    }
    for n in &new.outputs {
        let Some(o) = old.outputs.iter().find(|o| o.name == n.name) else { continue };
        let (os, ns) = (Shape::of(&o.value), Shape::of(&n.value));
        if os != ns {
            todo.push(format!("`{}` is now a {} (was a {}).", n.name, ns.name(), os.name()));
        }
        for t in o.attach.iter().filter(|t| !n.attach.contains(t)) {
            todo.push(format!("`{}` no longer takes `{}`: an attachment of that type is refused.", n.name, t));
        }
        for t in n.attach.iter().filter(|t| !o.attach.contains(t)) {
            info.push(format!("`{}` now takes `{}`.", n.name, t));
        }
        match (looked_up(o), looked_up(n)) {
            (false, true) => {
                let reads = lookups_of(n, new);
                todo.push(format!(
                    "`{}` is now looked up ({}): your plan needs {}.",
                    n.name,
                    quoted(&reads.iter().map(|l| l.address.clone()).collect::<Vec<_>>()),
                    reads.iter().map(|l| l.permission.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
            (true, false) => info.push(format!("`{}` is now static: no lookup, no permission.", n.name)),
            _ => {}
        }
        if os == Shape::Map && ns == Shape::Map {
            let (ok, nk) = (map_keys(&o.value), map_keys(&n.value));
            for k in ok.iter().filter(|k| !nk.contains(k)) {
                todo.push(format!("`{}` lost the key `\"{}\"`: `module.satz.{}[\"{}\"]` fails at plan.", n.name, k, n.name, k));
            }
            for k in nk.iter().filter(|k| !ok.contains(k)) {
                info.push(format!("`{}` gained the key `\"{}\"`.", n.name, k));
            }
        }
        if o.targets != n.targets {
            info.push(format!("`{}` now names {} (was {}).", n.name, quoted(&n.targets), quoted(&o.targets)));
        } else if os == ns && looked_up(o) == looked_up(n) && o.value != n.value && ns != Shape::Map {
            info.push(format!("`{}` has a new value.", n.name));
        }
    }
    for n in added {
        info.push(format!("`{}` is new{}.", n.name, if looked_up(n) { ", looked up" } else { "" }));
    }
    todo.into_iter().map(|text| Change { todo: true, text }).chain(info.into_iter().map(|text| Change { todo: false, text })).collect()
}

/// The file's text; the caller writes nothing when `changes` is empty.
pub(crate) fn render(module: &str, version: &str, estate_path: &str, changes: &[Change]) -> String {
    let mut s = format!(
        "# What changed in `{}`\n\n\
         Generated by satz v{} from `{}`: the previous `satz/interface.satz` of this interface\n\
         against this one — the last transpile's step. One transpile is one step, and the estate's\n\
         history of this file is the guide across several; a project reads its values as\n\
         `module.satz.<name>` (HCL) or `${{{{interface.<name>}}}}` (Satz).\n",
        module, version, estate_path
    );
    let (todo, info): (Vec<&Change>, Vec<&Change>) = changes.iter().partition(|c| c.todo);
    if !todo.is_empty() {
        s.push_str("\n## To do\n\n");
        for c in todo {
            s.push_str(&format!("- [ ] {}\n", c.text));
        }
    }
    if !info.is_empty() {
        s.push_str("\n## Also changed\n\n");
        for c in info {
            s.push_str(&format!("- {}\n", c.text));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(body: &str) -> InterfaceFile {
        let text = format!(
            "interface \"pay\"\n\ncentral {{\n  estate = \"e\"\n  organizations = [\"organizations/123456789012\"]\n}}\n\n{}\n\
             lookup \"data.google_project.infra\" {{\n  reads = \"google_project.infra\"\n  permission = \"resourcemanager.projects.get\"\n  arguments {{\n    project_id = \"corp-infra-001\"\n  }}\n}}\n",
            body
        );
        satz_core::satz::parse(&text).unwrap_or_else(|e| panic!("{}: {}\n{}", e.line, e.msg, text)).interface_file.expect("an interface file")
    }

    const OLD: &str = "output \"folder\" {\n  value = \"folders/1\"\n  targets = [\"google_folder.team\"]\n}\n\
        output \"number\" {\n  value = \"42\"\n  targets = [\"google_project.infra\"]\n}\n\
        output \"vpc\" {\n  value = \"projects/x/global/networks/v\"\n  attach = [\"google_compute_shared_vpc_service_project\"]\n  targets = [\"google_compute_network.v\"]\n}\n\
        output \"folders\" {\n  value = {\n    \"infra\" = \"folders/1\"\n    \"old\" = \"folders/2\"\n  }\n}\n\
        output \"region\" {\n  value = \"europe-west3\"\n}\n\
        output \"gone\" {\n  value = \"x\"\n}\n";

    const NEW: &str = "output \"team_folder\" {\n  value = \"folders/1\"\n  targets = [\"google_folder.team\"]\n}\n\
        output \"number\" {\n  value = \"${{data.google_project.infra.number}}\"\n  targets = [\"google_project.infra\"]\n}\n\
        output \"vpc\" {\n  value = \"projects/x/global/networks/v\"\n  targets = [\"google_compute_network.v\"]\n}\n\
        output \"folders\" {\n  value = {\n    \"infra\" = \"folders/1\"\n    \"new\" = \"folders/3\"\n  }\n}\n\
        output \"region\" {\n  value = [\"europe-west3\"]\n}\n\
        output \"added\" {\n  value = \"y\"\n}\n";

    /// A rename is a removed output and an added one with the same targets; everything a
    /// project must do is a todo, everything else information, in that order.
    #[test]
    fn every_kind_of_change_is_a_line_and_a_rename_is_told_apart_from_a_removal() {
        let c = changes(&file(OLD), &file(NEW));
        let todo: Vec<&str> = c.iter().filter(|c| c.todo).map(|c| c.text.as_str()).collect();
        let info: Vec<&str> = c.iter().filter(|c| !c.todo).map(|c| c.text.as_str()).collect();
        assert_eq!(
            todo,
            [
                "Replace `module.satz.folder` / `${{interface.folder}}` with `…team_folder`: renamed, the same `google_folder.team`.",
                "`gone` is gone: the interface no longer publishes it, so `module.satz.gone` / `${{interface.gone}}` fails at plan.",
                "`number` is now looked up (`data.google_project.infra`): your plan needs resourcemanager.projects.get.",
                "`vpc` no longer takes `google_compute_shared_vpc_service_project`: an attachment of that type is refused.",
                "`folders` lost the key `\"old\"`: `module.satz.folders[\"old\"]` fails at plan.",
                "`region` is now a list (was a single value).",
            ]
        );
        assert_eq!(info, ["`folders` gained the key `\"new\"`.", "`added` is new."]);
    }

    #[test]
    fn a_description_edit_and_an_unchanged_file_are_no_change() {
        let a = file("output \"x\" {\n  value = \"1\"\n  description = \"one\"\n}\n");
        let b = file("output \"x\" {\n  value = \"1\"\n  description = \"uno\"\n}\n");
        assert!(changes(&a, &b).is_empty());
        assert!(changes(&a, &a).is_empty());
        let c = file("output \"x\" {\n  value = \"2\"\n}\n");
        assert_eq!(changes(&a, &c), [Change { todo: false, text: "`x` has a new value.".into() }]);
    }

    #[test]
    fn the_file_holds_the_todo_list_first_and_leaves_out_an_empty_section() {
        let c = changes(&file(OLD), &file(NEW));
        let text = render("pay", "0.0.0", "satz/e.satz", &c);
        assert!(text.starts_with("# What changed in `pay`\n"), "{}", text);
        let todo = text.find("## To do").unwrap();
        let also = text.find("## Also changed").unwrap();
        assert!(todo < also && text[todo..also].contains("- [ ] Replace `module.satz.folder`"), "{}", text);
        let only_info = render("pay", "0.0.0", "e", &[Change { todo: false, text: "x".into() }]);
        assert!(!only_info.contains("## To do") && only_info.contains("## Also changed\n\n- x\n"), "{}", only_info);
    }
}
