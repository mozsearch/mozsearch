// SPDX-FileCopyrightText: 2026 Mozilla
// SPDX-FileContributor: Laurent Montel <laurent.montel@kdab.com>
// SPDX-FileContributor: Nicolas Qiu Guichard <nicolas.guichard@kdab.com>
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::bail;
use clap::Parser;
use git2::{Repository, RepositoryInitOptions};
use google_cloud_gax::paginator::Paginator as _;
use google_cloud_storage::client::{Storage, StorageControl};
use std::{
    collections::HashSet,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Iso8601};
use tokio::{
    sync::mpsc,
    task::{self, JoinHandle},
};
use tools::file_format::code_coverage_report::{Report, ReportMetadata};
use tracing::{debug, info, warn};
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan},
};

/// The number of tasks in-between each pipeline steps.
/// Because the git fast-import task needs to be serialized anyway, there's little interest in going above 1.
const PIPELINE_SIZE: usize = 1;

#[derive(Clone, Debug)]
struct RevisionData {
    datetime: OffsetDateTime,
    git_revision: String,
    hg_revision: String,
}

fn parse_date(string: &str) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(string, &Iso8601::DEFAULT)
}

#[derive(Parser)]
#[command(verbatim_doc_comment)]
/// Converts the coverage reports stored in the GCP bucket into a coverage Git repository.
///
/// Revisions are listed from the gcloud bucket using the firefox_repo to translate between git and mercurial revisions. (The folder names on gcloud use hg hashes.)
///
/// Then for each revision goes though a 5-step pipeline:
/// 1. We list the platforms/testsuites we have coverage for and send each of them down the pipeline.
/// 2. We download the platform/testsuite coverage report.
/// 3. We unzstd-it.
/// 4. We parse it.
/// 5. We send it to git-fast-import.
struct Args {
    /// Path of firefox repositories. Used to convert hg revisions to git revisions.
    #[arg(short, long)]
    firefox_repo: PathBuf,

    /// Path to the output repo, created if missing.
    #[arg(short, long)]
    output_repo: PathBuf,

    /// Only import for branch (mainly for debugging).
    #[arg(long)]
    branch: Option<String>,

    /// Only import revisions since (in ISO 8601-1:2019 format) (mainly for debugging).
    #[arg(long, value_parser=parse_date)]
    since: Option<OffsetDateTime>,
}

const BUCKET: &str = "projects/_/buckets/relman-code-coverage-prod";
const PREFIX: &str = "mozilla-central/";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt::fmt()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    if !args.firefox_repo.exists() {
        bail!(
            "Firefox repo path does not exist:  {}",
            args.firefox_repo.display()
        );
    }

    if !args.output_repo.exists() {
        fs::create_dir_all(&args.output_repo)?;
    }

    // The bucket is public => no need to log in.
    let credentials = google_cloud_auth::credentials::anonymous::Builder::new().build();
    let storage = Storage::builder()
        .with_credentials(credentials.clone())
        .build()
        .await?;
    let storage_control = StorageControl::builder()
        .with_credentials(credentials)
        .build()
        .await?;

    let hg_revisions = list_revisions(&storage_control).await?;
    info!("Nb revisions: {}", hg_revisions.len());

    let git_revisions = convert_hg_to_git(&args.firefox_repo, &hg_revisions)?;
    info!("Nb converted revisions: {}", git_revisions.len());

    let datetimes = extract_date_times(&args.firefox_repo, &git_revisions)?;
    info!("Nb extracted dates: {}", datetimes.len());

    let mut revisions: Vec<_> = datetimes
        .into_iter()
        .zip(git_revisions.into_iter().zip(hg_revisions.into_iter()))
        .filter(|(datetime, _)| args.since.is_none_or(|since| *datetime > since))
        .map(|(datetime, (git_revision, hg_revision))| RevisionData {
            datetime,
            git_revision,
            hg_revision,
        })
        .collect();
    revisions.sort_by_key(|revision_data| revision_data.datetime);

    info!("Nb sorted and filtered revisions: {}", revisions.len());

    let (file_lister, file_list) = file_lister(
        open_or_init_bare_repo(&args.output_repo)?,
        storage_control.clone(),
        revisions,
        args.branch,
    );
    let (downloader, compressed) = downloader(storage, file_list);
    let (decompressor, decompressed) = decompressor(compressed);
    let (parser_handle, parsed) = parser(decompressed);
    let git_sender_handle = git_sender(&args.output_repo, parsed)?;

    file_lister.await??;
    downloader.await??;
    decompressor.await??;
    parser_handle.await??;
    git_sender_handle.await??;

    Ok(())
}

#[tracing::instrument(skip(client))]
async fn list_revisions(client: &StorageControl) -> anyhow::Result<Vec<String>> {
    let req = client
        .list_objects()
        .set_parent(BUCKET)
        .set_prefix(PREFIX)
        .set_delimiter('/');
    let mut pages = req.by_page();
    let mut list_revisions = Vec::<String>::new();
    while let Some(page) = pages.next().await {
        let page = page?;
        list_revisions.extend(page.prefixes.into_iter().map(|p| {
            p.strip_prefix(PREFIX)
                .unwrap_or(&p)
                .strip_suffix("/")
                .unwrap_or(&p)
                .to_string()
        }));
    }

    Ok(list_revisions)
}

#[tracing::instrument]
fn sync_firefox_repo(firefox_repo: &Path) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args([
            "-C",
            &firefox_repo.to_string_lossy(),
            "cinnabar",
            "fetch",
            "hg::https://hg.mozilla.org/mozilla-central",
            "default",
        ])
        .status()?;
    if !status.success() {
        bail!(
            "git cinnabar fetch failed for repository {}",
            firefox_repo.display()
        );
    }

    Ok(())
}

#[tracing::instrument(skip(hg_revisions))]
fn convert_hg_to_git(firefox_repo: &Path, hg_revisions: &[String]) -> anyhow::Result<Vec<String>> {
    sync_firefox_repo(firefox_repo)?;

    let mut child = Command::new("git")
        .args([
            "-C",
            &firefox_repo.to_string_lossy(),
            "cinnabar",
            "hg2git",
            "--batch",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    {
        let mut stdin = child.stdin.take().unwrap();
        // Write the HG revisions to the stdin of the child process.
        writeln!(stdin, "{}", hg_revisions.join(" "))?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("hg2git failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = str::from_utf8(&output.stdout)?;
    Ok(stdout.lines().map(|s| s.to_string()).collect())
}

#[tracing::instrument(skip(git_revisions))]
fn extract_date_times(
    firefox_repo: &Path,
    git_revisions: &[String],
) -> anyhow::Result<Vec<time::OffsetDateTime>> {
    let firefox_repo_git = Repository::open(firefox_repo)?;
    let datetimes = git_revisions
        .iter()
        .map(|git_rev| -> anyhow::Result<_> {
            let commit_object = firefox_repo_git.revparse_single(git_rev)?;
            let commit = commit_object.peel_to_commit()?;
            let datetime = commit.time();
            let datetime = time::OffsetDateTime::from_unix_timestamp(datetime.seconds())?
                .replace_offset(UtcOffset::from_whole_seconds(
                    datetime.offset_minutes() * 60,
                )?);
            Ok(datetime)
        })
        .collect::<Result<_, _>>()?;
    Ok(datetimes)
}

#[tracing::instrument(skip(client))]
async fn list_files_for_revision(
    client: &StorageControl,
    prefix: &str,
) -> anyhow::Result<Vec<String>> {
    let req = client.list_objects().set_parent(BUCKET).set_prefix(prefix);
    let mut pages = req.by_page();
    let mut list_files = Vec::<String>::new();
    while let Some(page) = pages.next().await {
        let page = page?;
        list_files.extend(page.objects.into_iter().map(|o| o.name));
    }

    Ok(list_files)
}

#[derive(Debug)]
struct DownloadJob {
    platform: String,
    testsuite: String,
    metadata: RevisionData,
}

fn file_lister(
    repo: Repository,
    client: StorageControl,
    revisions: Vec<RevisionData>,
    only_branch: Option<String>,
) -> (JoinHandle<anyhow::Result<()>>, mpsc::Receiver<DownloadJob>) {
    let (sender, receiver) = mpsc::channel(PIPELINE_SIZE);

    let lister = tokio::spawn(async move {
        for revision in revisions {
            let revision_prefix = format!("mozilla-central/{}", revision.hg_revision);

            let files = list_files_for_revision(&client, &revision_prefix).await?;

            for file in files {
                let file = file
                    .strip_prefix(&revision_prefix)
                    .unwrap_or(&file)
                    .strip_prefix('/')
                    .unwrap_or(&file)
                    .strip_suffix(".json.zstd")
                    .unwrap_or(&file);
                if let Some((platform, testsuite)) = file.split_once(':') {
                    if let Some(ref only_branch) = only_branch
                        && format!("{platform}/{testsuite}") != *only_branch
                    {
                        continue;
                    }

                    let ref_revision = format!(
                        "refs/tags/reverse/{}/{}/{}",
                        platform, testsuite, revision.git_revision
                    );

                    if repo.find_reference(&ref_revision).is_ok() {
                        warn!("Tag {} already exists, skipping...", ref_revision);
                        continue;
                    }

                    sender
                        .send(DownloadJob {
                            platform: platform.to_string(),
                            testsuite: testsuite.to_string(),
                            metadata: revision.clone(),
                        })
                        .await?;
                } else {
                    warn!("Unexpected file format for file: {}", file);
                }
            }
        }
        Ok(())
    });

    (lister, receiver)
}

#[derive(Debug)]
struct DownloadedReport {
    platform: String,
    testsuite: String,
    metadata: RevisionData,
    compressed_bytes: Vec<u8>,
}

#[tracing::instrument(skip(client))]
async fn download(job: &DownloadJob, client: Storage) -> anyhow::Result<DownloadedReport> {
    let metadata = job.metadata.clone();
    let object_name = format!(
        "{PREFIX}{}/{}:{}.json.zstd",
        job.metadata.hg_revision, job.platform, job.testsuite
    );

    let mut resp = client.read_object(BUCKET, object_name).send().await?;
    let mut compressed_bytes = Vec::new();
    while let Some(chunk) = resp.next().await.transpose()? {
        compressed_bytes.extend_from_slice(&chunk);
    }

    Ok(DownloadedReport {
        platform: job.platform.clone(),
        testsuite: job.testsuite.clone(),
        metadata,
        compressed_bytes,
    })
}

fn downloader(
    client: Storage,
    mut jobs: mpsc::Receiver<DownloadJob>,
) -> (
    JoinHandle<anyhow::Result<()>>,
    mpsc::Receiver<DownloadedReport>,
) {
    let (compressed_sender, compressed_receiver) = mpsc::channel(PIPELINE_SIZE);

    let downloader = tokio::spawn(async move {
        while let Some(job) = jobs.recv().await {
            let compressed = download(&job, client.clone()).await?;
            compressed_sender.send(compressed).await?;
        }
        Ok(())
    });

    (downloader, compressed_receiver)
}

#[tracing::instrument(skip_all, fields(revision = ?downloaded.metadata.git_revision, platform = ?downloaded.platform, testsuite = ?downloaded.testsuite))]
fn decompress(downloaded: &DownloadedReport) -> anyhow::Result<Vec<u8>> {
    let decompress_data = zstd::decode_all(downloaded.compressed_bytes.as_slice())?;
    debug!(
        "Decompressed data size for object: {} bytes",
        decompress_data.len()
    );
    Ok(decompress_data)
}

struct DecompressedReport {
    platform: String,
    testsuite: String,
    metadata: RevisionData,
    decompressed_bytes: Vec<u8>,
}

fn decompressor(
    mut compressed: mpsc::Receiver<DownloadedReport>,
) -> (
    JoinHandle<anyhow::Result<()>>,
    mpsc::Receiver<DecompressedReport>,
) {
    let (decompressed_sender, decompressed_receiver) = mpsc::channel(PIPELINE_SIZE);

    let decompressor = task::spawn_blocking(move || {
        while let Some(compressed) = compressed.blocking_recv() {
            let decompressed = decompress(&compressed)?;
            decompressed_sender.blocking_send(DecompressedReport {
                platform: compressed.platform,
                testsuite: compressed.testsuite,
                metadata: compressed.metadata,
                decompressed_bytes: decompressed,
            })?;
        }
        Ok(())
    });

    (decompressor, decompressed_receiver)
}

#[tracing::instrument(skip_all, fields(revision = ?decompressed.metadata.git_revision, platform = ?decompressed.platform, testsuite = ?decompressed.testsuite))]
fn parse(decompressed: &DecompressedReport) -> anyhow::Result<Report> {
    let branch = format!("{}/{}", decompressed.platform, decompressed.testsuite);
    let metadata = ReportMetadata {
        commit: decompressed.metadata.git_revision.to_string(),
        branch,
        date: decompressed.metadata.datetime,
    };

    let report = Report::read(decompressed.decompressed_bytes.as_slice(), metadata)?;
    Ok(report)
}

fn parser(
    mut decompressed: mpsc::Receiver<DecompressedReport>,
) -> (JoinHandle<anyhow::Result<()>>, mpsc::Receiver<Report>) {
    let (parsed_sender, parsed_receiver) = mpsc::channel(PIPELINE_SIZE);

    let parser = task::spawn_blocking(move || {
        while let Some(decompressed) = decompressed.blocking_recv() {
            let parsed = parse(&decompressed)?;
            parsed_sender.blocking_send(parsed)?;
        }
        Ok(())
    });

    (parser, parsed_receiver)
}

#[tracing::instrument(skip_all, fields(report = ?report.metadata))]
fn send_to_git(
    output_repo_git: &Repository,
    fast_import_buffer: &mut impl Write,
    report: Report,
    existing_branches: &mut HashSet<String>,
) -> anyhow::Result<()> {
    let ref_branch = format!("refs/heads/{}", report.metadata.branch);

    // Only send `from {branch}^0` to fast-import once per branch
    let resume_preexisting_branch = if existing_branches.contains(&ref_branch) {
        false
    } else {
        let exists_in_repo = output_repo_git.find_reference(&ref_branch).is_ok();
        existing_branches.insert(ref_branch.clone());
        exists_in_repo
    };

    debug!("Existing branch '{ref_branch}' in output repo: {resume_preexisting_branch}");
    report.write_to_git(fast_import_buffer, resume_preexisting_branch)?;
    Ok(())
}

fn open_or_init_bare_repo(output_repo: &Path) -> anyhow::Result<Repository> {
    std::fs::create_dir_all(output_repo)?;
    let repo = Repository::init_opts(
        output_repo,
        RepositoryInitOptions::new()
            .bare(true)
            .initial_head("all/all"),
    )?;
    Ok(repo)
}

fn git_sender(
    output_repo: &Path,
    mut reports: mpsc::Receiver<Report>,
) -> anyhow::Result<JoinHandle<anyhow::Result<()>>> {
    let output_repo_git = open_or_init_bare_repo(output_repo)?;

    let mut fast_import = Command::new("git")
        .current_dir(output_repo)
        .arg("fast-import")
        .stdin(Stdio::piped())
        .spawn()?;

    let Some(stdin_fastimport) = fast_import.stdin.take() else {
        bail!("failed to open child process stdin");
    };

    let mut fast_import_buffer = BufWriter::new(stdin_fastimport);

    writeln!(&mut fast_import_buffer, "feature done")?;
    writeln!(&mut fast_import_buffer, "feature force")?;
    writeln!(&mut fast_import_buffer, "feature date-format=rfc2822")?;

    let mut existing_branches = HashSet::new();
    let handle = task::spawn_blocking(move || {
        while let Some(report) = reports.blocking_recv() {
            send_to_git(
                &output_repo_git,
                &mut fast_import_buffer,
                report,
                &mut existing_branches,
            )?;
        }
        // Call done => finish fast-import process and wait for it to complete.
        writeln!(&mut fast_import_buffer, "done")?;
        drop(fast_import_buffer);
        fast_import.wait()?;
        Ok(())
    });
    Ok(handle)
}
