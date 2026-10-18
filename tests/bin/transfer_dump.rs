//! Test-only projection; does not change FileFacts or the report schema.
use atlas::{TransferMethod, TransferStatus};
use serde_json::json;
use std::io::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut source = String::new();
    io::stdin().read_to_string(&mut source)?;
    let facts = atlas::analyze(&source)?;
    let boundaries: Vec<_> = facts
        .boundaries
        .iter()
        .map(|boundary| {
            let result = facts.assess_transfer(boundary.id).unwrap();
            json!({
                "method": match result.method {
                    TransferMethod::JsonScript => "JsonScript",
                    TransferMethod::AstroProps => "AstroProps",
                },
                "inputs": boundary.inputs.iter().zip(&result.inputs).map(|(input, result)| {
                    json!({
                        "name": input.name,
                        "status": match result.status {
                            TransferStatus::Supported => "Supported",
                            TransferStatus::Unsupported => "Unsupported",
                            TransferStatus::Unknown => "Unknown",
                        },
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&boundaries)?);
    Ok(())
}
