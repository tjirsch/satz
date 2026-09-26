//! `satz interfaces`: what the estate publishes, as a report — every export with the
//! interface it stands in, whether a project reads it as a literal, a lookup or a map, what
//! it names and what may be attached to it, and every interface with what it uses. It is
//! read off the same compile `transpile` runs, so it says what the next transpile writes to
//! `interfaces/`. satz-studio reads the json form.

use serde::Serialize;

use crate::interface::{How, Interface};
use satz_core::pipeline::{ResolvedExport, ResolvedInterface};

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct InterfacesReport {
    pub estate: String,
    /// every export, core ones first, then each interface's in declaration order
    pub exports: Vec<ExportRow>,
    /// every declared interface; `core` is not one — the core exports are the ones with
    /// no `interface`
    pub interfaces: Vec<InterfaceRow>,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct ExportRow {
    pub name: String,
    /// the interface it stands in; `None` for a core export, which every interface carries
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    /// `static` (a literal), `lookup` (a data source a project's plan reads) or `map`
    /// (`all <type>`, keyed by label)
    pub how: &'static str,
    /// what a project's module holds: the literal, or the data source expression
    pub value: String,
    /// `all <type>`: the type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all: Option<String>,
    /// the estate's resources it names, by address
    pub targets: Vec<String>,
    /// the attachment types a project may create against it
    pub attach: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// where it is declared
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct InterfaceRow {
    pub name: String,
    /// in the library every project's folder carries: marked `common`, or declared in a pack
    pub common: bool,
    /// the interfaces its module also carries, directly or through another
    pub uses: Vec<String>,
    /// its own exports
    pub exports: usize,
    pub file: String,
    pub line: usize,
}

/// The report of a compiled estate. `interface` is `None` when the estate exports nothing.
pub(crate) fn report(estate: &str, interface: Option<&Interface>, declared: &[ResolvedExport], interfaces: &[ResolvedInterface]) -> InterfacesReport {
    let at = |i: &Option<String>, name: &str| declared.iter().find(|d| &d.interface == i && d.name == name).map(|d| (d.file.clone(), d.line)).unwrap_or_default();
    let exports = interface
        .map(|i| {
            i.outputs
                .iter()
                .map(|o| {
                    let (file, line) = at(&o.interface, &o.name);
                    ExportRow {
                        name: o.name.clone(),
                        interface: o.interface.clone(),
                        how: match (&o.all, &o.how) {
                            (Some(_), _) => "map",
                            (None, How::Static(_)) => "static",
                            (None, How::Lookup(_)) => "lookup",
                        },
                        value: o.module_value.clone(),
                        all: o.all.clone(),
                        targets: o.targets.clone(),
                        attach: o.attach.clone(),
                        description: o.description.clone(),
                        file,
                        line,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let interfaces = interfaces
        .iter()
        .map(|d| InterfaceRow {
            name: d.name.clone(),
            common: d.common,
            uses: d.uses.clone(),
            exports: declared.iter().filter(|x| x.interface.as_deref() == Some(d.name.as_str())).count(),
            file: d.file.clone(),
            line: d.line,
        })
        .collect();
    InterfacesReport { estate: estate.to_string(), exports, interfaces }
}

/// The report for a person: the core exports, then each interface with its own.
pub(crate) fn render_text(r: &InterfacesReport) -> String {
    let row = |e: &ExportRow| {
        let mut s = format!("  {:<28} {:<7} {}", e.name, e.how, e.value);
        if !e.attach.is_empty() {
            s.push_str(&format!("  attach {}", e.attach.join(", ")));
        }
        s.push('\n');
        s
    };
    let mut s = format!("estate {}\n", r.estate);
    if r.exports.is_empty() {
        s.push_str("\nno export — the estate publishes nothing to a project\n");
        return s;
    }
    s.push_str("\ncore — every interface carries these\n");
    for e in r.exports.iter().filter(|e| e.interface.is_none()) {
        s.push_str(&row(e));
    }
    for i in &r.interfaces {
        let mut head = format!("\ninterface {}", i.name);
        if i.common {
            head.push_str(" (common)");
        }
        if !i.uses.is_empty() {
            head.push_str(&format!(" — uses {}", i.uses.join(", ")));
        }
        s.push_str(&format!("{}  {}:{}\n", head, i.file, i.line));
        for e in r.exports.iter().filter(|e| e.interface.as_deref() == Some(i.name.as_str())) {
            s.push_str(&row(e));
        }
    }
    s
}
