// SPDX-FileCopyrightText: 2025 Mozilla
// SPDX-FileContributor: Nicolas Qiu Guichard <nicolas.guichard@kdab.com>
//
// SPDX-License-Identifier: MPL-2.0

use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

use chrono::{DateTime, FixedOffset};
use clap::Parser;

use tools::file_format::code_coverage_report::{EXACT, Report, ReportMetadata, last_quantized_ref};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Path to the JSON code coverage report
    #[arg(short, long)]
    report: PathBuf,

    /// Path to the output repo, created if missing
    #[arg(short, long)]
    output_repo: PathBuf,

    /// Commit OID that the report came from
    #[arg(short, long)]
    commit: String,

    /// Date of the commit the report came from, in RFC3339 format
    #[arg(short, long, value_parser=chrono::DateTime::parse_from_rfc3339)]
    date: DateTime<FixedOffset>,

    /// Name of the platform covered by this report
    #[arg(short, long, default_value = "all")]
    platform: String,

    /// Name of the testsuite covered by this report
    #[arg(short, long, default_value = "all")]
    testsuite: String,

    /// Whether to save log10(hit count + 1) (default) or the exact hit count
    #[arg(short, long)]
    exact: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let branch = format!("{}/{}", &args.platform, &args.testsuite);

    let metadata = ReportMetadata {
        commit: args.commit,
        branch: branch.clone(),
        date: args.date,
        exact: args.exact,
    };

    Command::new("git")
        .args(["init", "--bare", "--quiet", "--initial-branch=all/all"])
        .arg(&args.output_repo)
        .spawn()?
        .wait()?;

    let reference = format!("refs/heads/{branch}");
    let last_commit_message = Command::new("git")
        .current_dir(&args.output_repo)
        .args(["log", "--format=%B", "-n", "1", &reference, "--"])
        .output()?;
    let existing_branch = last_commit_message.status.success();

    let last_quantized_ref = last_quantized_ref(&metadata.branch);
    let has_last_quantized = Command::new("git")
        .current_dir(&args.output_repo)
        .args(["show-ref", "--quiet", &last_quantized_ref])
        .output()?
        .status
        .success();

    let mut fast_import = Command::new("git")
        .current_dir(&args.output_repo)
        .arg("fast-import")
        .stdin(Stdio::piped())
        .spawn()?;

    let report = File::open(args.report)?;
    let report = BufReader::new(report);
    let report = Report::read(report, metadata)?;

    {
        let fast_import = fast_import
            .stdin
            .as_mut()
            .ok_or("failed to open child process stdin")?;
        let mut fast_import = BufWriter::new(fast_import);
        writeln!(fast_import, "feature date-format=rfc2822")?;
        writeln!(&mut fast_import, "feature done")?;
        writeln!(&mut fast_import, "feature force")?;

        if existing_branch {
            if has_last_quantized && !report.metadata.exact {
                writeln!(fast_import, "reset {reference}")?;
                writeln!(fast_import, "from {last_quantized_ref}")?;
            } else {
                let last_is_not_exact = existing_branch
                    && !last_commit_message
                        .stdout
                        .windows(EXACT.as_bytes().len())
                        .any(|window| window == EXACT.as_bytes());

                if last_is_not_exact {
                    writeln!(fast_import, "reset {last_quantized_ref}")?;
                    writeln!(fast_import, "from {reference}^0")?;
                }

                // Git fast-import will not add new commits to an existing branch unless we initialize it first.
                // See https://git-scm.com/docs/git-fast-import#_from
                writeln!(fast_import, "reset {reference}")?;
                writeln!(fast_import, "from {reference}^0")?;
            }
        }

        report.write_to_git(&mut fast_import)?;
        writeln!(&mut fast_import, "done")?;
    }

    fast_import.wait()?;

    Ok(())
}
