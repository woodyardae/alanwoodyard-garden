//! `bosun-garden` publishes one tenant vault to a static site.
//!
//! Progress and failures are JSON lines on stdout so Bosun Studio can stream
//! them. A successful build ends with `build_finished`. A failure ends with
//! `build_error` and a non-zero exit code.
//!
//! ```text
//! {"event":"build_started","vault":"...","output":"...","tenant":"acme","domain":"blog","origin":"https://garden.local"}
//! {"event":"tenant_started","tenant":"acme"}
//! {"event":"tenant_compiled","tenant":"acme","pages":1}
//! {"event":"page_emitted","tenant":"acme","source":"hello.md","output":"..."}
//! {"event":"tenant_finished","tenant":"acme","output_dir":"...","pages":1}
//! {"event":"build_finished","ok":true,"tenant":"acme","output_dir":"...","pages":1}
//! ```

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

use bosun_garden_compiler::{emit_tenant_sites, BuildEvent, CompileError};
use clap::{ColorChoice, Parser, Subcommand};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(
    name = "bosun-garden",
    version,
    about = "Publish a Bosun Garden tenant vault to static HTML",
    color = ColorChoice::Never,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile one tenant into an isolated static site.
    Build {
        /// Vault root containing Markdown notes.
        #[arg(long, value_name = "PATH")]
        vault: PathBuf,
        /// Directory that receives `{tenant}/` site output.
        #[arg(long, value_name = "DIR")]
        output: PathBuf,
        /// Frontmatter tenant slug to publish.
        #[arg(long, value_name = "SLUG")]
        tenant: String,
        /// Frontmatter key that holds the tenant slug.
        #[arg(long, value_name = "KEY", default_value = "blog")]
        domain: String,
        /// Public https origin used in permalinks.
        #[arg(long, value_name = "URL", default_value = "https://garden.local")]
        origin: String,
    },
}

#[derive(Debug, Error)]
enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("failed to encode telemetry: {0}")]
    Telemetry(String),
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Compile(_) | Self::Io(_) | Self::Telemetry(_) => 1,
        }
    }
}

fn main() {
    let code = match Cli::try_parse() {
        Ok(cli) => match execute(cli) {
            Ok(code) => code,
            Err(err) => report_failure(&err),
        },
        Err(err) => clap_exit(err),
    };
    process::exit(code);
}

fn execute(cli: Cli) -> Result<i32, CliError> {
    let Command::Build {
        vault,
        output,
        tenant,
        domain,
        origin,
    } = cli.command;
    write_json(&json!({
        "event": "build_started",
        "vault": vault.display().to_string(),
        "output": output.display().to_string(),
        "tenant": tenant,
        "domain": domain,
        "origin": origin,
    }))?;

    let mut telemetry_error = None;
    let compiled = emit_tenant_sites(
        &vault,
        &domain,
        &origin,
        &output,
        Some(tenant.as_str()),
        |event| {
            if telemetry_error.is_none() {
                if let Err(err) = write_build_event(&event) {
                    telemetry_error = Some(err);
                }
            }
        },
    );
    let sites = match compiled {
        Ok(sites) => sites,
        Err(err) => return Err(CliError::Compile(err)),
    };
    if let Some(err) = telemetry_error {
        return Err(err);
    }
    let site = sites
        .into_iter()
        .find(|site| site.tenant == tenant)
        .ok_or_else(|| {
            CliError::Compile(CompileError::UnknownTenant {
                tenant: tenant.clone(),
            })
        })?;
    write_json(&json!({
        "event": "build_finished",
        "ok": true,
        "tenant": site.tenant,
        "output_dir": site.output_dir.display().to_string(),
        "pages": site.pages.len(),
    }))?;
    Ok(0)
}

fn write_build_event(event: &BuildEvent) -> Result<(), CliError> {
    let value = match event {
        BuildEvent::Started { tenant } => json!({
            "event": "tenant_started",
            "tenant": tenant,
        }),
        BuildEvent::Compiled { tenant, pages } => json!({
            "event": "tenant_compiled",
            "tenant": tenant,
            "pages": pages,
        }),
        BuildEvent::Page {
            tenant,
            source,
            output,
        } => json!({
            "event": "page_emitted",
            "tenant": tenant,
            "source": source,
            "output": output.display().to_string(),
        }),
        BuildEvent::Finished {
            tenant,
            output_dir,
            pages,
        } => json!({
            "event": "tenant_finished",
            "tenant": tenant,
            "output_dir": output_dir.display().to_string(),
            "pages": pages,
        }),
    };
    write_json(&value)
}

fn report_failure(err: &CliError) -> i32 {
    if let Err(write_err) = write_json(&failure_event(err)) {
        let _ = writeln!(io::stderr(), "bosun-garden: {err} ({write_err})");
    }
    err.exit_code()
}

fn clap_exit(err: clap::Error) -> i32 {
    if err.kind() == clap::error::ErrorKind::DisplayHelp
        || err.kind() == clap::error::ErrorKind::DisplayVersion
    {
        let _ = err.print();
        return 0;
    }
    let failure = CliError::Usage(err.to_string());
    report_failure(&failure)
}

fn failure_event(err: &CliError) -> Value {
    match err {
        CliError::Compile(err) => json!({
            "event": "build_error",
            "ok": false,
            "code": err.code(),
            "message": err.to_string(),
            "path": err.diagnostic_path(),
        }),
        CliError::Usage(message) => json!({
            "event": "build_error",
            "ok": false,
            "code": "usage",
            "message": message,
            "path": Value::Null,
        }),
        CliError::Io(err) => json!({
            "event": "build_error",
            "ok": false,
            "code": "io",
            "message": err.to_string(),
            "path": Value::Null,
        }),
        CliError::Telemetry(message) => json!({
            "event": "build_error",
            "ok": false,
            "code": "telemetry",
            "message": message,
            "path": Value::Null,
        }),
    }
}

fn write_json(value: &Value) -> Result<(), CliError> {
    let mut stdout = io::stdout().lock();
    write_json_line(&mut stdout, value)
}

/// One JSON object, one newline, then flush — GUI clients read events as they land.
fn write_json_line<W: Write>(out: &mut W, value: &Value) -> Result<(), CliError> {
    let line = serde_json::to_string(value).map_err(|err| CliError::Telemetry(err.to_string()))?;
    writeln!(out, "{line}")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn json_line_events_are_flushed_as_complete_lines() {
        let mut buf = Cursor::new(Vec::new());
        write_json_line(
            &mut buf,
            &json!({
                "event": "build_started",
                "tenant": "acme",
            }),
        )
        .unwrap();
        write_json_line(
            &mut buf,
            &json!({
                "event": "tenant_started",
                "tenant": "acme",
            }),
        )
        .unwrap();
        write_json_line(
            &mut buf,
            &json!({
                "event": "page_emitted",
                "tenant": "acme",
                "source": "hello.md",
            }),
        )
        .unwrap();
        write_json_line(
            &mut buf,
            &json!({
                "event": "tenant_finished",
                "tenant": "acme",
                "pages": 1,
            }),
        )
        .unwrap();
        write_json_line(
            &mut buf,
            &json!({
                "event": "build_finished",
                "ok": true,
                "tenant": "acme",
                "pages": 1,
            }),
        )
        .unwrap();

        let text = String::from_utf8(buf.into_inner()).unwrap();
        assert!(text.ends_with('\n'));
        let events: Vec<Value> = text
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let names: Vec<&str> = events
            .iter()
            .map(|event| event["event"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "build_started",
                "tenant_started",
                "page_emitted",
                "tenant_finished",
                "build_finished",
            ]
        );
        // Cursor::flush is a no-op that still succeeds; each write_json_line call
        // reaches flush after the newline so a streaming reader can take complete events.
        assert_eq!(events.len(), 5);
    }
}
