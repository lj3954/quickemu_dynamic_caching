use anyhow::{bail, Context, Result};
use chrono::{DateTime, Days, Utc};
use clap::Parser;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const PROFILE: &str = "606624d44113";
const ARCH_DL_TYPES: [&str; 3] = ["i686-UNUSED", "x86_64", "aarch64"];

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let client = Client::new();
    let session_id = permit_session(&client).await?;

    let output = match args.find_url(&client, &session_id).await {
        Ok((url, expiration)) => Output {
            status: OutputStatus::Success { url },
            expiration,
        },
        Err(e) => Output {
            status: OutputStatus::Error {
                error: e.to_string(),
            },
            expiration: Utc::now() + Days::new(1),
        },
    };
    let output_serialized = serde_json::to_string(&output)?;
    println!("{output_serialized}");

    Ok(())
}

#[derive(Serialize)]
struct Output {
    #[serde(flatten)]
    status: OutputStatus,
    expiration: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(tag = "status")]
enum OutputStatus {
    Success { url: String },
    Error { error: String },
}

#[derive(Parser)]
struct Args {
    #[clap(long)]
    arch: String,
    #[clap(long)]
    referer: String,
    #[clap(long)]
    sku: String,
    #[clap(long)]
    product_edition_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DownloadOptions {
    #[serde(default)]
    product_download_options: Vec<DownloadOption>,
    #[serde(default)]
    errors: Vec<MsError>,
    #[serde(default)]
    download_expiration_datetime: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MsError {
    key: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DownloadOption {
    uri: String,
    download_type: usize,
}

impl Args {
    async fn find_url(&self, client: &Client, session_id: &str) -> Result<(String, DateTime<Utc>)> {
        let url = format!("https://www.microsoft.com/software-download-connector/api/getskuinformationbyproductedition?profile={PROFILE}&ProductEditionId={}&SKU=undefined&friendlyFileName=undefined&Locale=en-US&sessionID={session_id}", self.product_edition_id);
        client
            .get(&url)
            .send()
            .await
            .context("Failed to send request to get SKU IDs")?;

        let url = format!("https://www.microsoft.com/software-download-connector/api/GetProductDownloadLinksBySku?profile={PROFILE}&productEditionId=undefined&SKU={}&friendlyFileName=undefined&Locale=en-US&sessionID={session_id}", self.sku);

        let response = client
            .get(&url)
            .header(header::REFERER, &self.referer)
            .send()
            .await
            .context("Could not send request to find URL")?;

        let options: DownloadOptions = response
            .json()
            .await
            .context("Failed to deserialize download options")?;

        if !options.errors.is_empty() {
            bail!(options
                .errors
                .iter()
                .map(|MsError { key, value }| format!("{key}: {value}"))
                .collect::<Vec<String>>()
                .join(" "))
        }

        let url = options
            .product_download_options
            .into_iter()
            .find(|option| {
                ARCH_DL_TYPES
                    .get(option.download_type)
                    .is_some_and(|dl_type| dl_type == &self.arch)
            })
            .map(|dl| dl.uri)
            .context("Could not find any valid download option")?;

        Ok((url, options.download_expiration_datetime))
    }
}

async fn permit_session(client: &Client) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let permit_url =
        format!("https://vlscppe.microsoft.com/tags?org_id=y6jn8c31&session_id={session_id}");

    client
        .get(&permit_url)
        .header(header::ACCEPT, "")
        .send()
        .await
        .context("Failed to permit session")?;

    Ok(session_id)
}
