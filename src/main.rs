use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PYTHON: &str = "3.13";
const DEFAULT_BRANCH: &str = "main";
const BELAY_BEGIN: &str = "# >>> belay shell integration >>>";
const BELAY_END: &str = "# <<< belay shell integration <<<";
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_PINK: &str = "\x1b[38;2;231;89;154m";
const ANSI_PLUM: &str = "\x1b[38;2;181;72;157m";
const ANSI_PURPLE: &str = "\x1b[38;2;112;67;174m";
const ANSI_INDIGO: &str = "\x1b[38;2;69;72;169m";
const ANSI_DEEP_BLUE: &str = "\x1b[38;2;47;95;184m";

fn main() {
    if let Err(err) = run(env::args_os().skip(1).collect()) {
        let theme = Theme::stderr();
        eprintln!("{}: {err}", theme.paint("belay", Tone::Pink));
        std::process::exit(1);
    }
}

fn run(args: Vec<OsString>) -> Result<(), BelayError> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        print_help();
        return Ok(());
    };

    match command {
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        "-V" | "--version" | "version" => {
            let theme = Theme::stdout();
            println!(
                "{} {}",
                theme.paint("belay", Tone::Pink),
                theme.paint(VERSION, Tone::DeepBlue)
            );
            Ok(())
        }
        "py" => run_py(&args[1..]),
        "rs" => run_rs(&args[1..]),
        "shell" => run_shell(&args[1..]),
        other => Err(BelayError::usage(format!(
            "unknown command `{other}`\n\nRun `belay --help` for usage."
        ))),
    }
}

fn run_py(args: &[OsString]) -> Result<(), BelayError> {
    let mut project_name: Option<String> = None;
    let mut python = DEFAULT_PYTHON.to_string();
    let mut auto_shell = auto_shell_enabled();

    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        let Some(arg) = arg.to_str() else {
            return Err(BelayError::usage("arguments must be valid UTF-8"));
        };

        if arg == "-h" || arg == "--help" {
            print_py_help();
            return Ok(());
        } else if arg == "--no-shell" {
            auto_shell = false;
        } else if arg == "--python" {
            let Some(value) = iter.next().and_then(|value| value.to_str()) else {
                return Err(BelayError::usage(
                    "`--python` requires a version like `3.13`",
                ));
            };
            python = value.to_string();
        } else if let Some(value) = arg.strip_prefix("--python=") {
            python = value.to_string();
        } else if arg.starts_with('-') {
            return Err(BelayError::usage(format!("unknown option `{arg}`")));
        } else if project_name.is_none() {
            project_name = Some(arg.to_string());
        } else {
            return Err(BelayError::usage(
                "`belay py` accepts exactly one project name",
            ));
        }
    }

    let Some(project_name) = project_name else {
        return Err(BelayError::usage(
            "missing project name\n\nUsage: belay py <name>",
        ));
    };

    let spec = PythonProjectSpec::new(&project_name, &python)?;
    let project_dir = env::current_dir()?.join(&spec.directory_name);
    create_python_project(&project_dir, &spec)?;
    finish_project_creation(&project_dir, auto_shell)
}

fn run_rs(args: &[OsString]) -> Result<(), BelayError> {
    let mut project_name: Option<String> = None;
    let mut auto_shell = auto_shell_enabled();

    for arg in args {
        let Some(arg) = arg.to_str() else {
            return Err(BelayError::usage("arguments must be valid UTF-8"));
        };

        if arg == "-h" || arg == "--help" {
            print_rs_help();
            return Ok(());
        } else if arg == "--no-shell" {
            auto_shell = false;
        } else if arg.starts_with('-') {
            return Err(BelayError::usage(format!("unknown option `{arg}`")));
        } else if project_name.is_none() {
            project_name = Some(arg.to_string());
        } else {
            return Err(BelayError::usage(
                "`belay rs` accepts exactly one project name",
            ));
        }
    }

    let Some(project_name) = project_name else {
        return Err(BelayError::usage(
            "missing project name\n\nUsage: belay rs <name>",
        ));
    };

    let spec = RustProjectSpec::new(&project_name)?;
    let project_dir = env::current_dir()?.join(&spec.directory_name);
    create_rust_project(&project_dir, &spec)?;
    finish_project_creation(&project_dir, auto_shell)
}

fn auto_shell_enabled() -> bool {
    env::var_os("BELAY_AUTO_SHELL")
        .and_then(|value| value.into_string().ok())
        .map(|value| value != "0" && value != "false")
        .unwrap_or(true)
}

fn finish_project_creation(project_dir: &Path, auto_shell: bool) -> Result<(), BelayError> {
    finish_project_creation_with_git_runner(project_dir, auto_shell, &SystemGitRunner)
}

fn finish_project_creation_with_git_runner<R: GitRunner>(
    project_dir: &Path,
    auto_shell: bool,
    git_runner: &R,
) -> Result<(), BelayError> {
    initialize_git_repository_with_runner(project_dir, git_runner)?;

    let from_wrapper = env::var_os("BELAY_SHELL_WRAPPER").is_some();
    if from_wrapper {
        println!("{}", project_dir.display());
        return Ok(());
    }

    let stdout_theme = Theme::stdout();
    println!(
        "{} {}",
        stdout_theme.paint("created", Tone::Pink),
        stdout_theme.paint(project_dir.display(), Tone::DeepBlue)
    );

    if auto_shell {
        let stderr_theme = Theme::stderr();
        match detect_shell()
            .and_then(|shell| install_shell_integration(shell).map(|path| (shell, path)))
        {
            Ok((shell, path)) => {
                eprintln!(
                    "{} {} integration at {}; restart or source your shell config for automatic cd",
                    stderr_theme.paint("installed", Tone::Pink),
                    stderr_theme.paint(shell, Tone::Purple),
                    stderr_theme.paint(path.display(), Tone::DeepBlue)
                );
            }
            Err(err) => {
                eprintln!(
                    "{}: {err}\nRun `belay shell install` after setup.",
                    stderr_theme.paint("shell integration not installed automatically", Tone::Pink)
                );
            }
        }
    }

    let stderr_theme = Theme::stderr();
    eprintln!(
        "{}; run `cd {}` now",
        stderr_theme.paint("current process cannot cd the parent shell", Tone::Pink),
        stderr_theme.paint(project_dir.display(), Tone::DeepBlue)
    );
    Ok(())
}

fn run_shell(args: &[OsString]) -> Result<(), BelayError> {
    let Some(subcommand) = args.first().and_then(|arg| arg.to_str()) else {
        print_shell_help();
        return Ok(());
    };

    match subcommand {
        "-h" | "--help" | "help" => {
            print_shell_help();
            Ok(())
        }
        "init" => {
            if is_help_arg(args.get(1)) {
                print_shell_help();
                return Ok(());
            }
            let shell = shell_arg(args)?.unwrap_or(detect_shell()?);
            print!("{}", shell_function(shell));
            Ok(())
        }
        "install" => {
            if is_help_arg(args.get(1)) {
                print_shell_help();
                return Ok(());
            }
            let shell = shell_arg(args)?.unwrap_or(detect_shell()?);
            let path = install_shell_integration(shell)?;
            let theme = Theme::stdout();
            println!(
                "{} {} integration at {}",
                theme.paint("installed", Tone::Pink),
                theme.paint(shell, Tone::Purple),
                theme.paint(path.display(), Tone::DeepBlue)
            );
            Ok(())
        }
        other => Err(BelayError::usage(format!(
            "unknown shell command `{other}`\n\nRun `belay shell --help` for usage."
        ))),
    }
}

fn is_help_arg(arg: Option<&OsString>) -> bool {
    matches!(
        arg.and_then(|arg| arg.to_str()),
        Some("-h" | "--help" | "help")
    )
}

fn shell_arg(args: &[OsString]) -> Result<Option<Shell>, BelayError> {
    if args.len() > 2 {
        return Err(BelayError::usage(format!(
            "`belay shell {}` accepts at most one shell argument",
            args.first()
                .and_then(|arg| arg.to_str())
                .unwrap_or("<command>")
        )));
    }

    parse_shell(args.get(1).map(OsString::as_os_str))
}

fn parse_shell(value: Option<&OsStr>) -> Result<Option<Shell>, BelayError> {
    let Some(value) = value else {
        return Ok(None);
    };

    match value.to_str() {
        Some("bash") => Ok(Some(Shell::Bash)),
        Some("fish") => Ok(Some(Shell::Fish)),
        Some("zsh") => Ok(Some(Shell::Zsh)),
        Some(other) => Err(BelayError::usage(format!(
            "unsupported shell `{other}`; expected fish, bash, or zsh"
        ))),
        None => Err(BelayError::usage("shell name must be valid UTF-8")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PythonProjectSpec {
    directory_name: String,
    project_name: String,
    module_name: String,
    python_version: String,
    ruff_target: String,
}

impl PythonProjectSpec {
    fn new(name: &str, python_version: &str) -> Result<Self, BelayError> {
        validate_project_name(name)?;
        let module_name = module_name_for(name)?;
        let ruff_target = ruff_target_for(python_version)?;

        Ok(Self {
            directory_name: name.to_string(),
            project_name: name.to_string(),
            module_name,
            python_version: python_version.to_string(),
            ruff_target,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RustProjectSpec {
    directory_name: String,
    package_name: String,
}

impl RustProjectSpec {
    fn new(name: &str) -> Result<Self, BelayError> {
        validate_project_name(name)?;
        validate_rust_package_name(name)?;

        Ok(Self {
            directory_name: name.to_string(),
            package_name: name.to_string(),
        })
    }
}

fn validate_project_name(name: &str) -> Result<(), BelayError> {
    if name.is_empty() {
        return Err(BelayError::usage("project name cannot be empty"));
    }

    if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(BelayError::usage(
            "project name must be a single directory name, not a path",
        ));
    }

    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        return Err(BelayError::usage(
            "project name may only contain ASCII letters, numbers, underscores, dashes, and dots",
        ));
    }

    Ok(())
}

fn validate_rust_package_name(name: &str) -> Result<(), BelayError> {
    if name.contains('.') {
        return Err(BelayError::usage(
            "Rust project names may only contain ASCII letters, numbers, underscores, and dashes",
        ));
    }

    Ok(())
}

fn module_name_for(project_name: &str) -> Result<String, BelayError> {
    let module_name = project_name
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch == '-' || ch == '.' { '_' } else { ch })
        .collect::<String>();

    let mut chars = module_name.chars();
    let Some(first) = chars.next() else {
        return Err(BelayError::usage("project name cannot be empty"));
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(BelayError::usage(
            "project name must normalize to a Python module starting with a letter or underscore",
        ));
    }

    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(BelayError::usage(
            "project name must normalize to a valid Python module name",
        ));
    }

    Ok(module_name)
}

fn ruff_target_for(python_version: &str) -> Result<String, BelayError> {
    let Some((major, minor)) = python_version.split_once('.') else {
        return Err(BelayError::usage("python version must look like `3.13`"));
    };

    if major != "3" || minor.is_empty() || !minor.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(BelayError::usage("python version must look like `3.13`"));
    }

    Ok(format!("py3{minor}"))
}

fn create_python_project(project_dir: &Path, spec: &PythonProjectSpec) -> Result<(), BelayError> {
    if project_dir.exists() {
        return Err(BelayError::usage(format!(
            "{} already exists",
            project_dir.display()
        )));
    }

    create_python_project_with_runner(project_dir, spec, &SystemUvRunner)
}

fn create_rust_project(project_dir: &Path, spec: &RustProjectSpec) -> Result<(), BelayError> {
    if project_dir.exists() {
        return Err(BelayError::usage(format!(
            "{} already exists",
            project_dir.display()
        )));
    }

    create_rust_project_with_runner(project_dir, spec, &SystemCargoRunner)
}

fn create_python_project_with_runner<R: UvRunner>(
    project_dir: &Path,
    spec: &PythonProjectSpec,
    runner: &R,
) -> Result<(), BelayError> {
    let parent = project_dir
        .parent()
        .ok_or_else(|| BelayError::usage("project directory must have a parent"))?;

    runner.run_uv(parent, &uv_init_args(project_dir, spec))?;
    write_python_overlays(project_dir, spec)?;
    runner.run_uv(project_dir, &uv_add_dev_args())?;
    append_tool_config(project_dir, spec)?;

    Ok(())
}

fn create_rust_project_with_runner<R: CargoRunner>(
    project_dir: &Path,
    spec: &RustProjectSpec,
    runner: &R,
) -> Result<(), BelayError> {
    let parent = project_dir
        .parent()
        .ok_or_else(|| BelayError::usage("project directory must have a parent"))?;

    runner.run_cargo(parent, &cargo_new_args(project_dir, spec))?;
    write_rust_overlays(project_dir, spec)?;

    Ok(())
}

trait UvRunner {
    fn run_uv(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError>;
}

struct SystemUvRunner;

impl UvRunner for SystemUvRunner {
    fn run_uv(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
        run_uv_command(current_dir, args)
    }
}

trait CargoRunner {
    fn run_cargo(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError>;
}

struct SystemCargoRunner;

impl CargoRunner for SystemCargoRunner {
    fn run_cargo(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
        run_cargo_command(current_dir, args)
    }
}

trait GitRunner {
    fn run_git(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError>;
}

struct SystemGitRunner;

impl GitRunner for SystemGitRunner {
    fn run_git(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
        run_git_command(current_dir, args)
    }
}

fn uv_init_args(project_dir: &Path, spec: &PythonProjectSpec) -> Vec<OsString> {
    vec![
        OsString::from("init"),
        OsString::from("--lib"),
        OsString::from("--python"),
        OsString::from(&spec.python_version),
        OsString::from("--name"),
        OsString::from(&spec.project_name),
        OsString::from("--no-description"),
        OsString::from("--author-from"),
        OsString::from("none"),
        OsString::from("--vcs"),
        OsString::from("none"),
        OsString::from("--no-workspace"),
        project_dir.as_os_str().to_os_string(),
    ]
}

fn uv_add_dev_args() -> Vec<OsString> {
    vec![
        OsString::from("add"),
        OsString::from("--dev"),
        OsString::from("ruff"),
        OsString::from("ty"),
        OsString::from("pytest"),
    ]
}

fn cargo_new_args(project_dir: &Path, spec: &RustProjectSpec) -> Vec<OsString> {
    vec![
        OsString::from("new"),
        OsString::from("--bin"),
        OsString::from("--vcs"),
        OsString::from("none"),
        OsString::from("--name"),
        OsString::from(&spec.package_name),
        project_dir.as_os_str().to_os_string(),
    ]
}

fn git_init_args() -> Vec<OsString> {
    vec![
        OsString::from("init"),
        OsString::from("-b"),
        OsString::from(DEFAULT_BRANCH),
    ]
}

fn run_uv_command(current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
    run_command("uv", current_dir, args)
}

fn run_cargo_command(current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
    run_command("cargo", current_dir, args)
}

fn run_git_command(current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
    run_command("git", current_dir, args)
}

fn run_command(program: &str, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
    let display = format_command(program, args);
    let output = Command::new(program)
        .args(args)
        .current_dir(current_dir)
        .output()
        .map_err(|err| BelayError::usage(format!("failed to run `{display}`: {err}")))?;

    if output.status.success() {
        relay_command_output(&output);
        Ok(())
    } else {
        Err(BelayError::usage(command_failure_message(
            &display, &output,
        )))
    }
}

fn relay_command_output(output: &Output) {
    if !output.stdout.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
}

fn command_failure_message(display: &str, output: &Output) -> String {
    let mut message = format!("`{display}` failed with {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stdout.trim().is_empty() {
        message.push_str("\nstdout:\n");
        message.push_str(stdout.trim());
    }

    if !stderr.trim().is_empty() {
        message.push_str("\nstderr:\n");
        message.push_str(stderr.trim());
    }

    message
}

fn format_command(program: &str, args: &[OsString]) -> String {
    let mut parts = vec![program.to_string()];
    parts.extend(args.iter().map(|arg| shell_word(arg.as_os_str())));
    parts.join(" ")
}

fn shell_word(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'))
    {
        text.into_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

fn write_python_overlays(project_dir: &Path, spec: &PythonProjectSpec) -> Result<(), BelayError> {
    let module_dir = project_dir.join("src").join(&spec.module_name);
    let tests_dir = project_dir.join("tests");

    fs::create_dir_all(&module_dir)?;
    fs::create_dir_all(&tests_dir)?;

    write_file(project_dir.join(".editorconfig"), editorconfig())?;
    write_file(project_dir.join(".gitignore"), python_gitignore())?;
    write_file(project_dir.join("README.md"), python_readme(spec))?;
    write_file(module_dir.join("__init__.py"), init_py(spec))?;
    write_file(module_dir.join("py.typed"), "")?;
    write_file(
        tests_dir.join(format!("test_{}.py", spec.module_name)),
        test_py(spec),
    )?;

    Ok(())
}

fn write_rust_overlays(project_dir: &Path, spec: &RustProjectSpec) -> Result<(), BelayError> {
    write_file(project_dir.join(".editorconfig"), editorconfig())?;
    write_file(project_dir.join("README.md"), rust_readme(spec))?;
    append_gitignore_entries(&project_dir.join(".gitignore"), &["/target", ".DS_Store"])?;
    Ok(())
}

fn initialize_git_repository_with_runner<R: GitRunner>(
    project_dir: &Path,
    runner: &R,
) -> Result<(), BelayError> {
    if project_dir.join(".git").exists() {
        return Err(BelayError::usage(
            "project scaffold initialized Git itself; generators must leave repository initialization to belay so the initial branch is `main`",
        ));
    }

    runner.run_git(project_dir, &git_init_args())
}

fn append_tool_config(project_dir: &Path, spec: &PythonProjectSpec) -> Result<(), BelayError> {
    let path = project_dir.join("pyproject.toml");
    let mut pyproject = fs::read_to_string(&path)?;

    if pyproject.contains("[tool.ruff]") || pyproject.contains("[tool.ty]") {
        return Ok(());
    }

    if !pyproject.ends_with('\n') {
        pyproject.push('\n');
    }
    pyproject.push('\n');
    pyproject.push_str(&tool_config_toml(spec));

    fs::write(path, pyproject)?;
    Ok(())
}

fn write_file(path: PathBuf, contents: impl AsRef<[u8]>) -> io::Result<()> {
    fs::write(path, contents)
}

fn tool_config_toml(spec: &PythonProjectSpec) -> String {
    format!(
        r#"[tool.ruff]
line-length = 100
target-version = "{ruff_target}"

[tool.ruff.lint]
select = [
    "E",
    "F",
    "I",
    "B",
    "UP",
    "ANN",
    "ASYNC",
    "C4",
    "RUF",
]

[tool.ruff.format]
quote-style = "double"
indent-style = "space"

[tool.pytest.ini_options]
testpaths = ["tests"]

[tool.ty]
"#,
        ruff_target = spec.ruff_target,
    )
}

fn python_readme(spec: &PythonProjectSpec) -> String {
    format!(
        r#"# {project_name}

## Development

```sh
uv sync --dev
uv run ruff format .
uv run ruff check .
uv run ty check
uv run pytest
```
"#,
        project_name = spec.project_name
    )
}

fn rust_readme(spec: &RustProjectSpec) -> String {
    format!(
        r#"# {project_name}

## Development

```sh
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run
```
"#,
        project_name = spec.package_name
    )
}

fn init_py(spec: &PythonProjectSpec) -> String {
    format!(
        r#""""{project_name}."""

__all__ = ["greet"]


def greet(name: str) -> str:
    return f"Hello, {{name}}!"
"#,
        project_name = spec.project_name
    )
}

fn test_py(spec: &PythonProjectSpec) -> String {
    format!(
        r#"from {module_name} import greet


def test_greet() -> None:
    assert greet("belay") == "Hello, belay!"
"#,
        module_name = spec.module_name
    )
}

fn editorconfig() -> &'static str {
    r#"root = true

[*]
charset = utf-8
end_of_line = lf
indent_style = space
indent_size = 4
insert_final_newline = true
trim_trailing_whitespace = true

[*.toml]
indent_size = 4
"#
}

fn python_gitignore() -> &'static str {
    r#".DS_Store
.venv/
__pycache__/
*.py[cod]
.pytest_cache/
.ruff_cache/
.ty/
build/
dist/
*.egg-info/
"#
}

fn append_gitignore_entries(path: &Path, entries: &[&str]) -> io::Result<()> {
    let mut current = fs::read_to_string(path).unwrap_or_default();

    if !current.is_empty() && !current.ends_with('\n') {
        current.push('\n');
    }

    let mut changed = false;
    for entry in entries {
        if current.lines().any(|line| line == *entry) {
            continue;
        }
        current.push_str(entry);
        current.push('\n');
        changed = true;
    }

    if changed {
        fs::write(path, current)?;
    }

    Ok(())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Shell {
    Bash,
    Fish,
    Zsh,
}

impl std::fmt::Display for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bash => f.write_str("bash"),
            Self::Fish => f.write_str("fish"),
            Self::Zsh => f.write_str("zsh"),
        }
    }
}

fn detect_shell() -> Result<Shell, BelayError> {
    let Some(shell) = env::var_os("SHELL") else {
        return Err(BelayError::usage(
            "could not detect shell because SHELL is unset; pass fish, bash, or zsh",
        ));
    };

    let shell_name = Path::new(&shell)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();

    match shell_name {
        "bash" => Ok(Shell::Bash),
        "fish" => Ok(Shell::Fish),
        "zsh" => Ok(Shell::Zsh),
        other => Err(BelayError::usage(format!(
            "unsupported shell `{other}`; pass fish, bash, or zsh"
        ))),
    }
}

fn install_shell_integration(shell: Shell) -> Result<PathBuf, BelayError> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| BelayError::usage("HOME is unset"))?;
    install_shell_integration_at(shell, &home)
}

fn install_shell_integration_at(shell: Shell, home: &Path) -> Result<PathBuf, BelayError> {
    match shell {
        Shell::Fish => {
            let path = env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("fish")
                .join("conf.d")
                .join("belay.fish");
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, shell_function(shell))?;
            Ok(path)
        }
        Shell::Bash => install_managed_block(home.join(".bashrc"), shell),
        Shell::Zsh => install_managed_block(home.join(".zshrc"), shell),
    }
}

fn install_managed_block(path: PathBuf, shell: Shell) -> Result<PathBuf, BelayError> {
    let block = managed_block(shell);
    let current = fs::read_to_string(&path).unwrap_or_default();
    let next = replace_or_append_block(&current, &block);
    fs::write(&path, next)?;
    Ok(path)
}

fn managed_block(shell: Shell) -> String {
    format!("{BELAY_BEGIN}\n{}{BELAY_END}\n", shell_function(shell))
}

fn replace_or_append_block(current: &str, block: &str) -> String {
    let Some(start) = current.find(BELAY_BEGIN) else {
        let mut next = current.to_string();
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        if !next.is_empty() {
            next.push('\n');
        }
        next.push_str(block);
        return next;
    };

    let Some(end_relative) = current[start..].find(BELAY_END) else {
        let mut next = current.to_string();
        if !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(block);
        return next;
    };

    let end = start + end_relative + BELAY_END.len();
    let mut next = String::new();
    next.push_str(&current[..start]);
    next.push_str(block.trim_end());
    next.push_str(&current[end..]);
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next
}

fn shell_function(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash | Shell::Zsh => {
            r#"belay() {
    case "$1" in
        py|rs)
            local target
            target="$(BELAY_SHELL_WRAPPER=1 command belay "$@")"
            local status=$?
            if [ "$status" -eq 0 ] && [ -n "$target" ]; then
                cd "$target" || return $?
            fi
            return "$status"
            ;;
    esac

    command belay "$@"
}
"#
        }
        Shell::Fish => {
            r#"function belay
    switch "$argv[1]"
    case py rs
        set -l target (env BELAY_SHELL_WRAPPER=1 command belay $argv)
        set -l command_status $status
        if test $command_status -eq 0; and test -n "$target"
            cd "$target"; or return $status
        end
        return $command_status
    end

    command belay $argv
end
"#
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tone {
    Pink,
    Plum,
    Purple,
    Indigo,
    DeepBlue,
}

impl Tone {
    fn ansi(self) -> &'static str {
        match self {
            Self::Pink => ANSI_PINK,
            Self::Plum => ANSI_PLUM,
            Self::Purple => ANSI_PURPLE,
            Self::Indigo => ANSI_INDIGO,
            Self::DeepBlue => ANSI_DEEP_BLUE,
        }
    }
}

const BANNER_LETTERS: [[&str; 6]; 5] = [
    [
        "██████╗ ",
        "██╔══██╗",
        "██████╔╝",
        "██╔══██╗",
        "██████╔╝",
        "╚═════╝ ",
    ],
    [
        "███████╗",
        "██╔════╝",
        "█████╗  ",
        "██╔══╝  ",
        "███████╗",
        "╚══════╝",
    ],
    [
        "██╗     ",
        "██║     ",
        "██║     ",
        "██║     ",
        "███████╗",
        "╚══════╝",
    ],
    [
        " █████╗ ",
        "██╔══██╗",
        "███████║",
        "██╔══██║",
        "██║  ██║",
        "╚═╝  ╚═╝",
    ],
    [
        "██╗   ██╗",
        "╚██╗ ██╔╝",
        " ╚████╔╝ ",
        "  ╚██╔╝  ",
        "   ██║   ",
        "   ╚═╝   ",
    ],
];
const BANNER_TONES: [Tone; 5] = [
    Tone::Pink,
    Tone::Plum,
    Tone::Purple,
    Tone::Indigo,
    Tone::DeepBlue,
];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Theme {
    enabled: bool,
}

impl Theme {
    fn stdout() -> Self {
        Self::for_terminal(io::stdout().is_terminal())
    }

    fn stderr() -> Self {
        Self::for_terminal(io::stderr().is_terminal())
    }

    fn for_terminal(is_terminal: bool) -> Self {
        Self {
            enabled: is_terminal && env::var_os("NO_COLOR").is_none(),
        }
    }

    fn paint(self, value: impl std::fmt::Display, tone: Tone) -> String {
        if self.enabled {
            format!("{}{value}{ANSI_RESET}", tone.ansi())
        } else {
            value.to_string()
        }
    }

    fn heading(self, value: &str) -> String {
        if self.enabled {
            format!("{ANSI_BOLD}{ANSI_PINK}{value}{ANSI_RESET}")
        } else {
            value.to_string()
        }
    }
}

fn render_banner(theme: Theme) -> String {
    let mut banner = String::new();
    for row in 0..BANNER_LETTERS[0].len() {
        for (letter, tone) in BANNER_LETTERS.iter().zip(BANNER_TONES) {
            banner.push_str(&theme.paint(letter[row], tone));
        }
        banner.push('\n');
    }
    banner
}

fn print_banner(theme: Theme) {
    println!("{}", render_banner(theme));
}

fn print_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!(
        r#"{brand} {version}

{usage}:
  {brand} {py} <name> [{python} <version>] [{no_shell}]
  {brand} {rs} <name> [{no_shell}]
  {brand} {shell} {init} [fish|bash|zsh]
  {brand} {shell} {install} [fish|bash|zsh]

{commands}:
  {py}       Create a typed uv Python project and print/cd to it through shell integration
  {rs}       Create a Cargo Rust project with Git and print/cd to it through shell integration
  {shell}    Print or install shell integration for automatic cd
"#,
        brand = theme.paint("belay", Tone::Pink),
        version = theme.paint(VERSION, Tone::DeepBlue),
        usage = theme.heading("Usage"),
        commands = theme.heading("Commands"),
        py = theme.paint("py", Tone::Purple),
        rs = theme.paint("rs", Tone::Purple),
        shell = theme.paint("shell", Tone::Purple),
        init = theme.paint("init", Tone::Purple),
        install = theme.paint("install", Tone::Purple),
        python = theme.paint("--python", Tone::Purple),
        no_shell = theme.paint("--no-shell", Tone::Purple),
    );
}

fn print_py_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!(
        r#"{usage}:
  {brand} {py} <name> [{python} <version>] [{no_shell}]

{creates} a new directory in the current working directory with:
  pyproject.toml, src/<module>, tests, README.md, .python-version
  uv init --lib, uv add --dev ruff ty pytest, a py.typed marker, and Git on main

{options}:
  {python} <version>  Python lower bound and .python-version value (default: {default_python})
  {no_shell}          Skip automatic shell integration provisioning
"#,
        usage = theme.heading("Usage"),
        brand = theme.paint("belay", Tone::Pink),
        py = theme.paint("py", Tone::Purple),
        python = theme.paint("--python", Tone::Purple),
        no_shell = theme.paint("--no-shell", Tone::Purple),
        creates = theme.heading("Creates"),
        options = theme.heading("Options"),
        default_python = theme.paint(DEFAULT_PYTHON, Tone::DeepBlue),
    );
}

fn print_rs_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!(
        r#"{usage}:
  {brand} {rs} <name> [{no_shell}]

{creates} a new directory in the current working directory with:
  cargo new --bin --vcs none, Git initialized on main, README.md, .editorconfig, and .gitignore

{options}:
  {no_shell}  Skip automatic shell integration provisioning
"#,
        usage = theme.heading("Usage"),
        brand = theme.paint("belay", Tone::Pink),
        rs = theme.paint("rs", Tone::Purple),
        no_shell = theme.paint("--no-shell", Tone::Purple),
        creates = theme.heading("Creates"),
        options = theme.heading("Options"),
    );
}

fn print_shell_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!(
        r#"{usage}:
  {brand} {shell} {init} [fish|bash|zsh]
  {brand} {shell} {install} [fish|bash|zsh]

{init_quoted} prints the function for manual shell setup.
{install_quoted} writes a managed integration block/file so `{brand} {py} <name>` and `{brand} {rs} <name>` can cd into the new directory.
"#,
        usage = theme.heading("Usage"),
        brand = theme.paint("belay", Tone::Pink),
        shell = theme.paint("shell", Tone::Purple),
        init = theme.paint("init", Tone::Purple),
        install = theme.paint("install", Tone::Purple),
        init_quoted = theme.paint("`init`", Tone::Purple),
        install_quoted = theme.paint("`install`", Tone::Purple),
        py = theme.paint("py", Tone::Purple),
        rs = theme.paint("rs", Tone::Purple),
    );
}

#[derive(Debug)]
struct BelayError {
    message: String,
}

impl BelayError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for BelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BelayError {}

impl From<io::Error> for BelayError {
    fn from(value: io::Error) -> Self {
        Self {
            message: value.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct FakeUvRunner {
        calls: RefCell<Vec<(PathBuf, Vec<OsString>)>>,
    }

    #[derive(Default)]
    struct FakeCargoRunner {
        calls: RefCell<Vec<(PathBuf, Vec<OsString>)>>,
    }

    #[derive(Default)]
    struct FakeGitRunner {
        calls: RefCell<Vec<(PathBuf, Vec<OsString>)>>,
    }

    impl UvRunner for FakeUvRunner {
        fn run_uv(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
            self.calls
                .borrow_mut()
                .push((current_dir.to_path_buf(), args.to_vec()));

            match args.first().and_then(|arg| arg.to_str()) {
                Some("init") => fake_uv_init(args),
                Some("add") => fake_uv_add(current_dir),
                Some(command) => Err(BelayError::usage(format!(
                    "unexpected fake uv command `{command}`"
                ))),
                None => Err(BelayError::usage("missing fake uv command")),
            }
        }
    }

    impl CargoRunner for FakeCargoRunner {
        fn run_cargo(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
            self.calls
                .borrow_mut()
                .push((current_dir.to_path_buf(), args.to_vec()));

            match args.first().and_then(|arg| arg.to_str()) {
                Some("new") => fake_cargo_new(args),
                Some(command) => Err(BelayError::usage(format!(
                    "unexpected fake cargo command `{command}`"
                ))),
                None => Err(BelayError::usage("missing fake cargo command")),
            }
        }
    }

    impl GitRunner for FakeGitRunner {
        fn run_git(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
            self.calls
                .borrow_mut()
                .push((current_dir.to_path_buf(), args.to_vec()));

            if args == git_init_args() {
                fake_git_init(current_dir)
            } else {
                Err(BelayError::usage("unexpected fake git command"))
            }
        }
    }

    fn fake_uv_init(args: &[OsString]) -> Result<(), BelayError> {
        let project_dir = PathBuf::from(
            args.last()
                .ok_or_else(|| BelayError::usage("missing uv init path"))?,
        );
        let name = arg_after(args, "--name")?;
        let python = arg_after(args, "--python")?;
        let module = module_name_for(&name)?;

        fs::create_dir_all(project_dir.join("src").join(&module))?;
        fs::write(
            project_dir.join("pyproject.toml"),
            format!(
                r#"[project]
name = "{name}"
version = "0.1.0"
readme = "README.md"
requires-python = ">={python}"
dependencies = []

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
"#
            ),
        )?;
        fs::write(project_dir.join("README.md"), format!("# {name}\n"))?;
        fs::write(project_dir.join(".python-version"), format!("{python}\n"))?;
        fs::write(
            project_dir.join("src").join(&module).join("__init__.py"),
            "def hello() -> str:\n    return \"Hello\"\n",
        )?;
        fs::write(project_dir.join("src").join(&module).join("py.typed"), "")?;

        Ok(())
    }

    fn fake_uv_add(current_dir: &Path) -> Result<(), BelayError> {
        let pyproject_path = current_dir.join("pyproject.toml");
        let mut pyproject = fs::read_to_string(&pyproject_path)?;
        pyproject.push_str(
            r#"
[dependency-groups]
dev = [
    "pytest>=0",
    "ruff>=0",
    "ty>=0",
]
"#,
        );
        fs::write(pyproject_path, pyproject)?;
        fs::write(current_dir.join("uv.lock"), "")?;

        Ok(())
    }

    fn fake_cargo_new(args: &[OsString]) -> Result<(), BelayError> {
        let project_dir = PathBuf::from(
            args.last()
                .ok_or_else(|| BelayError::usage("missing cargo new path"))?,
        );
        let name = arg_after(args, "--name")?;

        fs::create_dir_all(project_dir.join("src"))?;
        fs::write(
            project_dir.join("Cargo.toml"),
            format!(
                r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
"#
            ),
        )?;
        fs::write(
            project_dir.join("src").join("main.rs"),
            r#"fn main() {
    println!("Hello, world!");
}
"#,
        )?;

        Ok(())
    }

    fn fake_git_init(project_dir: &Path) -> Result<(), BelayError> {
        fs::create_dir_all(project_dir.join(".git"))?;
        fs::write(
            project_dir.join(".git").join("HEAD"),
            format!("ref: refs/heads/{DEFAULT_BRANCH}\n"),
        )?;
        Ok(())
    }

    fn arg_after(args: &[OsString], flag: &str) -> Result<String, BelayError> {
        args.windows(2)
            .find_map(|window| {
                (window[0] == flag).then(|| window[1].to_string_lossy().into_owned())
            })
            .ok_or_else(|| BelayError::usage(format!("missing fake uv arg `{flag}`")))
    }

    #[test]
    fn enabled_theme_uses_the_belay_palette() {
        let theme = Theme { enabled: true };
        let banner = render_banner(theme);

        assert_eq!(
            theme.paint("belay", Tone::Pink),
            format!("{ANSI_PINK}belay{ANSI_RESET}")
        );
        assert_eq!(
            theme.paint("py", Tone::Purple),
            format!("{ANSI_PURPLE}py{ANSI_RESET}")
        );
        assert_eq!(
            theme.heading("Usage"),
            format!("{ANSI_BOLD}{ANSI_PINK}Usage{ANSI_RESET}")
        );
        assert!(banner.contains(ANSI_PINK));
        assert!(banner.contains(ANSI_PLUM));
        assert!(banner.contains(ANSI_PURPLE));
        assert!(banner.contains(ANSI_INDIGO));
        assert!(banner.contains(ANSI_DEEP_BLUE));
    }

    #[test]
    fn disabled_theme_banner_and_shell_functions_stay_plain() {
        let theme = Theme { enabled: false };
        let banner = render_banner(theme);

        assert_eq!(theme.paint("/tmp/project", Tone::DeepBlue), "/tmp/project");
        assert_eq!(theme.heading("Usage"), "Usage");
        assert!(banner.starts_with("██████╗ ███████╗██╗"));
        assert!(!banner.contains('\u{1b}'));
        assert!(!shell_function(Shell::Zsh).contains('\u{1b}'));
    }

    #[test]
    fn normalizes_python_module_names() {
        let spec = PythonProjectSpec::new("My-App.Tools", "3.13").unwrap();
        assert_eq!(spec.module_name, "my_app_tools");
        assert_eq!(spec.ruff_target, "py313");
    }

    #[test]
    fn rejects_names_that_cannot_be_modules() {
        assert!(PythonProjectSpec::new("123-app", "3.13").is_err());
        assert!(PythonProjectSpec::new("bad/path", "3.13").is_err());
        assert!(PythonProjectSpec::new("bad name", "3.13").is_err());
    }

    #[test]
    fn rejects_rust_names_with_dots() {
        assert!(RustProjectSpec::new("bad.name").is_err());
    }

    #[test]
    fn creates_python_project_layout() {
        let root = unique_temp_dir();
        fs::create_dir(&root).unwrap();
        let project_dir = root.join("sample-app");
        let spec = PythonProjectSpec::new("sample-app", "3.13").unwrap();
        let runner = FakeUvRunner::default();
        let git_runner = FakeGitRunner::default();

        create_python_project_with_runner(&project_dir, &spec, &runner).unwrap();
        finish_project_creation_with_git_runner(&project_dir, false, &git_runner).unwrap();

        assert!(project_dir.join("pyproject.toml").exists());
        assert!(project_dir.join(".git").exists());
        assert!(project_dir
            .join("src")
            .join("sample_app")
            .join("__init__.py")
            .exists());
        assert!(project_dir
            .join("src")
            .join("sample_app")
            .join("py.typed")
            .exists());
        assert!(project_dir
            .join("tests")
            .join("test_sample_app.py")
            .exists());
        assert!(project_dir.join("uv.lock").exists());

        let head = fs::read_to_string(project_dir.join(".git").join("HEAD")).unwrap();
        assert_eq!(head, "ref: refs/heads/main\n");

        let pyproject = fs::read_to_string(project_dir.join("pyproject.toml")).unwrap();
        assert!(pyproject.contains("build-backend = \"hatchling.build\""));
        assert!(pyproject.contains("\"ruff>=0\""));
        assert!(pyproject.contains("\"ty>=0\""));
        assert!(pyproject.contains("[tool.ruff]"));
        assert!(pyproject.contains("[tool.ty]"));

        let init_py = fs::read_to_string(
            project_dir
                .join("src")
                .join("sample_app")
                .join("__init__.py"),
        )
        .unwrap();
        assert!(init_py.contains("def greet(name: str) -> str:"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, uv_init_args(&project_dir, &spec));
        assert_eq!(calls[1].0, project_dir);
        assert_eq!(calls[1].1, uv_add_dev_args());

        let git_calls = git_runner.calls.borrow();
        assert_eq!(git_calls.len(), 1);
        assert_eq!(git_calls[0].0, project_dir);
        assert_eq!(git_calls[0].1, git_init_args());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creates_rust_project_layout() {
        let root = unique_temp_dir();
        fs::create_dir(&root).unwrap();
        let project_dir = root.join("sample-app");
        let spec = RustProjectSpec::new("sample-app").unwrap();
        let runner = FakeCargoRunner::default();
        let git_runner = FakeGitRunner::default();

        create_rust_project_with_runner(&project_dir, &spec, &runner).unwrap();
        finish_project_creation_with_git_runner(&project_dir, false, &git_runner).unwrap();

        assert!(project_dir.join(".git").exists());
        assert!(project_dir.join(".gitignore").exists());
        assert!(project_dir.join("Cargo.toml").exists());
        assert!(project_dir.join("README.md").exists());
        assert!(project_dir.join(".editorconfig").exists());
        assert!(project_dir.join("src").join("main.rs").exists());

        let cargo_toml = fs::read_to_string(project_dir.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("name = \"sample-app\""));

        let gitignore = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
        assert!(gitignore.contains("/target"));
        assert!(gitignore.contains(".DS_Store"));

        let head = fs::read_to_string(project_dir.join(".git").join("HEAD")).unwrap();
        assert_eq!(head, "ref: refs/heads/main\n");

        let readme = fs::read_to_string(project_dir.join("README.md")).unwrap();
        assert!(readme.contains("cargo clippy --all-targets --all-features -- -D warnings"));

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, cargo_new_args(&project_dir, &spec));

        let git_calls = git_runner.calls.borrow();
        assert_eq!(git_calls.len(), 1);
        assert_eq!(git_calls[0].0, project_dir);
        assert_eq!(git_calls[0].1, git_init_args());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_generators_that_initialize_git_themselves() {
        let root = unique_temp_dir();
        let project_dir = root.join("sample-app");
        fs::create_dir_all(project_dir.join(".git")).unwrap();
        let runner = FakeGitRunner::default();

        let err = initialize_git_repository_with_runner(&project_dir, &runner).unwrap_err();

        assert!(err
            .to_string()
            .contains("generators must leave repository initialization to belay"));
        assert!(runner.calls.borrow().is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shell_blocks_are_replaced_idempotently() {
        let old = format!("before\n\n{BELAY_BEGIN}\nold\n{BELAY_END}\nafter\n");
        let new = replace_or_append_block(&old, &managed_block(Shell::Zsh));

        assert!(new.contains("before"));
        assert!(new.contains("after"));
        assert!(new.contains("BELAY_SHELL_WRAPPER=1"));
        assert!(new.contains("py|rs"));
        assert!(!new.contains("\nold\n"));
        assert_eq!(new.matches(BELAY_BEGIN).count(), 1);
    }

    #[test]
    fn shell_arg_rejects_extra_values() {
        let args = vec![
            OsString::from("install"),
            OsString::from("zsh"),
            OsString::from("extra"),
        ];

        assert!(shell_arg(&args).is_err());
    }

    #[test]
    fn installs_fish_integration_under_config_home() {
        let root = unique_temp_dir();
        let home = root.join("home");
        let config = root.join("xdg");
        fs::create_dir_all(&home).unwrap();
        env::set_var("XDG_CONFIG_HOME", &config);

        let path = install_shell_integration_at(Shell::Fish, &home).unwrap();

        assert_eq!(path, config.join("fish").join("conf.d").join("belay.fish"));
        let script = fs::read_to_string(path).unwrap();
        assert!(script.contains("function belay"));
        assert!(script.contains("BELAY_SHELL_WRAPPER=1"));

        env::remove_var("XDG_CONFIG_HOME");
        fs::remove_dir_all(root).unwrap();
    }

    fn unique_temp_dir() -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("belay-test-{}-{nanos}-{id}", std::process::id()))
    }
}
