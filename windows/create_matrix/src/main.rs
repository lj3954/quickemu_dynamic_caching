use std::collections::HashMap;

use anyhow::{Context, Result};
use regex::Regex;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const USER_AGENT: &str = "Mozilla/5.0 (X11, Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0";

const RELEASE_MATRIX: [(&str, &str, &str); 3] = [
    (
        "11",
        "x86_64",
        "https://microsoft.com/en-us/software-download/windows11",
    ),
    (
        "11",
        "aarch64",
        "https://microsoft.com/en-us/software-download/windows11ARM64",
    ),
    (
        "10",
        "x86_64",
        "https://microsoft.com/en-us/software-download/windows10ISO",
    ),
];

const PROFILE: &str = "606624d44113";

const HASH_REGEX: &str = r#"</tr><tr><td>([\w\s()]+) 64-bit</td>\s*<td>([A-F0-9]{64})</td>"#;
const PRODUCT_EDITION_REGEX: &str = r#"option value="(\d+)"#;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();
    let hash_regex = Regex::new(HASH_REGEX).unwrap();
    let product_edition_regex = Regex::new(PRODUCT_EDITION_REGEX).unwrap();
    let session_id = permit_session(&client).await?;

    let search_instances: Vec<_> = RELEASE_MATRIX
        .into_iter()
        .map(|(release, arch, url)| {
            (
                DataSearch {
                    client: client.clone(),
                    url,
                    hash_regex: hash_regex.clone(),
                    product_edition_regex: product_edition_regex.clone(),
                    session_id: session_id.clone(),
                },
                release,
                arch,
            )
        })
        .collect();

    let searches = search_instances
        .iter()
        .map(|(instance, release, arch)| instance.get_matrix_entries(release, arch));

    let matrix: SkuMatrix = futures::future::join_all(searches)
        .await
        .into_iter()
        .inspect(|r| {
            if let Err(e) = r {
                eprintln!("Error: {e}")
            }
        })
        .flatten()
        .flatten()
        .collect();

    println!("{}", serde_json::to_string(&matrix).unwrap());

    Ok(())
}

async fn permit_session(client: &Client) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let permit_url =
        format!("https://vlscppe.microsoft.com/tags?org_id=y6jn8c31&session_id={session_id}");

    client
        .get(&permit_url)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT, "")
        .send()
        .await
        .context("Failed to permit session")?;

    Ok(session_id)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Skus {
    skus: Vec<Sku>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Sku {
    id: String,
    language: String,
}

struct DataSearch {
    client: Client,
    url: &'static str,
    hash_regex: Regex,
    product_edition_regex: Regex,
    session_id: String,
}

type SkuMatrix = Vec<SkuMatrixEntry>;

#[derive(Serialize)]
struct SkuMatrixEntry {
    release: &'static str,
    arch: &'static str,
    referer: &'static str,
    language: String,
    product_edition_id: String,
    sku: String,
    checksum: Option<String>,
}

impl DataSearch {
    async fn get_matrix_entries(
        &self,
        release: &'static str,
        arch: &'static str,
    ) -> Result<impl IntoIterator<Item = SkuMatrixEntry> + use<'_>> {
        let (skus, product_edition_id, mut checksums) = self.find_sku_data().await?;
        Ok(skus.into_iter().map(move |sku| SkuMatrixEntry {
            release,
            arch,
            referer: self.url,
            checksum: checksums.remove(&sku.language),
            language: sku.language,
            sku: sku.id,
            product_edition_id: product_edition_id.clone(),
        }))
    }

    async fn find_sku_data(&self) -> Result<(Vec<Sku>, String, HashMap<String, String>)> {
        let (product_edition_id, checksum_map) = self.find_product_data().await?;

        let url = format!("https://www.microsoft.com/software-download-connector/api/getskuinformationbyproductedition?profile={PROFILE}&ProductEditionId={product_edition_id}&SKU=undefined&friendlyFileName=undefined&Locale=en-US&sessionID={}", self.session_id);
        let body = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send request to get SKU IDs")?;

        let skus: Vec<Sku> = body
            .json::<Skus>()
            .await
            .context("Failed to parse SKU JSON data")?
            .skus;

        Ok((skus, product_edition_id, checksum_map))
    }

    async fn find_product_data(&self) -> Result<(String, HashMap<String, String>)> {
        let response = self
            .client
            .get(self.url)
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::ACCEPT, "")
            .send()
            .await
            .context("Failed to send request to get product edition ID")?;
        let body = response
            .text()
            .await
            .context("Failed to get body from response for product edition ID")?;

        let product_edition_id = self
            .product_edition_regex
            .captures(&body)
            .context("Failed to parse product edition ID from response")?[1]
            .to_string();

        let checksum_map: HashMap<String, String> = self
            .hash_regex
            .captures_iter(&body)
            .map(|c| c.extract())
            .map(|(_, [language, checksum])| (language.to_string(), checksum.to_string()))
            .collect();

        eprintln!("checksums: {:?}", checksum_map);

        Ok((product_edition_id, checksum_map))
    }
}
