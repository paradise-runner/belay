use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::process::Stdio;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PYTHON: &str = "3.13";
const DEFAULT_BRANCH: &str = "main";
const DEFAULT_COBRA_VERSION: &str = "v1.8.1";
const BELAY_BEGIN: &str = "# >>> belay shell integration >>>";
const BELAY_END: &str = "# <<< belay shell integration <<<";
const SGR_RESET: &str = "\x1b[0m";
const SGR_BOLD: &str = "\x1b[1m";
const SGR_NOT_BOLD: &str = "\x1b[22m";
const SGR_PINK: &str = "\x1b[38;2;231;89;154m";
const SGR_PLUM: &str = "\x1b[38;2;201;80;177m";
const SGR_VIOLET: &str = "\x1b[38;2;153;102;231m";
const SGR_INDIGO: &str = "\x1b[38;2;101;112;217m";
const SGR_BLUE: &str = "\x1b[38;2;69;147;230m";
const SGR_DARK_PINK: &str = "\x1b[38;2;176;30;101m";
const SGR_DARK_PLUM: &str = "\x1b[38;2;142;37;124m";
const SGR_PURPLE: &str = "\x1b[38;2;112;67;174m";
const SGR_DARK_INDIGO: &str = "\x1b[38;2;62;69;148m";
const SGR_DARK_BLUE: &str = "\x1b[38;2;30;86;163m";
const SGR_WHITE: &str = "\x1b[38;2;255;255;255m";
const SGR_BLACK: &str = "\x1b[38;2;0;0;0m";
const OSC_BACKGROUND_QUERY: &[u8] = b"\x1b]11;?\x1b\\";
static TERMINAL_BACKGROUND: OnceLock<Background> = OnceLock::new();

fn main() {
    if let Err(err) = run(env::args_os().skip(1).collect()) {
        let theme = Theme::stderr();
        eprintln!(
            "{}",
            theme.message(format!("{}: {err}", theme.accent("belay")))
        );
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
                "{}",
                theme.message(format!("{} {VERSION}", theme.accent("belay")))
            );
            Ok(())
        }
        "py" => run_py(&args[1..]),
        "rs-cli" => run_rs_cli(&args[1..]),
        "go-cli" => run_go_cli(&args[1..]),
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

fn run_rs_cli(args: &[OsString]) -> Result<(), BelayError> {
    let Some((project_name, auto_shell)) = parse_name_and_shell_option(args, "rs-cli")? else {
        return Ok(());
    };

    let spec = RustCliProjectSpec::new(&project_name)?;
    let project_dir = env::current_dir()?.join(&spec.directory_name);
    create_rust_cli_project(&project_dir, &spec)?;
    finish_project_creation(&project_dir, auto_shell)
}

fn run_go_cli(args: &[OsString]) -> Result<(), BelayError> {
    let Some((project_name, auto_shell)) = parse_name_and_shell_option(args, "go-cli")? else {
        return Ok(());
    };

    let spec = GoCliProjectSpec::new(&project_name)?;
    let project_dir = env::current_dir()?.join(&spec.directory_name);
    create_go_cli_project(&project_dir, &spec)?;
    finish_project_creation(&project_dir, auto_shell)
}

fn parse_name_and_shell_option(
    args: &[OsString],
    command_name: &str,
) -> Result<Option<(String, bool)>, BelayError> {
    let mut project_name: Option<String> = None;
    let mut auto_shell = auto_shell_enabled();

    for arg in args {
        let Some(arg) = arg.to_str() else {
            return Err(BelayError::usage("arguments must be valid UTF-8"));
        };

        if arg == "-h" || arg == "--help" {
            match command_name {
                "rs-cli" => print_rs_cli_help(),
                "go-cli" => print_go_cli_help(),
                _ => {}
            }
            return Ok(None);
        } else if arg == "--no-shell" {
            auto_shell = false;
        } else if arg.starts_with('-') {
            return Err(BelayError::usage(format!("unknown option `{arg}`")));
        } else if project_name.is_none() {
            project_name = Some(arg.to_string());
        } else {
            return Err(BelayError::usage(format!(
                "`belay {command_name}` accepts exactly one project name"
            )));
        }
    }

    let Some(project_name) = project_name else {
        return Err(BelayError::usage(format!(
            "missing project name\n\nUsage: belay {command_name} <name>"
        )));
    };

    Ok(Some((project_name, auto_shell)))
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
        "{}",
        stdout_theme.message(format!(
            "{} {}",
            stdout_theme.accent("created"),
            project_dir.display()
        ))
    );

    if auto_shell {
        let stderr_theme = Theme::stderr();
        match detect_shell()
            .and_then(|shell| install_shell_integration(shell).map(|path| (shell, path)))
        {
            Ok((shell, path)) => {
                eprintln!(
                    "{}",
                    stderr_theme.message(format!(
                    "{} {} integration at {}; restart or source your shell config for automatic cd",
                    stderr_theme.accent("installed"),
                    stderr_theme.accent(shell),
                    path.display()
                ))
                );
            }
            Err(err) => {
                eprintln!(
                    "{}",
                    stderr_theme.message(format!(
                        "{}: {err}\nRun `belay shell install` after setup.",
                        stderr_theme.accent("shell integration not installed automatically")
                    ))
                );
            }
        }
    }

    let stderr_theme = Theme::stderr();
    eprintln!(
        "{}",
        stderr_theme.message(format!(
            "{}; run `cd {}` now",
            stderr_theme.accent("current process cannot cd the parent shell"),
            project_dir.display()
        ))
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
                "{}",
                theme.message(format!(
                    "{} {} integration at {}",
                    theme.accent("installed"),
                    theme.accent(shell),
                    path.display()
                ))
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
struct RustCliProjectSpec {
    directory_name: String,
    package_name: String,
    crate_name: String,
    command_name: String,
}

impl RustCliProjectSpec {
    fn new(name: &str) -> Result<Self, BelayError> {
        validate_project_name(name)?;
        let command_name = cli_slug_for(name)?;
        let crate_name = command_name.replace('-', "_");

        Ok(Self {
            directory_name: name.to_string(),
            package_name: command_name.clone(),
            crate_name,
            command_name,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GoCliProjectSpec {
    directory_name: String,
    module_name: String,
    command_name: String,
}

impl GoCliProjectSpec {
    fn new(name: &str) -> Result<Self, BelayError> {
        validate_project_name(name)?;
        let command_name = cli_slug_for(name)?;

        Ok(Self {
            directory_name: name.to_string(),
            module_name: command_name.clone(),
            command_name,
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

fn cli_slug_for(project_name: &str) -> Result<String, BelayError> {
    let slug = project_name
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch == '_' || ch == '.' { '-' } else { ch })
        .collect::<String>();

    let mut chars = slug.chars();
    let Some(first) = chars.next() else {
        return Err(BelayError::usage("project name cannot be empty"));
    };

    if !first.is_ascii_alphabetic() {
        return Err(BelayError::usage(
            "CLI project names must normalize to a name starting with a letter",
        ));
    }

    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-') {
        return Err(BelayError::usage(
            "CLI project names must normalize to lowercase letters, numbers, and dashes",
        ));
    }

    Ok(slug)
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

fn create_rust_cli_project(
    project_dir: &Path,
    spec: &RustCliProjectSpec,
) -> Result<(), BelayError> {
    if project_dir.exists() {
        return Err(BelayError::usage(format!(
            "{} already exists",
            project_dir.display()
        )));
    }

    let src_dir = project_dir.join("src");
    let tests_dir = project_dir.join("tests");

    fs::create_dir_all(&src_dir)?;
    fs::create_dir_all(&tests_dir)?;

    write_file(project_dir.join(".editorconfig"), editorconfig())?;
    write_file(project_dir.join(".gitignore"), rust_gitignore())?;
    write_file(project_dir.join("Cargo.toml"), rust_cli_cargo_toml(spec))?;
    write_file(project_dir.join("README.md"), rust_cli_readme(spec))?;
    write_file(src_dir.join("lib.rs"), rust_cli_lib_rs())?;
    write_file(src_dir.join("main.rs"), rust_cli_main_rs(spec))?;
    write_file(tests_dir.join("smoke.rs"), rust_cli_smoke_test(spec))?;

    Ok(())
}

fn create_go_cli_project(project_dir: &Path, spec: &GoCliProjectSpec) -> Result<(), BelayError> {
    if project_dir.exists() {
        return Err(BelayError::usage(format!(
            "{} already exists",
            project_dir.display()
        )));
    }

    let cmd_dir = project_dir.join("cmd");
    fs::create_dir_all(&cmd_dir)?;

    write_file(project_dir.join(".editorconfig"), editorconfig())?;
    write_file(project_dir.join(".gitignore"), go_gitignore())?;
    write_file(project_dir.join("go.mod"), go_cli_go_mod(spec))?;
    write_file(project_dir.join("README.md"), go_cli_readme(spec))?;
    write_file(project_dir.join("main.go"), go_cli_main_go(spec))?;
    write_file(cmd_dir.join("root.go"), go_cli_root_go(spec))?;
    write_file(cmd_dir.join("root_test.go"), go_cli_root_test_go())?;

    Ok(())
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

trait UvRunner {
    fn run_uv(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError>;
}

struct SystemUvRunner;

impl UvRunner for SystemUvRunner {
    fn run_uv(&self, current_dir: &Path, args: &[OsString]) -> Result<(), BelayError> {
        run_uv_command(current_dir, args)
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

fn rust_gitignore() -> &'static str {
    r#".DS_Store
/target/
"#
}

fn go_gitignore() -> &'static str {
    r#".DS_Store
/bin/
.coverprofile
"#
}

fn rust_cli_cargo_toml(spec: &RustCliProjectSpec) -> String {
    format!(
        r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2021"
description = "A polished CLI built with clap."
license = "MIT"

[dependencies]
clap = {{ version = "4.5", features = ["cargo", "derive", "string", "unicode", "wrap_help"] }}
color-eyre = "0.6"
"#,
        package_name = spec.package_name
    )
}

fn rust_cli_readme(spec: &RustCliProjectSpec) -> String {
    format!(
        r#"# {command_name}

## Development

```sh
cargo fmt
cargo test
cargo run -- hello --name belay
```
"#,
        command_name = spec.command_name
    )
}

fn rust_cli_lib_rs() -> &'static str {
    r#"pub fn render_greeting(name: &str) -> String {
    format!("hello, {name}!")
}

#[cfg(test)]
mod tests {
    use super::render_greeting;

    #[test]
    fn renders_greeting() {
        assert_eq!(render_greeting("belay"), "hello, belay!");
    }
}
"#
}

fn rust_cli_main_rs(spec: &RustCliProjectSpec) -> String {
    format!(
        r#"use clap::builder::styling::{{AnsiColor, Effects, Styles}};
use clap::{{Parser, Subcommand}};
use color_eyre::eyre::Result;

use {crate_name}::render_greeting;

fn main() -> Result<()> {{
    color_eyre::install()?;
    let cli = Cli::parse();

    match cli.command {{
        Commands::Hello {{ name }} => {{
            println!("{{}}", render_greeting(&name));
        }}
    }}

    Ok(())
}}

#[derive(Debug, Parser)]
#[command(
    name = "{command_name}",
    version,
    about = "A polished CLI scaffold built with clap.",
    styles = styles()
)]
struct Cli {{
    #[command(subcommand)]
    command: Commands,
}}

#[derive(Debug, Subcommand)]
enum Commands {{
    Hello {{
        #[arg(short, long, default_value = "world")]
        name: String,
    }},
}}

fn styles() -> Styles {{
    Styles::styled()
        .header(AnsiColor::Yellow.on_default() | Effects::BOLD)
        .usage(AnsiColor::Yellow.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Green.on_default())
}}
"#,
        crate_name = spec.crate_name,
        command_name = spec.command_name
    )
}

fn rust_cli_smoke_test(spec: &RustCliProjectSpec) -> String {
    format!(
        r#"use {crate_name}::render_greeting;

#[test]
fn smoke() {{
    assert_eq!(render_greeting("cli"), "hello, cli!");
}}
"#,
        crate_name = spec.crate_name
    )
}

fn go_cli_go_mod(spec: &GoCliProjectSpec) -> String {
    format!(
        r#"module {module_name}

go 1.22

require github.com/spf13/cobra {cobra_version}
"#,
        module_name = spec.module_name,
        cobra_version = DEFAULT_COBRA_VERSION
    )
}

fn go_cli_readme(spec: &GoCliProjectSpec) -> String {
    format!(
        r#"# {command_name}

## Development

```sh
go test ./...
go run . hello --name belay
```
"#,
        command_name = spec.command_name
    )
}

fn go_cli_main_go(spec: &GoCliProjectSpec) -> String {
    format!(
        r#"package main

import (
    "fmt"
    "os"

    "{module_name}/cmd"
)

func main() {{
    if err := cmd.Execute(); err != nil {{
        fmt.Fprintln(os.Stderr, err)
        os.Exit(1)
    }}
}}
"#,
        module_name = spec.module_name
    )
}

fn go_cli_root_go(spec: &GoCliProjectSpec) -> String {
    format!(
        r#"package cmd

import (
    "fmt"

    "github.com/spf13/cobra"
)

type options struct {{
    name string
}}

func Execute() error {{
    return newRootCmd().Execute()
}}

func newRootCmd() *cobra.Command {{
    opts := options{{}}

    cmd := &cobra.Command{{
        Use:           "{command_name}",
        Short:         "A polished CLI scaffold built with Cobra.",
        SilenceUsage:  true,
        SilenceErrors: true,
        CompletionOptions: cobra.CompletionOptions{{
            DisableDefaultCmd: true,
        }},
    }}

    cmd.AddCommand(newHelloCmd(&opts))
    cmd.SetHelpCommand(&cobra.Command{{Hidden: true}})
    return cmd
}}

func newHelloCmd(opts *options) *cobra.Command {{
    cmd := &cobra.Command{{
        Use:   "hello",
        Short: "Print a clean greeting.",
        RunE: func(cmd *cobra.Command, args []string) error {{
            cmd.Println(renderGreeting(opts.name))
            return nil
        }},
    }}

    cmd.Flags().StringVarP(&opts.name, "name", "n", "world", "Who to greet.")
    return cmd
}}

func renderGreeting(name string) string {{
    return fmt.Sprintf("hello, %s!", name)
}}
"#,
        command_name = spec.command_name
    )
}

fn go_cli_root_test_go() -> &'static str {
    r#"package cmd

import "testing"

func TestRenderGreeting(t *testing.T) {
    t.Parallel()

    if got := renderGreeting("belay"); got != "hello, belay!" {
        t.Fatalf("unexpected greeting: %q", got)
    }
}
"#
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
        py|rs-cli|go-cli)
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
    case py rs-cli go-cli
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
enum Background {
    Dark,
    Light,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Rgb {
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tone {
    Pink,
    Purple,
    White,
    Black,
}

impl Tone {
    fn sgr(self) -> &'static str {
        match self {
            Self::Pink => SGR_PINK,
            Self::Purple => SGR_PURPLE,
            Self::White => SGR_WHITE,
            Self::Black => SGR_BLACK,
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
const DARK_BACKGROUND_BANNER_COLORS: [&str; 5] =
    [SGR_PINK, SGR_PLUM, SGR_VIOLET, SGR_INDIGO, SGR_BLUE];
const LIGHT_BACKGROUND_BANNER_COLORS: [&str; 5] = [
    SGR_DARK_PINK,
    SGR_DARK_PLUM,
    SGR_PURPLE,
    SGR_DARK_INDIGO,
    SGR_DARK_BLUE,
];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Theme {
    enabled: bool,
    accent: Tone,
    text: Tone,
    banner_colors: [&'static str; 5],
}

impl Theme {
    fn stdout() -> Self {
        Self::for_terminal(io::stdout().is_terminal())
    }

    fn stderr() -> Self {
        Self::for_terminal(io::stderr().is_terminal())
    }

    fn for_terminal(is_terminal: bool) -> Self {
        if !is_terminal || env::var_os("NO_COLOR").is_some() {
            return Self::plain();
        }

        Self::for_background(reported_terminal_background())
    }

    fn for_background(background: Background) -> Self {
        match background {
            Background::Dark => Self {
                enabled: true,
                accent: Tone::Pink,
                text: Tone::White,
                banner_colors: DARK_BACKGROUND_BANNER_COLORS,
            },
            Background::Light => Self {
                enabled: true,
                accent: Tone::Purple,
                text: Tone::Black,
                banner_colors: LIGHT_BACKGROUND_BANNER_COLORS,
            },
        }
    }

    fn plain() -> Self {
        Self {
            enabled: false,
            accent: Tone::Pink,
            text: Tone::White,
            banner_colors: DARK_BACKGROUND_BANNER_COLORS,
        }
    }

    fn message(self, value: impl std::fmt::Display) -> String {
        if self.enabled {
            format!("{}{value}{SGR_RESET}", self.text.sgr())
        } else {
            value.to_string()
        }
    }

    fn accent(self, value: impl std::fmt::Display) -> String {
        if self.enabled {
            format!("{}{value}{}", self.accent.sgr(), self.text.sgr())
        } else {
            value.to_string()
        }
    }

    fn heading(self, value: &str) -> String {
        if self.enabled {
            format!(
                "{SGR_BOLD}{}{value}{SGR_NOT_BOLD}{}",
                self.accent.sgr(),
                self.text.sgr()
            )
        } else {
            value.to_string()
        }
    }

    fn banner_letter(self, value: &str, color: &str) -> String {
        if self.enabled {
            format!("{color}{value}{}", self.text.sgr())
        } else {
            value.to_string()
        }
    }
}

fn reported_terminal_background() -> Background {
    if let Some(background) = background_override() {
        return background;
    }

    *TERMINAL_BACKGROUND.get_or_init(|| query_terminal_background().unwrap_or(Background::Dark))
}

fn background_override() -> Option<Background> {
    match env::var("BELAY_BACKGROUND")
        .ok()?
        .to_ascii_lowercase()
        .as_str()
    {
        "dark" => Some(Background::Dark),
        "light" => Some(Background::Light),
        _ => None,
    }
}

fn classify_background(rgb: Rgb) -> Background {
    let luminosity =
        u32::from(rgb.red) * 299 + u32::from(rgb.green) * 587 + u32::from(rgb.blue) * 114;
    if luminosity >= 150_000 {
        Background::Light
    } else {
        Background::Dark
    }
}

fn parse_background_response(response: &[u8]) -> Option<Background> {
    let response = std::str::from_utf8(response).ok()?;
    let rgb = response.split_once("rgb:")?.1;
    let rgb = rgb.split(['\u{7}', '\u{1b}']).next()?;
    let mut components = rgb.split('/');
    let color = Rgb {
        red: parse_color_component(components.next()?)?,
        green: parse_color_component(components.next()?)?,
        blue: parse_color_component(components.next()?)?,
    };
    if components.next().is_some() {
        return None;
    }
    Some(classify_background(color))
}

fn parse_color_component(component: &str) -> Option<u8> {
    if component.is_empty() || component.len() > 4 {
        return None;
    }

    let value = u32::from_str_radix(component, 16).ok()?;
    let maximum = (1_u32 << (component.len() * 4)) - 1;
    Some(((value * 255 + maximum / 2) / maximum) as u8)
}

#[cfg(unix)]
fn query_terminal_background() -> Option<Background> {
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let saved_mode = stty_output(&tty, &["-g"])?;
    let _restore = TerminalModeRestore {
        tty: tty.try_clone().ok()?,
        saved_mode,
    };

    if !stty_status(&tty, &["-echo", "-icanon", "min", "0", "time", "1"]) {
        return None;
    }

    tty.write_all(OSC_BACKGROUND_QUERY).ok()?;
    tty.flush().ok()?;

    let mut response = Vec::new();
    let mut buffer = [0; 128];
    for _ in 0..3 {
        let bytes_read = tty.read(&mut buffer).ok()?;
        if bytes_read == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..bytes_read]);
        if response.contains(&b'\x07') || response.windows(2).any(|end| end == b"\x1b\\") {
            break;
        }
    }

    parse_background_response(&response)
}

#[cfg(not(unix))]
fn query_terminal_background() -> Option<Background> {
    None
}

#[cfg(unix)]
fn stty_output(tty: &File, args: &[&str]) -> Option<String> {
    let output = Command::new("stty")
        .args(args)
        .stdin(Stdio::from(tty.try_clone().ok()?))
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

#[cfg(unix)]
fn stty_status(tty: &File, args: &[&str]) -> bool {
    Command::new("stty")
        .args(args)
        .stdin(Stdio::from(match tty.try_clone() {
            Ok(tty) => tty,
            Err(_) => return false,
        }))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
struct TerminalModeRestore {
    tty: File,
    saved_mode: String,
}

#[cfg(unix)]
impl Drop for TerminalModeRestore {
    fn drop(&mut self) {
        let _ = stty_status(&self.tty, &[&self.saved_mode]);
    }
}

fn render_banner(theme: Theme) -> String {
    let mut banner = String::new();
    for row in 0..BANNER_LETTERS[0].len() {
        for (letter, color) in BANNER_LETTERS.iter().zip(theme.banner_colors) {
            banner.push_str(&theme.banner_letter(letter[row], color));
        }
        banner.push('\n');
    }
    if theme.enabled {
        banner.push_str(SGR_RESET);
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
        "{}",
        theme.message(format!(
            r#"{brand} {version}

{usage}:
  {brand} {py} <name> [{python} <version>] [{no_shell}]
  {brand} {rs_cli} <name> [{no_shell}]
  {brand} {go_cli} <name> [{no_shell}]
  {brand} {shell} {init} [fish|bash|zsh]
  {brand} {shell} {install} [fish|bash|zsh]

{commands}:
  {py}       Create a typed uv Python project and print/cd to it through shell integration
  {rs_cli}   Create a polished Rust CLI scaffold built with clap
  {go_cli}   Create a polished Go CLI scaffold built with Cobra
  {shell}    Print or install shell integration for automatic cd
"#,
            brand = theme.accent("belay"),
            version = VERSION,
            usage = theme.heading("Usage"),
            commands = theme.heading("Commands"),
            py = theme.accent("py"),
            rs_cli = theme.accent("rs-cli"),
            go_cli = theme.accent("go-cli"),
            shell = theme.accent("shell"),
            init = theme.accent("init"),
            install = theme.accent("install"),
            python = theme.accent("--python"),
            no_shell = theme.accent("--no-shell"),
        ))
    );
}

fn print_py_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!(
        "{}",
        theme.message(format!(
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
            brand = theme.accent("belay"),
            py = theme.accent("py"),
            python = theme.accent("--python"),
            no_shell = theme.accent("--no-shell"),
            creates = theme.heading("Creates"),
            options = theme.heading("Options"),
            default_python = DEFAULT_PYTHON,
        ))
    );
}

fn print_rs_cli_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!(
        "{}",
        theme.message(format!(
            r#"{usage}:
  {brand} {rs_cli} <name> [{no_shell}]

{creates} a new directory in the current working directory with:
  Cargo.toml, src/main.rs, src/lib.rs, tests/smoke.rs, README.md
  clap with styled help output, color-eyre, and Git on main

{options}:
  {no_shell}  Skip automatic shell integration provisioning
"#,
            usage = theme.heading("Usage"),
            brand = theme.accent("belay"),
            rs_cli = theme.accent("rs-cli"),
            creates = theme.heading("Creates"),
            options = theme.heading("Options"),
            no_shell = theme.accent("--no-shell"),
        ))
    );
}

fn print_go_cli_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!(
        "{}",
        theme.message(format!(
            r#"{usage}:
  {brand} {go_cli} <name> [{no_shell}]

{creates} a new directory in the current working directory with:
  go.mod, main.go, cmd/root.go, cmd/root_test.go, README.md
  Cobra with a clean command layout and Git on main

{options}:
  {no_shell}  Skip automatic shell integration provisioning
"#,
            usage = theme.heading("Usage"),
            brand = theme.accent("belay"),
            go_cli = theme.accent("go-cli"),
            creates = theme.heading("Creates"),
            options = theme.heading("Options"),
            no_shell = theme.accent("--no-shell"),
        ))
    );
}

fn print_shell_help() {
    let theme = Theme::stdout();
    print_banner(theme);
    println!("{}", theme.message(format!(
        r#"{usage}:
  {brand} {shell} {init} [fish|bash|zsh]
  {brand} {shell} {install} [fish|bash|zsh]

{init_quoted} prints the function for manual shell setup.
{install_quoted} writes a managed integration block/file so `{brand} {py} <name>`, `{brand} {rs_cli} <name>`, and `{brand} {go_cli} <name>` can cd into the new directory.
"#,
        usage = theme.heading("Usage"),
        brand = theme.accent("belay"),
        shell = theme.accent("shell"),
        init = theme.accent("init"),
        install = theme.accent("install"),
        init_quoted = theme.accent("`init`"),
        install_quoted = theme.accent("`install`"),
        py = theme.accent("py"),
        rs_cli = theme.accent("rs-cli"),
        go_cli = theme.accent("go-cli"),
    )));
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
    fn dark_background_uses_pink_accent_and_white_text() {
        let theme = Theme::for_background(Background::Dark);
        let banner = render_banner(theme);
        let message = theme.message(format!("{} version", theme.accent("belay")));

        assert_eq!(theme.accent("belay"), format!("{SGR_PINK}belay{SGR_WHITE}"));
        assert_eq!(
            message,
            format!("{SGR_WHITE}{SGR_PINK}belay{SGR_WHITE} version{SGR_RESET}")
        );
        assert_eq!(
            theme.heading("Usage"),
            format!("{SGR_BOLD}{SGR_PINK}Usage{SGR_NOT_BOLD}{SGR_WHITE}")
        );
        assert!(banner.contains(SGR_PINK));
        assert!(banner.contains(SGR_PLUM));
        assert!(banner.contains(SGR_VIOLET));
        assert!(banner.contains(SGR_INDIGO));
        assert!(banner.contains(SGR_BLUE));
    }

    #[test]
    fn light_background_uses_purple_accent_and_black_text() {
        let theme = Theme::for_background(Background::Light);
        let banner = render_banner(theme);

        assert_eq!(
            theme.accent("belay"),
            format!("{SGR_PURPLE}belay{SGR_BLACK}")
        );
        assert_eq!(theme.message("body"), format!("{SGR_BLACK}body{SGR_RESET}"));
        assert!(banner.contains(SGR_DARK_PINK));
        assert!(banner.contains(SGR_DARK_PLUM));
        assert!(banner.contains(SGR_PURPLE));
        assert!(banner.contains(SGR_DARK_INDIGO));
        assert!(banner.contains(SGR_DARK_BLUE));
    }

    #[test]
    fn parses_terminal_reported_background_brightness() {
        assert_eq!(
            parse_background_response(b"\x1b]11;rgb:1919/1b1b/2424\x1b\\"),
            Some(Background::Dark)
        );
        assert_eq!(
            parse_background_response(b"\x1b]11;rgb:ffff/fafa/f5f5\x07"),
            Some(Background::Light)
        );
        assert_eq!(
            parse_background_response(b"\x1b]11;rgb:f5/f5/f5\x1b\\"),
            Some(Background::Light)
        );
    }

    #[test]
    fn disabled_theme_banner_and_shell_functions_stay_plain() {
        let theme = Theme::plain();
        let banner = render_banner(theme);

        assert_eq!(theme.message("/tmp/project"), "/tmp/project");
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
    fn normalizes_cli_project_names() {
        let rust_spec = RustCliProjectSpec::new("My_App.Tools").unwrap();
        let go_spec = GoCliProjectSpec::new("My_App.Tools").unwrap();

        assert_eq!(rust_spec.package_name, "my-app-tools");
        assert_eq!(rust_spec.crate_name, "my_app_tools");
        assert_eq!(go_spec.module_name, "my-app-tools");
    }

    #[test]
    fn rejects_names_that_cannot_be_modules() {
        assert!(PythonProjectSpec::new("123-app", "3.13").is_err());
        assert!(PythonProjectSpec::new("bad/path", "3.13").is_err());
        assert!(PythonProjectSpec::new("bad name", "3.13").is_err());
        assert!(RustCliProjectSpec::new("123-app").is_err());
        assert!(GoCliProjectSpec::new("123-app").is_err());
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
    fn creates_rust_cli_project_layout() {
        let root = unique_temp_dir();
        fs::create_dir(&root).unwrap();
        let project_dir = root.join("sample-cli");
        let spec = RustCliProjectSpec::new("sample-cli").unwrap();
        let git_runner = FakeGitRunner::default();

        create_rust_cli_project(&project_dir, &spec).unwrap();
        finish_project_creation_with_git_runner(&project_dir, false, &git_runner).unwrap();

        assert!(project_dir.join("Cargo.toml").exists());
        assert!(project_dir.join("README.md").exists());
        assert!(project_dir.join("src").join("main.rs").exists());
        assert!(project_dir.join("src").join("lib.rs").exists());
        assert!(project_dir.join("tests").join("smoke.rs").exists());
        assert!(project_dir.join(".git").exists());

        let cargo_toml = fs::read_to_string(project_dir.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("clap = { version = \"4.5\""));
        assert!(cargo_toml.contains("color-eyre = \"0.6\""));

        let main_rs = fs::read_to_string(project_dir.join("src").join("main.rs")).unwrap();
        assert!(main_rs.contains("A polished CLI scaffold built with clap."));
        assert!(main_rs.contains("render_greeting"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creates_go_cli_project_layout() {
        let root = unique_temp_dir();
        fs::create_dir(&root).unwrap();
        let project_dir = root.join("sample-go");
        let spec = GoCliProjectSpec::new("sample-go").unwrap();
        let git_runner = FakeGitRunner::default();

        create_go_cli_project(&project_dir, &spec).unwrap();
        finish_project_creation_with_git_runner(&project_dir, false, &git_runner).unwrap();

        assert!(project_dir.join("go.mod").exists());
        assert!(project_dir.join("README.md").exists());
        assert!(project_dir.join("main.go").exists());
        assert!(project_dir.join("cmd").join("root.go").exists());
        assert!(project_dir.join("cmd").join("root_test.go").exists());
        assert!(project_dir.join(".git").exists());

        let go_mod = fs::read_to_string(project_dir.join("go.mod")).unwrap();
        assert!(go_mod.contains("github.com/spf13/cobra"));
        assert!(go_mod.contains(DEFAULT_COBRA_VERSION));

        let root_go = fs::read_to_string(project_dir.join("cmd").join("root.go")).unwrap();
        assert!(root_go.contains("A polished CLI scaffold built with Cobra."));
        assert!(root_go.contains("renderGreeting"));

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
        assert!(new.contains("py|rs-cli|go-cli)"));
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
