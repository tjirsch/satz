# Contributing to satz

Thank you for your interest in contributing to satz!
Contributions from the community to help improve this project are welcome.

## How to Contribute

### Reporting Bugs

If you find a bug, please create a new issue using the [Bug Report template](.github/ISSUE_TEMPLATE/bug_report.md). Be sure to include:
- A clear description of the issue
- Steps to reproduce
- Expected vs. actual behavior
- Any relevant logs or screenshots

### Suggesting Enhancements

If you have an idea for a new feature or improvement, please create a new issue using the [Feature Request template](.github/ISSUE_TEMPLATE/feature_request.md).

### Pull Requests

1.  **Fork the repository** and create your branch from `main`.
2.  **Clone the repository** to your local machine.
3.  **Create a new branch** for your feature or bug fix:
    ```bash
    git checkout -b feature/my-new-feature
    ```
4.  **Make your changes**. Ensure your code follows the project's coding standards.
5.  **Test your changes**. Run existing tests and add new ones if necessary.
6.  **Commit your changes** with descriptive commit messages.
7.  **Push your branch** to your fork:
    ```bash
    git push origin feature/my-new-feature
    ```
8.  **Open a Pull Request** against the `main` branch of the original repository.

## Development Setup

1.  Ensure you have Rust installed (latest stable version recommended).
2.  Clone the repository.
3.  Run `cargo build` to verify the build.
4.  Run `cargo test --workspace --locked` to run the test suite.
5.  CI runs four jobs on every pull request and on every push to `main`, and
    a pull request merges only when all four pass: the privacy gate (`scripts/check-names.sh`,
    see below), the grammar parse (`scripts/check-grammar.sh`: every Satz file
    parses with the pinned tree-sitter grammar), `checks` (`cargo clippy
    --workspace --all-targets -- -D warnings`, where a warning is a failure,
    and `cargo test --workspace --locked`), and the smoke matrix
    (`scripts/smoke.sh`, which runs every estate-consuming command end to end
    against `tests/smoke/`). Run them before opening a pull request. A push to
    a branch runs nothing by itself — its pull request runs the jobs on the
    branch merged into `main` — and a newer push to a pull request cancels
    the run it supersedes. A run on `main` is never cancelled.

## Coding Standards

-   Follow standard Rust idioms and best practices.
-   Use `cargo fmt` to format your code before committing.
-   `cargo clippy --workspace --all-targets -- -D warnings` must be clean;
    CI denies warnings.

## Decisions

A change that was a genuine choice between defensible alternatives — one a reader of
the code would later ask "why on earth" about, or one that would be expensive to
reverse — gets an architecture decision record in `docs/adr/`, in MADR form: the
context, the options with their real trade-offs (the chosen one included), what was
decided and what it costs. Most changes are none of those and need no record. See
`docs/adr/README.md` for the conventions; number the file one past the highest, and
never renumber.

## Privacy gate

This is a public repository and a privacy gate runs on every pull request
and every push to `main` (`scripts/check-names.sh`, `.github/workflows/names-gate.yml`). It is
neutral — it names nobody — and rejects:

- identifiers that are not one of the documented example values in
  `docs/examples.md`: Google Workspace directory ids (`C0…`),
  organisation / project / folder numbers, billing accounts, GUIDs and
  their dashless 32-hex form, project ids in `projects/…`, `project = …`
  and `--project`;
- e-mail addresses, domains that are neither IANA-reserved
  (`example.com`, `.example`, `.test`, …) nor a known vendor host,
  repository URLs and checkout paths;
- local files, if they are ever staged: `CLAUDE.local.md`, `*.local.md`,
  `.claude/`, `attestations.yaml`, `evidence/`;
- commits whose author or committer is not a GitHub noreply address
  (`<id>+<user>@users.noreply.github.com` — enable "keep my email address
  private" in your GitHub settings) or the maintainer's address.

Run it before you push: `bash scripts/check-names.sh`, and enable the
pre-commit hook once per clone with `git config core.hooksPath .githooks`.
Use the example customers for anything that looks like a real organisation.

## License

satz is licensed under the Apache License, Version 2.0. Section 5 of that licence
states the terms for contributions: a contribution you deliberately submit for
inclusion is licensed to the project under Apache 2.0. There is no separate
contributor agreement to sign.
