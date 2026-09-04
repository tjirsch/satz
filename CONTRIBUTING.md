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
4.  Run `cargo test` to run the test suite.

## Coding Standards

-   Follow standard Rust idioms and best practices.
-   Use `cargo fmt` to format your code before committing.
-   Use `cargo clippy` to catch common mistakes and improve code quality.

## Privacy gate

This is a public repository and a privacy gate runs on every push and pull
request (`scripts/check-names.sh`, `.github/workflows/names-gate.yml`). It is
neutral — it names nobody — and rejects:

- identifiers that are not one of the documented example values in
  `docs/examples.md`: Google Workspace directory ids (`C0…`),
  organisation / project / folder numbers, billing accounts;
- e-mail addresses and domains that are neither IANA-reserved
  (`example.com`, `.example`, `.test`, …) nor a known vendor host;
- commits whose author or committer is not a GitHub noreply address
  (`<id>+<user>@users.noreply.github.com` — enable "keep my email address
  private" in your GitHub settings) or the maintainer's address.

Run it before you push: `bash scripts/check-names.sh`, and enable the
pre-commit hook once per clone with `git config core.hooksPath .githooks`.
Use the example customers for anything that looks like a real organisation.

## License

By contributing to this project, you agree that your contributions will be licensed under the MIT License.
