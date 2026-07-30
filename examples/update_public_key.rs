use std::env;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rupora::updater::{decode_signing_key, derive_verifying_key};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let signing_key = env::var("RUPORA_UPDATE_SIGNING_KEY")
        .map_err(|_| "RUPORA_UPDATE_SIGNING_KEY is not configured".to_owned())
        .and_then(|value| decode_signing_key(&value))?;
    println!("{}", BASE64.encode(derive_verifying_key(&signing_key)));
    Ok(())
}
