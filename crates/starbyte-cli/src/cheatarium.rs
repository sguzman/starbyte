//! Read-only Cheatarium v1 SNES index inspection.
//!
//! The user supplies the local compressed bundle path. There is no network,
//! ROM lookup, cheat decoding, memory write, or automatic code activation.
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use serde_json::{Value, json};

const MAX_UNCOMPRESSED_BYTES: u64 = 128 * 1024 * 1024;

fn decode_snes_bundle(reader: impl Read) -> Result<Value> {
    let mut decoded = Vec::new();
    GzDecoder::new(reader)
        .take(MAX_UNCOMPRESSED_BYTES + 1)
        .read_to_end(&mut decoded)
        .context("failed to decompress Cheatarium SNES bundle")?;
    ensure!(
        (decoded.len() as u64) <= MAX_UNCOMPRESSED_BYTES,
        "Cheatarium bundle is too large"
    );
    let bundle: Value =
        serde_json::from_slice(&decoded).context("invalid Cheatarium bundle JSON")?;
    ensure!(
        bundle["schema_version"] == 1,
        "unsupported Cheatarium schema"
    );
    ensure!(bundle["platform"] == "snes", "Starbyte requires SNES index");
    ensure!(
        bundle["records"].is_array(),
        "Cheatarium bundle has no source records"
    );
    Ok(bundle)
}

/// Search only the explicitly selected local index; never enables codes.
pub fn search(path: &Path, title: &str, limit: usize, as_json: bool) -> Result<()> {
    ensure!(!title.trim().is_empty(), "title cannot be blank");
    ensure!(
        (1..=100).contains(&limit),
        "limit must be between 1 and 100"
    );
    let input =
        File::open(path).with_context(|| format!("failed to open index {}", path.display()))?;
    let bundle = decode_snes_bundle(input)?;
    let records = bundle["records"]
        .as_array()
        .context("Cheatarium records are missing")?;
    let query = title.trim().to_lowercase();
    let matches: Vec<&Value> = records
        .iter()
        .filter(|record| {
            record["title_hint"]
                .as_str()
                .is_some_and(|name| name.to_lowercase().contains(&query))
        })
        .collect();
    let total = matches.len();
    let results: Vec<&Value> = matches.into_iter().take(limit).collect();
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema": "starbyte.cheatarium_candidates.v1",
                "platform": "snes",
                "query": title,
                "candidate_only": true,
                "cheats_activated": false,
                "total_source_records": total,
                "records": results
            }))?
        );
    } else {
        println!("{total} SNES source-record candidates for {title:?}");
        println!("Filename matches are not cartridge/build verification; no cheats are enabled.");
        for record in results {
            let filename = record["raw_filename"].as_str().unwrap_or("<unknown>");
            let source = record["provenance"]["source_id"]
                .as_str()
                .unwrap_or("<unknown>");
            let codes = record["codes"].as_array().map_or(0, |items| {
                items
                    .iter()
                    .filter(|entry| {
                        entry["role"] == "code"
                            && entry["code"].as_str().is_some_and(|v| !v.is_empty())
                    })
                    .count()
            });
            println!("- {filename}: {codes} encoded codes ({source})");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use flate2::{Compression, GzBuilder};
    use std::io::Write;

    use super::decode_snes_bundle;

    fn example(platform: &str) -> Vec<u8> {
        let content = serde_json::json!({
            "schema_version": 1,
            "platform": platform,
            "records": [{
                "title_hint": "Donkey Kong Country",
                "raw_filename": "Donkey Kong Country (USA) (Game Genie).cht",
                "codes": [{"role": "code", "code": "1DCC-CA7A"}],
                "provenance": {"source_id": "libretro-database"}
            }]
        });
        let mut gz = GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), Compression::fast());
        gz.write_all(&serde_json::to_vec(&content).unwrap())
            .unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn reads_read_only_candidate_source_record() {
        let record = decode_snes_bundle(example("snes").as_slice()).unwrap();
        assert_eq!(record["records"][0]["codes"][0]["code"], "1DCC-CA7A");
        assert_eq!(
            record["records"][0]["provenance"]["source_id"],
            "libretro-database"
        );
    }

    #[test]
    fn rejects_another_platform() {
        assert!(decode_snes_bundle(example("ps1").as_slice()).is_err());
    }
}
