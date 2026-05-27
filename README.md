# belay

`belay` is a small Rust CLI for creating opinionated project folders and repos.

## Python Projects

```sh
belay py something
```

This creates `./something` as a uv-compatible Python package with:

- `pyproject.toml`
- `src/<module>/`
- `tests/`
- `.python-version`
- `uv.lock`
- `.git/` with initial branch `main`
- `ruff`, `ty`, and `pytest` added through `uv add --dev`
- `py.typed` for typed package distribution

Belay shells out to uv for the project setup:

```sh
uv init --lib --python <version> --name <name> --no-description --author-from none --vcs none --no-workspace <path>
uv add --dev ruff ty pytest
```

The default Python target is `3.13`:

```sh
belay py something --python 3.14
```

## Rust Projects

```sh
belay rs something
```

This creates `./something` as a Cargo binary project with:

- `Cargo.toml`
- `src/main.rs`
- `.git/`
- initial branch `main`
- `.gitignore`
- `README.md`
- `.editorconfig`

Belay shells out to Cargo for the initial repo setup:

```sh
cargo new --bin --vcs none --name <name> <path>
```

For every project type, Belay owns repository initialization and runs:

```sh
git init -b main
```

This keeps the initial branch policy independent of scaffold tool defaults and applies it to future project generators.

## Shell Integration

A binary cannot directly change its parent shell's current directory. `belay`
solves that by installing a shell function that delegates project creation to
the Rust binary, captures the new directory, and then runs `cd` in the shell.

```sh
belay shell install fish
belay shell install bash
belay shell install zsh
```

After restarting or sourcing the shell config:

```sh
belay py something
pwd
# .../something
```

```sh
belay rs something
pwd
# .../something
```

You can also print the function without installing it:

```sh
belay shell init zsh
```
