fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "keygen")]
    {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use p256::SecretKey;
        use p256::elliptic_curve::Generate;
        use p256::elliptic_curve::sec1::ToSec1Point;
        use p256::pkcs8::{EncodePrivateKey, LineEnding};
        use serde_json::json;

        let private_key = SecretKey::generate_from_rng(&mut rand::rng());
        let private_key_pem = private_key.to_pkcs8_pem(LineEnding::LF)?;

        let public_key = private_key.public_key();
        let encoded_point = public_key.to_sec1_point(false); // Uncompressed point

        let x_bytes = encoded_point.x().ok_or("Missing X coordinate")?;
        let y_bytes = encoded_point.y().ok_or("Missing Y coordinate")?;
        let d_bytes = private_key.to_bytes();

        let x_b64 = URL_SAFE_NO_PAD.encode(x_bytes);
        let y_b64 = URL_SAFE_NO_PAD.encode(y_bytes);
        let d_b64 = URL_SAFE_NO_PAD.encode(d_bytes);

        let jwk = json!({
            "kty": "EC",
            "crv": "P-256",
            "x": x_b64,
            "y": y_b64,
            "d": d_b64
        });

        println!("{}", replace_all_but_last(&private_key_pem, "\n", "\\r\\n"));
        println!("{}", serde_json::to_string_pretty(&jwk)?);
    }
    #[cfg(not(feature = "keygen"))]
    {
        eprintln!("Error: Please run with --features keygen");
    }

    Ok(())
}

#[cfg(feature = "keygen")]
fn replace_all_but_last(input: &str, target: &str, replacement: &str) -> String {
    if let Some(last_idx) = input.rfind(target) {
        let (before, after) = input.split_at(last_idx);

        let new_before = before.replace(target, replacement);
        format!("{}{}", new_before, after)
    } else {
        input.to_string()
    }
}
