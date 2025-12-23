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

use clap::Parser;
use time::{format_description::well_known::Iso8601, OffsetDateTime};

use tools::file_format::code_coverage_report::{Report, ReportMetadata};

fn parse_date(string: &str) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(string, &Iso8601::DEFAULT)
}

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

    /// Date of the commit the report came from, in ISO 8601-1:2019 format
    #[arg(short, long, value_parser=parse_date)]
    date: OffsetDateTime,

    /// Name of the platform covered by this report
    #[arg(short, long, default_value = "all")]
    platform: String,

    /// Name of the testsuite covered by this report
    #[arg(short, long, default_value = "all")]
    testsuite: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let branch = format!("{}/{}", &args.platform, &args.testsuite);

    let metadata = ReportMetadata {
        commit: args.commit,
        branch: branch.clone(),
        date: args.date,
    };

    Command::new("git")
        .args(["init", "--bare", "--quiet", "--initial-branch=all/all"])
        .arg(&args.output_repo)
        .spawn()?
        .wait()?;

    let existing_branch = Command::new("git")
        .current_dir(&args.output_repo)
        .args(["show-ref", "--quiet", &format!("refs/heads/{branch}")])
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
        writeln!(&mut fast_import, "feature done")?;
        writeln!(&mut fast_import, "feature force")?;
        report.write_to_git(&mut fast_import, existing_branch)?;
        writeln!(&mut fast_import, "done")?;
    }

    fast_import.wait()?;

    Ok(())
}
