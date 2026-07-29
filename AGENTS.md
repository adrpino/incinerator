# Agent Development Guidelines

This document outlines the strict engineering and architectural standards that all autonomous AI agents must adhere to when contributing to this codebase.

## 1. Strict CI Alignment

Every single pull request and commit to `main` must pass the Continuous Integration pipeline seamlessly. Ensure that before proposing or implementing changes, you execute and verify the following commands locally:

- **Formatting:** `cargo fmt` (CI strictly enforces `cargo fmt -- --check`)
- **Linting:** `cargo clippy -- -D warnings` (CI treats all warnings as hard errors)
- **Testing:** `cargo test` (all unit and integration tests must pass successfully)
- **Building:** `cargo build` (must compile without warnings or errors)

## 2. Top-Level Import Policy

To maintain code readability, clarity, and prevent structural spaghetti, inline full-path calls are strictly prohibited:

- **Prohibited:** `crate::foo::bar::function(...)`
- **Mandatory:** All modules, structs, enums, functions, and other dependencies must be explicitly imported at the top of the source file using standard `use crate::...;` statements.

By keeping all imports at the top of each file, dependencies remain transparent and easily trackable during architectural reviews.
