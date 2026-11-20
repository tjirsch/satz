//! `satz init`: an organization's estate, or with `--from` a workload estate, with its
//! `config.toml`, `.gitignore`, directories and schemas. Everything that can be checked is
//! checked before the first write — the flags, every interface folder `--from` names, the
//! estate's name and text, the estate file that would be written — so a refused run writes
//! nothing. A run that fails while writing undoes what it wrote: [`Changes`] records every
//! file and folder the run created, replaced or rewrote, and puts the disk back as it was.

use crate::fsx;
use crate::settings::ToolConfig;
use std::error::Error;
use std::path::{Path, PathBuf};

type BoxErr = Box<dyn Error>;

/// The flags of `satz init`, as the command line gave them.
pub(crate) struct Args {
    pub defaults: Option<Vec<String>>,
    pub providers: Option<Vec<String>>,
    pub tf_tool: Option<String>,
    pub customer_id: Option<String>,
    pub customer_shortname: Option<String>,
    pub billing_account_infra: Option<String>,
    pub default_region: Option<String>,
    pub customer_organization_id: Option<String>,
    pub customer_domain: Option<String>,
    pub infra_project_name: Option<String>,
    pub infra_bucket_name: Option<String>,
    pub iac_user: Option<String>,
    pub from: Vec<PathBuf>,
    pub name: Option<String>,
    pub force: bool,
    pub interview: bool,
}

/// Fetches one provider's schema: the tool, the provider, its version, the file to write.
pub(crate) type FetchSchema<'a> = &'a dyn Fn(&str, &str, &str, &str) -> Result<(), BoxErr>;

/// Where init writes: the config as the file says it, the same resolved against its
/// directory, the file's path and directory, and how a schema is fetched.
pub(crate) struct Ctx<'a> {
    pub tool_config: &'a ToolConfig,
    pub runtime_config: &'a ToolConfig,
    pub config_file_path: &'a Path,
    pub config_dir: &'a Path,
    pub fetch_schema: FetchSchema<'a>,
}

const GITIGNORE: &str = r#"# Terraform / OpenTofu
.terraform/
*.tfstate
*.tfstate.backup

# Tool Cache
schemas/

# OS files
.DS_Store
Thumbs.db
"#;

/// The provider lists the flags name, refused when a value names none.
#[derive(Default)]
struct Providers {
    google: Vec<String>,
    aws: Vec<String>,
    azure: Vec<String>,
    alibaba: Vec<String>,
}

fn providers(defaults: Option<Vec<String>>, explicit: Option<Vec<String>>) -> Result<Providers, String> {
    let mut p = Providers::default();
    for d in defaults.unwrap_or_default() {
        if d != "google" {
            return Err(format!("init --defaults: `{}` is no default set — `google` is the one there is", d));
        }
        p.google.extend(["google".to_string(), "google-beta".to_string()]);
    }
    for v in explicit.unwrap_or_default() {
        if v.starts_with("google") {
            p.google.push(v);
        } else if v.starts_with("aws") {
            p.aws.push(v);
        } else if v.starts_with("az") {
            p.azure.push(v);
        } else if v.starts_with("ali") {
            p.alibaba.push(v);
        } else {
            return Err(format!("init --providers: `{}` is no provider of google, aws, azure or alibaba", v));
        }
    }
    p.google.sort();
    p.google.dedup();
    Ok(p)
}

/// A customer id names the estate file and is written into it: letters and digits only.
fn valid_customer_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "init --customer-id: `{}` — a customer id is letters and digits (`C0example`); it names the estate file",
            id
        ));
    }
    Ok(())
}

/// `config.toml` as init writes it: the raw relative values of `tool`, interpreted from the
/// file's own directory.
fn config_text(tool: &ToolConfig, tf_tool: &str, p: &Providers) -> String {
    let mut lines = vec![
        format!("schema_dir = \"{}\"", tool.schema_dir),
        format!("presets_dir = \"{}\"", tool.presets_dir),
        format!("yaml_dir = \"{}\"", tool.yaml_dir),
        format!("hcl_dir = \"{}\"", tool.hcl_dir),
        format!("interfaces_dir = \"{}\"", tool.interfaces_dir),
        // From the same source as the serde default, so the written file and an omitted
        // key cannot disagree.
        format!("include_dirs = {:?}", tool.include_dirs),
        format!("tf_tool = \"{}\"", tf_tool),
    ];
    for (key, list) in [("google_providers", &p.google), ("aws_providers", &p.aws), ("azure_providers", &p.azure), ("alibaba_providers", &p.alibaba)] {
        if !list.is_empty() {
            lines.push(format!("{} = {:?}", key, list));
        }
    }
    lines.push(format!("provider_version = \"{}\"", tool.provider_version));
    lines.push(format!("auto_explode = {:?}", tool.auto_explode));
    lines.push(format!("validation_level = \"{}\"", tool.validation_level));
    lines.join("\n")
}

/// One interface folder `--from` names, read where it stands: `source` is the folder,
/// `copy_to` where it goes under `interfaces/incoming/` — `None` when it stands there
/// already — `use_path` the path the estate `use`s and `file` the interface.
struct Taken {
    source: PathBuf,
    copy_to: Option<PathBuf>,
    use_path: String,
    file: satz_core::satz::InterfaceFile,
}

/// An interface folder for a workload estate: a folder an organization's estate wrote under
/// its `interfaces/outgoing/` — or a copy of one — named after its interface, with that
/// interface's file at `<name>/satz/interface.satz` inside. Read where it stands; the copy to
/// `incoming/<name>/` is the write phase's.
fn read_interface(folder: &Path, incoming: &Path, config_dir: &Path) -> Result<Taken, BoxErr> {
    use crate::interface::{SATZ_DIR, SATZ_FILE};
    let folder = fsx::canonicalize(folder).map_err(|e| format!("{}: {}", folder.display(), e))?;
    let name = folder.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("{}: not a folder with a name", folder.display()))?.to_string();
    let inside = Path::new(&name).join(SATZ_DIR).join(SATZ_FILE);
    let path = folder.join(&inside);
    if !path.is_file() {
        return Err(format!(
            "{}: no {} in it — an interface folder is one folder under interfaces/outgoing/ of an organization's estate, named after its interface",
            folder.display(),
            fsx::slash(&inside)
        )
        .into());
    }
    let text = fsx::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let file = satz_core::satz::parse(&text)
        .map_err(|e| format!("{}:{}: {}", path.display(), e.line, e.msg))?
        .interface_file
        .ok_or_else(|| format!("{}: no interface file — `satz transpile` of the organization's estate writes one under interfaces/outgoing/", path.display()))?;
    let target = absolute(&incoming.join(&name))?;
    let copy_to = (target != folder).then(|| incoming.join(&name));
    // the path the estate `use`s: relative to the config directory when it is under it
    let used = target.join(&inside);
    let use_path = match used.strip_prefix(absolute(config_dir)?) {
        Ok(rel) => fsx::slash(rel),
        Err(_) => fsx::slash(&used),
    };
    Ok(Taken { source: folder, copy_to, use_path, file })
}

/// The canonical absolute form of a path that need not exist yet: its nearest existing
/// ancestor canonicalized, the rest joined on.
fn absolute(path: &Path) -> Result<PathBuf, BoxErr> {
    let path = if path.as_os_str().is_empty() { Path::new(".") } else { path };
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if at.exists() {
            let mut out = fsx::canonicalize(at)?;
            out.extend(rest.iter().rev());
            return Ok(out);
        }
        match (at.file_name(), at.parent()) {
            (Some(n), Some(parent)) => {
                rest.push(n.to_os_string());
                at = if parent.as_os_str().is_empty() { Path::new(".") } else { parent };
            }
            _ => return Err(format!("{}: no part of this path exists", path.display()).into()),
        }
    }
}

/// Copy a directory with everything in it.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fsx::create_dir_all(to)?;
    for entry in fsx::read_dir_entries(from)? {
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| std::io::Error::new(e.kind(), format!("failed to copy '{}': {}", entry.path().display(), e)))?;
        }
    }
    Ok(())
}

/// One change a run made, in the order it made them.
enum Step {
    /// A file or folder that did not exist before the run.
    Created(PathBuf),
    /// A folder that existed, moved aside to `kept` before `path` was written anew.
    Replaced { path: PathBuf, kept: PathBuf },
    /// A file that existed, with its bytes before the run.
    Rewrote { path: PathBuf, before: Vec<u8> },
}

/// What one run changed on disk: undone when the run fails, kept when it succeeds.
/// Nothing that existed before the run is removed by an undo.
#[derive(Default)]
pub(crate) struct Changes {
    steps: Vec<Step>,
}

impl Changes {
    /// Create `dir` and its missing parents, recording the topmost one created.
    fn mkdirs(&mut self, dir: &Path) -> std::io::Result<()> {
        let mut top = None;
        for a in dir.ancestors() {
            if a.as_os_str().is_empty() || a.exists() {
                break;
            }
            top = Some(a.to_path_buf());
        }
        fsx::create_dir_all(dir)?;
        if let Some(t) = top {
            self.steps.push(Step::Created(t));
        }
        Ok(())
    }

    /// Write the file `path` with `write`: recorded before it is written, so a write that
    /// fails half-way is undone too.
    fn file<E: From<std::io::Error>>(&mut self, path: &Path, write: impl FnOnce(&Path) -> Result<(), E>) -> Result<(), E> {
        if path.exists() {
            let before = std::fs::read(path).map_err(|e| std::io::Error::new(e.kind(), format!("failed to read file '{}': {}", path.display(), e)))?;
            self.steps.push(Step::Rewrote { path: path.to_path_buf(), before });
        } else {
            if let Some(parent) = path.parent() {
                self.mkdirs(parent)?;
            }
            self.steps.push(Step::Created(path.to_path_buf()));
        }
        write(path)
    }

    /// Write the folder `path` anew with `write`: one that exists is moved aside first and
    /// put back by an undo.
    fn folder(&mut self, path: &Path, write: impl FnOnce(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
        if path.exists() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let kept = path.with_file_name(format!(".{}.satz-init-{}", name, std::process::id()));
            std::fs::rename(path, &kept).map_err(|e| std::io::Error::new(e.kind(), format!("failed to move '{}' aside: {}", path.display(), e)))?;
            self.steps.push(Step::Replaced { path: path.to_path_buf(), kept });
        } else {
            if let Some(parent) = path.parent() {
                self.mkdirs(parent)?;
            }
            self.steps.push(Step::Created(path.to_path_buf()));
        }
        write(path)
    }

    /// Record `path`, which something else is about to write, when it does not exist yet.
    fn will_create(&mut self, path: &Path) {
        if !path.exists() {
            self.steps.push(Step::Created(path.to_path_buf()));
        }
    }

    /// The run succeeded: drop the folders moved aside.
    fn keep(self) -> std::io::Result<()> {
        for step in self.steps {
            if let Step::Replaced { kept, .. } = step {
                fsx::remove_dir_all(&kept)?;
            }
        }
        Ok(())
    }

    /// The run failed: undo every step, the last first. Returns what was undone, and what
    /// could not be, as lines for the error.
    fn undo(self) -> (Vec<String>, Vec<String>) {
        let (mut done, mut failed) = (Vec::new(), Vec::new());
        for step in self.steps.into_iter().rev() {
            let result = match &step {
                Step::Created(p) => match std::fs::symlink_metadata(p) {
                    Ok(m) if m.is_dir() => fsx::remove_dir_all(p).map(|_| Some(format!("removed {}", p.display()))),
                    Ok(_) => fsx::remove_file(p).map(|_| Some(format!("removed {}", p.display()))),
                    Err(_) => Ok(None),
                },
                Step::Replaced { path, kept } => (|| {
                    if path.exists() {
                        fsx::remove_dir_all(path)?;
                    }
                    std::fs::rename(kept, path)?;
                    Ok(Some(format!("put back {}", path.display())))
                })(),
                Step::Rewrote { path, before } => fsx::write_verbatim(path, before).map(|_| Some(format!("restored {}", path.display()))),
            };
            match result {
                Ok(Some(line)) => done.push(line),
                Ok(None) => {}
                Err(e) => failed.push(e.to_string()),
            }
        }
        (done, failed)
    }
}

/// `e`, the error that ended the run, with what the undo of `changes` did.
fn undone(e: BoxErr, changes: Changes) -> BoxErr {
    if changes.steps.is_empty() {
        return e;
    }
    let (done, failed) = changes.undo();
    let mut msg = format!("{}\ninit undid what this run wrote: {}", e, done.join(", "));
    if !failed.is_empty() {
        msg.push_str(&format!("\ninit could not undo: {}", failed.join("; ")));
    }
    msg.into()
}

/// The estate file an organization's `init` writes, decided before anything is written.
enum OrgEstate {
    /// The estate exists and `--force` is not given: the params named here merged into it.
    Merge { path: PathBuf, before: String, merged: String, log: Vec<crate::init_params::Merged> },
    /// A new estate, or one `--force` rewrites: composed from these, with the pack graph
    /// read once the library is there.
    Compose { path: PathBuf, args: crate::template::TemplateArgs, fetch_library: bool },
}

/// `satz init`.
pub(crate) async fn run(args: Args, ctx: &Ctx<'_>) -> Result<(), BoxErr> {
    let rc = ctx.runtime_config;
    let p = providers(args.defaults, args.providers)?;
    let tool = args.tf_tool.unwrap_or_else(|| ctx.tool_config.tf_tool.clone());
    if let (Some(id), true) = (&args.customer_id, args.from.is_empty()) {
        valid_customer_id(id)?;
    }

    // the files beside the estate: each written only when it is missing
    let config_text = (!ctx.config_file_path.exists()).then(|| config_text(ctx.tool_config, &tool, &p));
    // next to config.toml — the root the directories hang off, not wherever init was run
    let gitignore_path = ctx.config_dir.join(".gitignore");
    let write_gitignore = !gitignore_path.exists();

    // the schemas: for the providers the command line names, else the ones the config does;
    // one already on disk is kept
    let mut all_provs = p.google;
    all_provs.extend(p.aws);
    all_provs.extend(p.azure);
    all_provs.extend(p.alibaba);
    let wanted: Vec<(String, String)> = if all_provs.is_empty() {
        ctx.tool_config.parsed_providers()
    } else {
        all_provs.into_iter().map(|p| (p, rc.provider_version.clone())).collect()
    };

    // A workload estate: written from the interface folders — the delegation's account,
    // infrastructure project and state bucket, and the core export `default_region` — so
    // nothing is derived from the sign-in: the organization's estate answered it all.
    let mut workload = None;
    let mut org = None;
    if !args.from.is_empty() {
        let base = if ctx.config_dir.as_os_str().is_empty() { Path::new(".") } else { ctx.config_dir };
        let incoming = PathBuf::from(&rc.interfaces_dir).join(crate::interface::INCOMING);
        let taken = args.from.iter().map(|f| read_interface(f, &incoming, base)).collect::<Result<Vec<_>, _>>()?;
        let files: Vec<(String, satz_core::satz::InterfaceFile)> = taken.iter().map(|t| (t.use_path.clone(), t.file.clone())).collect();
        let name = match args.name {
            Some(n) => n,
            None => crate::project::default_name(&files).ok_or("init --from: none of the folders is a delegation's own (`delegation-<name>`), so the estate has no name to take — take that folder too, or give `--name`")?,
        };
        let src = crate::project::estate(&name, &files)?;
        let path = PathBuf::from(&rc.yaml_dir).join(format!("{}.satz", crate::project::label(&name)));
        if path.exists() && !args.force {
            return Err(format!("{} exists — `--force` rewrites it", path.display()).into());
        }
        workload = Some((taken, name, src, path));
    } else {
        let stated = crate::init_params::Stated {
            customer_id: args.customer_id,
            customer_shortname: args.customer_shortname,
            billing_account_infra: args.billing_account_infra,
            default_region: args.default_region,
            customer_organization_id: args.customer_organization_id,
            customer_domain: args.customer_domain,
            infra_project_name: args.infra_project_name,
            infra_bucket_name: args.infra_bucket_name,
            iac_user: args.iac_user,
        };
        org = org_estate(stated, rc, args.force, args.interview).await?;
    }

    // Everything is checked: the writes, each recorded so a failure undoes the run.
    let mut changes = Changes::default();
    let written: Result<(), BoxErr> = async {
        for d in [&rc.yaml_dir, &rc.hcl_dir, &rc.schema_dir] {
            changes.mkdirs(Path::new(d))?;
            println!("Created directory: {}", d);
        }
        if let Some(text) = &config_text {
            changes.file(ctx.config_file_path, |p| fsx::write(p, text))?;
            println!("Generated {}", ctx.config_file_path.display());
        }
        if write_gitignore {
            changes.file(&gitignore_path, |p| fsx::write(p, GITIGNORE))?;
            println!("Created {}", gitignore_path.display());
        }
        if let Some((taken, name, src, path)) = &workload {
            for t in taken {
                if let Some(to) = &t.copy_to {
                    changes.folder(to, |to| copy_dir(&t.source, to))?;
                    println!("Took {} into {}", t.source.display(), to.display());
                }
            }
            changes.file(path, |p| fsx::write_generated_satz(p, src))?;
            println!(
                "Generated the workload estate: {} — read through {}; next: `satz transpile {}.satz`",
                path.display(),
                taken.iter().map(|t| t.use_path.as_str()).collect::<Vec<_>>().join(", "),
                crate::project::label(name)
            );
        }
        if let Some((c_id, estate)) = &org {
            write_org_estate(estate, c_id, rc, &mut changes).await?;
        }
        for (provider, version) in &wanted {
            let name = provider.split('/').next_back().unwrap_or(provider).to_string();
            let out = format!("{}/{}.json", rc.schema_dir, name);
            if Path::new(&out).exists() {
                println!("Schema for {} already present: {}", name, out);
                continue;
            }
            println!("Fetching schema for {} {}...", provider, version);
            changes.will_create(Path::new(&out));
            (ctx.fetch_schema)(&tool, provider, version, &out).map_err(|e| {
                format!(
                    "init: the {} schema could not be fetched ({}) — nothing transpiles without it. Put OpenTofu on PATH and run init again",
                    name, e
                )
            })?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = written {
        return Err(undone(e, changes));
    }
    changes.keep()?;

    // A bootstrap param is either stated, derived, or ASKED — there is no fourth state
    // where an estate is simply born incomplete. Stating it is the flag because the
    // interview is interactive and a scripted run must not block on it. It runs on the
    // written estate: what it answers is the interview's, not part of init's run.
    if let Some((_, OrgEstate::Merge { path, .. } | OrgEstate::Compose { path, .. })) = &org {
        if args.interview {
            // init writes the estate-core `use` commented out so the estate transpiles
            // before any library is fetched; the interview switches it on, or it finds no
            // pack declaring a question and asks nothing
            if crate::template::use_estate_core(path)? {
                println!("switched on `use \"presets/estate-core.satz\"` in {}", path.display());
            }
            println!();
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            let mut out = std::io::stdout();
            crate::interview::run(path, rc, false, false, &mut input, &mut out)?;
        }
    }
    println!("Initialization complete.");
    Ok(())
}

/// The organization's estate file: the params the operator typed, the rest derived from
/// the sign-in, and whether it is merged into the estate that exists or composed anew —
/// `None` when no customer id is stated or derivable, and there is no estate to write.
async fn org_estate(stated: crate::init_params::Stated, rc: &ToolConfig, force: bool, interview: bool) -> Result<Option<(String, OrgEstate)>, BoxErr> {
    // What the operator TYPED, before anything is derived: a re-run merges exactly these
    // into an estate that already exists, and nothing else, so a value somebody put there
    // by hand survives.
    let crate::init_params::Stated {
        customer_id,
        customer_shortname,
        billing_account_infra,
        default_region,
        customer_organization_id,
        customer_domain,
        infra_project_name,
        infra_bucket_name,
        iac_user,
    } = stated.clone();

    // Derivation from the sign-in is the DEFAULT, not a flag: every value below is
    // readable by the person already signed in. Stated wins, derived fills the rest and
    // says where it came from, and what nothing can answer stays EMPTY — never a
    // placeholder.
    let (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user) = {
        let need_org = customer_organization_id.is_none() || customer_id.is_none();
        let need_billing = billing_account_infra.is_none();
        match crate::gcp::identity::live_defaults(need_org, need_billing, None, None).await {
            Ok(live) => {
                let mut note = crate::init_params::Derivations::default();
                let customer_domain = note.fill("customer_domain", customer_domain, live.customer_domain.clone(), "the signed-in person");
                let iac_user = note.fill(
                    "first_admin",
                    iac_user,
                    live.first_admin.as_ref().zip(live.customer_domain.as_ref()).map(|(a, d)| format!("{}@{}", a, d)),
                    "the signed-in person",
                );
                let customer_id = note.fill("customer_id", customer_id, live.customer_id.clone(), "organizations:search");
                // No organization visible is the greenfield case: the id stays empty and
                // `bootstrap --greenfield` fills it in.
                let customer_organization_id = note.fill("customer_organization_id", customer_organization_id, live.org_id.clone(), "organizations:search");
                let billing_account_infra = note.fill(
                    "billing_account_infra",
                    billing_account_infra,
                    live.billing_account.clone(),
                    "billingAccounts.list (the one open account)",
                );
                // nothing on the platform names the customer: it is reported as unanswered
                // rather than guessed at
                let customer_shortname = note.fill("customer_shortname", customer_shortname, None, "");
                note.report();
                (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user)
            }
            Err(why) => {
                eprintln!("init: nothing could be derived from the sign-in — {}", why);
                eprintln!(
                    "      what you did not pass is written empty; `satz bootstrap` names each one, and \
                     `satz init` merges them in later."
                );
                (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user)
            }
        }
    };
    let Some(c_id) = customer_id else { return Ok(None) };
    valid_customer_id(&c_id)?;
    let path = PathBuf::from(&rc.yaml_dir).join(format!("{}.satz", c_id));
    if path.exists() && !force {
        let before = fsx::read_to_string(&path)?;
        let (merged, log) = crate::init_params::merge(&before, &stated)?;
        return Ok(Some((c_id, OrgEstate::Merge { path, before, merged, log })));
    }

    let domain = customer_domain.unwrap_or_default();
    // an admin nobody named stays empty: `first.admin` looked like an answer and was not one
    let resolved_iac_user = iac_user.unwrap_or_default();
    // The template and the shipped presets both compose members as
    // `user:{first-admin}@{customer-domain}`, so `first-admin` holds the local part only.
    let (first_admin, user_domain) = resolved_iac_user.split_once('@').unwrap_or((resolved_iac_user.as_str(), ""));
    if !user_domain.is_empty() && !domain.is_empty() && user_domain != domain {
        eprintln!(
            "Warning: --iac-user domain '{}' differs from --customer-domain '{}'. \
             Members are built as first-admin@customer-domain, so they will use '{}@{}'.",
            user_domain, domain, first_admin, domain
        );
    }
    // the two names that FOLLOW from the short name, by the defaults
    // `presets/estate-core.satz` documents — derived when it is known, empty when it is not
    let shortname = customer_shortname.clone().unwrap_or_default();
    let derive_from_shortname = |given: Option<String>, suffix: &str| -> String {
        match given {
            Some(v) => v,
            None if !shortname.trim().is_empty() => format!("{}{}", shortname.trim(), suffix),
            None => String::new(),
        }
    };
    let args = crate::template::TemplateArgs {
        customer_id: c_id.clone(),
        shortname: customer_shortname.unwrap_or_default(),
        billing_id: billing_account_infra.unwrap_or_default(),
        region: default_region.unwrap_or_else(|| crate::bootstrap::DEFAULT_REGION.to_string()),
        // never a placeholder: unset stays empty, and the bootstrap gate refuses it by name
        org_id: customer_organization_id.unwrap_or_default(),
        domain: domain.clone(),
        project_id: derive_from_shortname(infra_project_name, "-infra-001"),
        bucket_id: derive_from_shortname(infra_bucket_name, "-infra-001-state"),
        first_admin: first_admin.to_string(),
    };
    // Every bootstrap question lives in presets/estate-core.satz, and the pack lines come
    // from the graph beside it: `--interview` fetches the library before the file is
    // written, so the estate it asks about carries the menu.
    let fetch_library = interview && !Path::new(&rc.presets_dir).join("estate-core.satz").exists();
    Ok(Some((c_id, OrgEstate::Compose { path, args, fetch_library })))
}

/// Write the organization's estate file `estate` decided, recording each change.
async fn write_org_estate(estate: &OrgEstate, c_id: &str, rc: &ToolConfig, changes: &mut Changes) -> Result<(), BoxErr> {
    match estate {
        OrgEstate::Merge { path, before, merged, log } => {
            if log.is_empty() {
                println!("{} exists and this command named no params to merge into it (`--force` rewrites it).", path.display());
                return Ok(());
            }
            for entry in log {
                match entry {
                    crate::init_params::Merged::Changed { param, from, to } if from.is_empty() => println!("  set     {} = {:?}", param, to),
                    crate::init_params::Merged::Changed { param, from, to } => println!("  changed {} = {:?} (was {:?})", param, to, from),
                    crate::init_params::Merged::Same { param, value } => println!("  kept    {} = {:?}", param, value),
                }
            }
            if merged != before {
                changes.file(path, |p| fsx::write_edited_satz(p, before, merged))?;
            }
            println!("Merged into {} — every line this command did not name is unchanged.", path.display());
        }
        OrgEstate::Compose { path, args, fetch_library } => {
            let presets_dir = Path::new(&rc.presets_dir);
            if *fetch_library {
                let core = presets_dir.join("estate-core.satz");
                println!("\nfetching the preset library — the bootstrap questions are read from {}", core.display());
                changes.will_create(presets_dir);
                crate::presets::get_presets(&rc.presets_dir, rc, false, None).await.map_err(|e| {
                    format!(
                        "init --interview: the questions live in {}, which is not here and could not be fetched ({}) — run `satz get-presets`, then `satz interview {}.satz`",
                        core.display(),
                        e,
                        c_id
                    )
                })?;
            }
            let graph = crate::pack_graph::read(presets_dir)?;
            let text = crate::template::template_text(args, graph.as_ref());
            changes.file(path, |p| fsx::write_generated_satz(p, &text))?;
            if graph.is_none() {
                println!("{}", crate::pack_graph::no_menu_note(presets_dir));
            }
            println!("Generated estate: {} — next: `satz bootstrap {}.satz --dry-run`", path.display(), c_id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-init-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        fsx::canonicalize(&dir).unwrap()
    }

    /// Every file and folder under `dir`, with each file's bytes.
    fn listing(dir: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
        fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
            for e in std::fs::read_dir(at).unwrap() {
                let p = e.unwrap().path();
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                if p.is_dir() {
                    out.insert(rel, None);
                    walk(root, &p, out);
                } else {
                    out.insert(rel, Some(std::fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// The interface folders `(name, text)` under `root`, as an organization's estate writes them.
    fn folders(root: &Path, entries: &[(&str, &str)]) -> Vec<PathBuf> {
        entries
            .iter()
            .map(|(name, text)| {
                let folder = root.join(name);
                std::fs::create_dir_all(folder.join(name).join("satz")).unwrap();
                std::fs::write(folder.join(name).join("satz").join("interface.satz"), text).unwrap();
                folder
            })
            .collect()
    }

    fn args(from: Vec<PathBuf>, name: Option<&str>) -> Args {
        Args {
            defaults: None,
            providers: None,
            tf_tool: None,
            customer_id: None,
            customer_shortname: None,
            billing_account_infra: None,
            default_region: None,
            customer_organization_id: None,
            customer_domain: None,
            infra_project_name: None,
            infra_bucket_name: None,
            iac_user: None,
            from,
            name: name.map(str::to_string),
            force: false,
            interview: false,
        }
    }

    /// `satz init` in `dir`, as the binary runs it: the config read from `dir/config.toml`,
    /// a schema fetched by `fetch`.
    async fn init(dir: &Path, args: Args, fetch: FetchSchema<'_>) -> Result<(), String> {
        let config_file_path = dir.join("config.toml");
        let tool_config = crate::settings::parse_tool_config(&config_file_path).unwrap();
        let runtime_config = crate::settings::resolved_config(&tool_config, dir);
        let ctx = Ctx { tool_config: &tool_config, runtime_config: &runtime_config, config_file_path: &config_file_path, config_dir: dir, fetch_schema: fetch };
        run(args, &ctx).await.map_err(|e| e.to_string())
    }

    fn fetched(_: &str, _: &str, _: &str, out: &str) -> Result<(), BoxErr> {
        std::fs::write(out, "{}")?;
        Ok(())
    }

    /// A refused `init --from` — a folder copied before the estate account's id was
    /// published, no delegation's own folder, a name that is no interface name — writes
    /// nothing, into an empty folder or one that holds a repository already; the corrected
    /// run then succeeds without `--force`.
    #[tokio::test]
    async fn a_refused_init_from_leaves_the_folder_as_it_found_it() {
        let taken = crate::project::tests::published();
        let (base, project) = (taken("delegation-data"), taken("data-lake"));
        let no_id = base.1.replace("output \"estate_account_id\"", "output \"gone\"");
        for prepared in [false, true] {
            let tag = if prepared { "prepared" } else { "empty" };
            let sources = scratch(&format!("{}-sources", tag));
            let good = folders(&sources.join("good"), &[("delegation-data", &base.1), ("data-lake", &project.1)]);
            let stale = folders(&sources.join("stale"), &[("delegation-data", &no_id), ("data-lake", &project.1)]);
            let dir = scratch(tag);
            if prepared {
                std::fs::write(dir.join("README.md"), "the repository\n").unwrap();
                std::fs::create_dir_all(dir.join("interfaces/incoming/delegation-data")).unwrap();
                std::fs::write(dir.join("interfaces/incoming/delegation-data/old.txt"), "an earlier copy\n").unwrap();
            }
            let before = listing(&dir);
            for (refused, name, says) in [
                (stale.clone(), None, "publishes no static `estate_account_id`"),
                (vec![good[1].clone()], None, "none of the folders is a delegation's own"),
                (good.clone(), Some("Data_Lake"), "`Data_Lake`: a name is an interface name"),
            ] {
                let e = init(&dir, args(refused, name), &fetched).await.expect_err("refused");
                assert!(e.contains(says), "{}: {}", tag, e);
                assert_eq!(listing(&dir), before, "{}: the refused run `{}` wrote", tag, says);
            }
            init(&dir, args(good.clone(), None), &fetched).await.unwrap_or_else(|e| panic!("{}: {}", tag, e));
            let estate = std::fs::read_to_string(dir.join("satz/data.satz")).unwrap();
            assert!(estate.contains("use \"interfaces/incoming/delegation-data/delegation-data/satz/interface.satz\""), "{}", estate);
            assert!(dir.join("interfaces/incoming/data-lake/data-lake/satz/interface.satz").is_file());
            assert!(!dir.join("interfaces/incoming/delegation-data/old.txt").exists(), "the earlier copy is replaced");
            assert!(dir.join("config.toml").is_file() && dir.join("schemas/google.json").is_file());
            let leftovers: Vec<String> = listing(&dir.join("interfaces/incoming")).into_keys().filter(|k| k.contains(".satz-init-")).collect();
            assert!(leftovers.is_empty(), "{:?}", leftovers);
            // the estate exists now: a second run is refused, and writes nothing
            let after = listing(&dir);
            let e = init(&dir, args(good, None), &fetched).await.expect_err("an estate that exists");
            assert!(e.contains("exists — `--force` rewrites it"), "{}", e);
            assert_eq!(listing(&dir), after);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&sources);
        }
    }

    /// A run that fails while writing — here the schema fetch, after the estate and the copies
    /// are written — undoes every write: what it created goes, a folder it replaced comes back.
    #[tokio::test]
    async fn a_failure_while_writing_undoes_the_run() {
        let taken = crate::project::tests::published();
        let (base, project) = (taken("delegation-data"), taken("data-lake"));
        let sources = scratch("undo-sources");
        let good = folders(&sources, &[("delegation-data", &base.1), ("data-lake", &project.1)]);
        let failing = |_: &str, _: &str, _: &str, out: &str| -> Result<(), BoxErr> {
            std::fs::write(out, "half")?;
            Err("no tofu".into())
        };
        for prepared in [false, true] {
            let dir = scratch(if prepared { "undo-prepared" } else { "undo-empty" });
            if prepared {
                std::fs::write(dir.join("config.toml"), "").unwrap();
                std::fs::create_dir_all(dir.join("interfaces/incoming/delegation-data")).unwrap();
                std::fs::write(dir.join("interfaces/incoming/delegation-data/old.txt"), "an earlier copy\n").unwrap();
            }
            let before = listing(&dir);
            let e = init(&dir, args(good.clone(), None), &failing).await.expect_err("the fetch fails");
            assert!(e.contains("schema could not be fetched") && e.contains("init undid what this run wrote"), "{}", e);
            assert_eq!(listing(&dir), before);
            let _ = std::fs::remove_dir_all(&dir);
        }
        let _ = std::fs::remove_dir_all(&sources);
    }

    /// The organization's `init` refuses a flag value it cannot use before it writes anything.
    #[tokio::test]
    async fn a_refused_organization_init_writes_nothing() {
        let dir = scratch("org");
        let mut defaults = args(Vec::new(), None);
        defaults.defaults = Some(vec!["aws".into()]);
        let mut provider = args(Vec::new(), None);
        provider.providers = Some(vec!["oracle".into()]);
        let mut customer = args(Vec::new(), None);
        customer.customer_id = Some("../C0example".into());
        for (a, says) in [(defaults, "`aws` is no default set"), (provider, "`oracle` is no provider"), (customer, "a customer id is letters and digits")] {
            let e = init(&dir, a, &fetched).await.expect_err("refused");
            assert!(e.contains(says), "{}", e);
            assert!(listing(&dir).is_empty(), "{:?}", listing(&dir).keys().collect::<Vec<_>>());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
