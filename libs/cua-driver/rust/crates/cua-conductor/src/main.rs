use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::Duration,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "cua-conductor",
    about = "Visible multi-automation orchestration for cua-driver"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(long)]
        force: bool,
    },
    Automation {
        #[command(subcommand)]
        command: AutomationCommand,
    },
    List,
    Run {
        id: String,
    },
    Status,
    Stop {
        session: String,
    },
    OverlayTest {
        #[arg(long, default_value_t = 6)]
        seconds: u64,
        #[arg(long, default_value = "VISIBLE AUTOMATION TEST")]
        label: String,
    },
}

#[derive(Subcommand)]
enum AutomationCommand {
    Add {
        id: String,
        #[arg(long)]
        command: String,
        #[arg(long = "arg")]
        args: Vec<String>,
        #[arg(long, default_value = "cli")]
        kind: String,
        #[arg(long, default_value = "Desktop")]
        target: String,
        #[arg(long, default_value = "")]
        group: String,
    },
    Remove {
        id: String,
    },
    Show {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    overlay: OverlayConfig,
    automations: Vec<Automation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OverlayConfig {
    enabled: bool,
    flash_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Automation {
    id: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    target_hint: String,
    #[serde(default)]
    exclusive_group: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Session {
    session_id: String,
    automation_id: String,
    pid: u32,
    state: String,
    target_window: String,
    exclusive_group: String,
}
fn data_root() -> Result<PathBuf> {
    let root = if cfg!(windows) {
        PathBuf::from(env::var("LOCALAPPDATA").context("LOCALAPPDATA is not set")?)
            .join("CuaConductor")
    } else if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("cua-conductor")
    } else {
        PathBuf::from(env::var("HOME").context("HOME is not set")?)
            .join(".local/share/cua-conductor")
    };
    fs::create_dir_all(root.join("sessions"))?;
    Ok(root)
}

fn config_path() -> Result<PathBuf> {
    Ok(data_root()?.join("config.json"))
}

fn read_config() -> Result<Config> {
    let path = config_path()?;
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "missing config: {} (run `cua-conductor init`)",
            path.display()
        )
    })?;
    parse_config_text(&text)
}
fn parse_config_text(text: &str) -> Result<Config> {
    serde_json::from_str(text.trim_start_matches('\u{feff}')).context("parse conductor config")
}

fn write_config(cfg: &Config) -> Result<()> {
    write_json(&config_path()?, cfg)
}

fn add_automation(mut cfg: Config, automation: Automation) -> Result<()> {
    if cfg.automations.iter().any(|a| a.id == automation.id) {
        bail!("automation `{}` already exists", automation.id);
    }
    cfg.automations.push(automation);
    write_config(&cfg)
}

fn remove_automation(mut cfg: Config, id: &str) -> Result<()> {
    let before = cfg.automations.len();
    cfg.automations.retain(|a| a.id != id);
    if cfg.automations.len() == before {
        bail!("unknown automation `{id}`");
    }
    write_config(&cfg)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}
fn sample_config() -> Config {
    Config {
        overlay: OverlayConfig {
            enabled: true,
            flash_enabled: true,
        },
        automations: vec![Automation {
            id: "visible-test".into(),
            command: if cfg!(windows) {
                "powershell".into()
            } else {
                "sh".into()
            },
            args: if cfg!(windows) {
                vec![
                    "-NoProfile".into(),
                    "-Command".into(),
                    "Start-Sleep -Seconds 6".into(),
                ]
            } else {
                vec!["-c".into(), "sleep 6".into()]
            },
            kind: "test".into(),
            target_hint: "Desktop".into(),
            exclusive_group: "foreground-gui".into(),
        }],
    }
}

fn session_files() -> Result<Vec<PathBuf>> {
    let dir = data_root()?.join("sessions");
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|x| x.to_str()) == Some("json") {
            out.push(path);
        }
    }
    Ok(out)
}
fn load_session(path: &Path) -> Result<Session> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    Command::new("tasklist")
        .args(["/FI", &filter, "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn pid_alive(pid: u32) -> bool {
    Command::new("sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn active_in_group(group: &str) -> Result<Option<Session>> {
    if group.is_empty() {
        return Ok(None);
    }
    for path in session_files()? {
        let s = load_session(&path)?;
        if s.exclusive_group == group && s.state == "active" && pid_alive(s.pid) {
            return Ok(Some(s));
        }
    }
    Ok(None)
}
fn overlay_exe() -> Result<PathBuf> {
    let exe = env::current_exe()?;
    let name = if cfg!(windows) {
        "cua-warning-overlay.exe"
    } else {
        "cua-warning-overlay"
    };
    let path = exe
        .parent()
        .context("current executable has no parent")?
        .join(name);
    if !path.exists() {
        bail!(
            "warning overlay executable not found beside conductor: {}",
            path.display()
        );
    }
    Ok(path)
}

fn spawn_overlay(
    label: &str,
    session: &str,
    target: &str,
    flash: bool,
    duration: Option<u64>,
) -> Result<Child> {
    let mut cmd = Command::new(overlay_exe()?);
    cmd.args([
        "--label",
        label,
        "--session",
        session,
        "--target",
        target,
        "--mode",
        "active",
    ]);
    if flash {
        cmd.arg("--flash");
    }
    if let Some(seconds) = duration {
        cmd.args(["--duration", &seconds.to_string()]);
    }
    cmd.spawn().context("start warning overlay")
}

#[cfg(windows)]
fn terminate_pid(pid: u32) -> Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()?;
    if !status.success() {
        bail!("taskkill failed for PID {pid}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn terminate_pid(pid: u32) -> Result<()> {
    let status = Command::new("kill").arg(pid.to_string()).status()?;
    if !status.success() {
        bail!("kill failed for PID {pid}");
    }
    Ok(())
}
fn run_automation(cfg: &Config, id: &str) -> Result<()> {
    let automation = cfg
        .automations
        .iter()
        .find(|a| a.id == id)
        .with_context(|| format!("unknown automation `{id}`"))?;
    if let Some(other) = active_in_group(&automation.exclusive_group)? {
        bail!(
            "exclusive group `{}` is occupied by {} ({})",
            automation.exclusive_group,
            other.automation_id,
            other.session_id
        );
    }

    let session_id = format!("sess_{}", Uuid::new_v4().simple());
    let mut overlay = if cfg.overlay.enabled {
        Some(spawn_overlay(
            &automation.id,
            &session_id,
            &automation.target_hint,
            cfg.overlay.flash_enabled,
            None,
        )?)
    } else {
        None
    };
    thread::sleep(Duration::from_millis(350));

    let mut child = Command::new(&automation.command)
        .args(&automation.args)
        .spawn()
        .with_context(|| format!("launch automation `{}`", automation.id))?;
    let path = data_root()?
        .join("sessions")
        .join(format!("{session_id}.json"));
    let mut session = Session {
        session_id: session_id.clone(),
        automation_id: automation.id.clone(),
        pid: child.id(),
        state: "active".into(),
        target_window: automation.target_hint.clone(),
        exclusive_group: automation.exclusive_group.clone(),
    };
    write_json(&path, &session)?;
    println!(
        "started {} as {} (pid {})",
        automation.id,
        session_id,
        child.id()
    );

    let status = child.wait()?;
    if let Some(ref mut o) = overlay {
        let _ = o.kill();
        let _ = o.wait();
    }
    session.state = format!("exited:{}", status.code().unwrap_or(-1));
    write_json(&path, &session)?;
    if !status.success() {
        bail!("automation exited with {status}");
    }
    Ok(())
}
fn print_status() -> Result<()> {
    for path in session_files()? {
        let mut s = load_session(&path)?;
        if s.state == "active" && !pid_alive(s.pid) {
            s.state = "stale".into();
            write_json(&path, &s)?;
        }
        println!(
            "{}  {:<18} pid={:<7} state={:<12} target={}",
            s.session_id, s.automation_id, s.pid, s.state, s.target_window
        );
    }
    Ok(())
}

fn stop_session(id: &str) -> Result<()> {
    let path = data_root()?.join("sessions").join(format!("{id}.json"));
    if !path.exists() {
        bail!("unknown session `{id}`");
    }
    let mut session = load_session(&path)?;
    if pid_alive(session.pid) {
        terminate_pid(session.pid)?;
    }
    session.state = "stopped".into();
    write_json(&path, &session)?;
    println!("stopped {} (pid {})", id, session.pid);
    Ok(())
}

fn print_automation(a: &Automation) {
    println!(
        "{:<20} kind={:<10} group={:<16} target={} :: {} {}",
        a.id,
        a.kind,
        a.exclusive_group,
        a.target_hint,
        a.command,
        a.args.join(" ")
    );
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { force } => {
            let path = config_path()?;
            if path.exists() && !force {
                bail!("config already exists: {} (use --force)", path.display());
            }
            write_json(&path, &sample_config())?;
            println!("wrote {}", path.display());
        }
        Commands::Automation { command } => {
            let cfg = read_config()?;
            match command {
                AutomationCommand::Add {
                    id,
                    command,
                    args,
                    kind,
                    target,
                    group,
                } => {
                    add_automation(
                        cfg,
                        Automation {
                            id,
                            command,
                            args,
                            kind,
                            target_hint: target,
                            exclusive_group: group,
                        },
                    )?;
                }
                AutomationCommand::Remove { id } => remove_automation(cfg, &id)?,
                AutomationCommand::Show { id } => {
                    let a = cfg
                        .automations
                        .iter()
                        .find(|a| a.id == id)
                        .with_context(|| format!("unknown automation `{id}`"))?;
                    print_automation(a);
                }
            }
        }
        Commands::List => {
            let cfg = read_config()?;
            for a in &cfg.automations {
                print_automation(a);
            }
        }
        Commands::Run { id } => {
            let cfg = read_config()?;
            run_automation(&cfg, &id)?;
        }
        Commands::Status => print_status()?,
        Commands::Stop { session } => stop_session(&session)?,
        Commands::OverlayTest { seconds, label } => {
            let cfg = read_config().unwrap_or_else(|_| sample_config());
            let session = format!("test_{}", Uuid::new_v4().simple());
            let mut child = spawn_overlay(
                &label,
                &session,
                "Desktop",
                cfg.overlay.flash_enabled,
                Some(seconds),
            )?;
            println!("overlay test {} for {} seconds", session, seconds);
            let status = child.wait()?;
            if !status.success() {
                bail!("overlay test failed: {status}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_config_is_visible_and_grouped() {
        let cfg = sample_config();
        assert!(cfg.overlay.enabled);
        assert!(cfg.overlay.flash_enabled);
        assert_eq!(cfg.automations.len(), 1);
        assert_eq!(cfg.automations[0].id, "visible-test");
        assert_eq!(cfg.automations[0].exclusive_group, "foreground-gui");
    }

    #[test]
    fn config_parser_accepts_utf8_bom() {
        let cfg = sample_config();
        let encoded = format!("\u{feff}{}", serde_json::to_string(&cfg).unwrap());
        let decoded = parse_config_text(&encoded).unwrap();
        assert_eq!(decoded.automations[0].id, "visible-test");
    }

    #[test]
    fn sample_config_round_trips() {
        let cfg = sample_config();
        let encoded = serde_json::to_string(&cfg).unwrap();
        let decoded: Config = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.automations[0].target_hint, "Desktop");
        assert!(decoded.overlay.flash_enabled);
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn parses_automation_add() {
        let cli = Cli::try_parse_from([
            "cua-conductor",
            "automation",
            "add",
            "worker-a",
            "--command",
            "python",
            "--arg",
            "worker.py",
            "--target",
            "Chrome",
            "--group",
            "foreground-gui",
        ])
        .unwrap();
        match cli.command {
            Commands::Automation {
                command:
                    AutomationCommand::Add {
                        id,
                        command,
                        args,
                        target,
                        group,
                        ..
                    },
            } => {
                assert_eq!(id, "worker-a");
                assert_eq!(command, "python");
                assert_eq!(args, vec!["worker.py"]);
                assert_eq!(target, "Chrome");
                assert_eq!(group, "foreground-gui");
            }
            _ => panic!("unexpected command"),
        }
    }
}
